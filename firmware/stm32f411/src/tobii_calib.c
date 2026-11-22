/*
 * Tobii calibration over TTP for the STM32F411 gaze reader.
 * See tobii_calib.h and crates/tobii-proto/src/calibration.rs.
 */
#include "tobii_calib.h"

#include "config.h"
#include "tobii_host.h"
#include "tobii_proto.h"

#define CMD_HEADER_LEN 24u /* 6 BE u32 words */
#define FRAME_BUF 2048

static void put_be32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >> 8);
    p[3] = (uint8_t)v;
}
static void put_le32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}
static void put_be64(uint8_t *p, int64_t v) {
    put_be32(p, (uint32_t)((uint64_t)v >> 32));
    put_be32(p + 4, (uint32_t)(uint64_t)v);
}
static uint32_t get_be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | (uint32_t)p[3];
}
/* The TTP error word (BE) of a bulk IN frame. */
static uint32_t frame_error(const uint8_t *frame) { return get_be32(frame + TOBII_OFF_ERROR); }

size_t tobii_build_command(uint8_t seq, uint32_t id, const uint8_t *payload, size_t plen,
                           uint8_t *out, size_t cap) {
    size_t body = CMD_HEADER_LEN + plen;      /* the TTP message */
    size_t total = 8 + body;                  /* + channel + body_len prefix */
    if (total > cap) return 0;
    put_le32(out + 0, 0);                      /* channel 0 */
    put_le32(out + 4, (uint32_t)body);
    put_be32(out + 8, TOBII_MARKER_COMMAND);   /* 0x51 */
    put_be32(out + 12, seq);
    put_be32(out + 16, 0);                     /* status */
    put_be32(out + 20, id);
    put_be32(out + 24, 0);                     /* error */
    put_be32(out + 28, (uint32_t)plen);
    for (size_t i = 0; i < plen; i++) out[32 + i] = payload[i];
    return total;
}

/* round(v * 2^32) as signed 32.32 (calibration.rs::fixed_32_32). */
static int64_t fixed_32_32(float v) {
    double d = (double)v * 4294967296.0;
    return (int64_t)(d < 0 ? d - 0.5 : d + 0.5);
}

size_t tobii_collect_payload(float x, float y, uint32_t eye_mask, uint8_t *out, size_t cap) {
    /* 00 00, then TLV entries { u8 type, u32 BE len, value }. */
    size_t need = 2 + (5 + 8) + (5 + 8) + (5 + 4);
    if (need > cap) return 0;
    size_t o = 0;
    out[o++] = 0x00;
    out[o++] = 0x00;
    out[o++] = TOBII_TLV_FIXED32;
    put_be32(out + o, 8); o += 4;
    put_be64(out + o, fixed_32_32(x)); o += 8;
    out[o++] = TOBII_TLV_FIXED32;
    put_be32(out + o, 8); o += 4;
    put_be64(out + o, fixed_32_32(y)); o += 8;
    out[o++] = TOBII_TLV_U32;
    put_be32(out + o, 4); o += 4;
    put_be32(out + o, eye_mask); o += 4;
    return o;
}

/* Send one command and wait for its 0x52 response; returns the TTP error
 * (0 = ok, 0xFFFFFFFF = no response). */
static uint32_t cal_command(uint8_t seq, uint32_t id, const uint8_t *payload, size_t plen) {
    uint8_t buf[FRAME_BUF];
    size_t n = tobii_build_command(seq, id, payload, plen, buf, sizeof buf);
    if (n == 0 || tobii_host_bulk_out(buf, n) < 0) return 0xFFFFFFFFu;

    for (int tries = 0; tries < 200; tries++) {
        int r = tobii_host_bulk_in(buf, sizeof buf, 1000);
        if (r <= 0) continue;
        if (r >= (int)TOBII_HEADER_LEN && tobii_frame_marker(buf) == TOBII_MARKER_RESPONSE) {
            /* a response for our command (seq match is in the header too) */
            return frame_error(buf);
        }
        /* otherwise a stream/notification frame: keep reading */
    }
    return 0xFFFFFFFFu;
}

bool tobii_calib_run(const struct tobii_cal_point *points, size_t n, uint32_t fixation_ms) {
    uint8_t seq = 1;
    uint8_t pl[128];

    tobii_log("calibration: start\n");
    if (cal_command(seq++, TOBII_CAL_START, 0, 0) != 0) return false;
    if (cal_command(seq++, TOBII_CAL_CLEAR, 0, 0) != 0) return false;

    for (size_t i = 0; i < n; i++) {
#if OPEN_TOBII_ENABLE_DISPLAY
        tobii_display_dot(points[i].x, points[i].y, true); /* show the target */
#endif
        tobii_host_delay_ms(fixation_ms); /* let the user fixate it */
        size_t plen = tobii_collect_payload(points[i].x, points[i].y, TOBII_EYES_BOTH, pl, sizeof pl);
        uint32_t e = cal_command(seq++, TOBII_CAL_COLLECT_2D, pl, plen);
        if (e != 0) {
            tobii_log("calibration: collect failed\n");
            cal_command(seq++, TOBII_CAL_STOP, 0, 0);
            return false;
        }
    }

    if (cal_command(seq++, TOBII_CAL_COMPUTE, 0, 0) != 0) {
        cal_command(seq++, TOBII_CAL_STOP, 0, 0);
        return false;
    }
    /* TODO: READ (1100) the computed blob and WRITE (1110) it back if the
     * device does not apply on COMPUTE — validate against tobiid. */
    cal_command(seq++, TOBII_CAL_STOP, 0, 0);
    tobii_log("calibration: done\n");
    return true;
}
