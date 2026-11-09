/*
 * Portable Tobii protocol decode for the STM32F411 gaze reader.
 * Ported from crates/tobii-proto (protocol.rs, tlv.rs, gaze83.rs).
 * No hardware dependency — unit-testable on the host.
 */
#include "tobii_proto.h"

static uint32_t be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | (uint32_t)p[3];
}

static int64_t be64_signed(const uint8_t *p) {
    uint64_t hi = be32(p);
    uint64_t lo = be32(p + 4);
    return (int64_t)((hi << 32) | lo);
}

uint32_t tobii_frame_marker(const uint8_t *f) { return be32(f + TOBII_OFF_MARKER); }
uint32_t tobii_frame_id(const uint8_t *f) { return be32(f + TOBII_OFF_ID); }
uint32_t tobii_frame_len(const uint8_t *f) { return be32(f + TOBII_OFF_LEN); }

bool tobii_is_gaze_frame(const uint8_t *f, size_t len) {
    if (len < TOBII_HEADER_LEN) return false;
    return tobii_frame_marker(f) == TOBII_MARKER_STREAM && tobii_frame_id(f) == TOBII_STREAM_GAZE;
}

/*
 * A 32.32 component of a 2-D gaze point, as a display-normalised coordinate.
 * gaze83.rs: these points are "display-normalised, each a point / 1024", and
 * a 32.32 value is raw / 2^32. So normalised = raw / 2^32 / 1024.
 */
static float fixed32_gaze_norm(const uint8_t *v) {
    int64_t raw = be64_signed(v);
    /* raw / 2^32 / 1024, in float (display precision is ample, and the MCU
     * build forbids double). */
    return (float)raw / (4294967296.0f * 1024.0f);
}

/*
 * Pull the combined gaze point (key 0x1C, validity 0x1B) out of a 0x500 frame.
 *
 * The payload is `00 00` then keyed-TLV entries { u8 type, u32 BE len, value }
 * (tlv.rs). A field is `05 KEY_FIELD_ID`, `02 key`, then its value — a scalar,
 * a string, or a `05 <id>` point whose 0x04 (32.32) entries are the axes.
 *
 * This walk is a best-effort heuristic: it tracks the last `02` key and reads
 * the 0x04 components that follow key 0x1C as (x, y). It has NOT been validated
 * against a real capture — port tlv.rs::keyed_fields exactly and check against
 * hardware at milestone 5 before trusting the coordinates.
 */
bool tobii_decode_gaze(const uint8_t *frame, size_t len, struct tobii_gaze *out) {
    out->x = 0.0f;
    out->y = 0.0f;
    out->valid = false;
    if (!tobii_is_gaze_frame(frame, len)) return false;

    const uint8_t *p = frame + TOBII_HEADER_LEN;
    size_t avail = len - TOBII_HEADER_LEN;
    uint32_t declared = tobii_frame_len(frame);
    if (declared != 0 && declared < avail) avail = declared;
    if (avail < 2) return false;
    const uint8_t *end = p + avail;
    p += 2; /* skip the 00 00 lead-in */

    uint32_t cur_key = 0xFFFFFFFFu;
    int axes = 0; /* components read for key 0x1C */
    bool gaze_valid_flag = false;

    while (p + 5 <= end) {
        uint8_t type = p[0];
        uint32_t elen = be32(p + 1);
        const uint8_t *val = p + 5;
        if (val + elen > end) break;

        if (type == TOBII_TLV_U32 && elen == 4) {
            uint32_t u = be32(val);
            if (cur_key == TOBII_KEY_COMBINED_GAZE_VALID) {
                gaze_valid_flag = (u != 0);
                cur_key = 0xFFFFFFFFu;
            } else {
                cur_key = u; /* a `02` key for the field that follows */
            }
        } else if (type == TOBII_TLV_FIXED32 && elen == 8 && cur_key == TOBII_KEY_COMBINED_GAZE) {
            if (axes == 0) {
                out->x = fixed32_gaze_norm(val);
                axes = 1;
            } else if (axes == 1) {
                out->y = fixed32_gaze_norm(val);
                axes = 2;
                cur_key = 0xFFFFFFFFu;
            }
        }
        p = val + elen;
    }

    out->valid = (axes == 2) && gaze_valid_flag;
    return axes == 2;
}
