/*
 * Portable Tobii TTP-over-USB protocol layer for the STM32F411 gaze reader.
 *
 * No hardware dependency: parses the bytes a USB bulk read returns and pulls
 * out the combined gaze point. Ported from this repo's `crates/tobii-proto`
 * (the source of truth) — see protocol.rs, tlv.rs and gaze83.rs there.
 *
 * TTP numeric fields are big-endian; the USB bulk framing words are
 * little-endian.
 */
#ifndef OPEN_TOBII_PROTO_H
#define OPEN_TOBII_PROTO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* ---- USB identity & endpoints (crates/tobii-usb/src/models.rs) ---- */
#define TOBII_USB_VID 0x2104u
#define TOBII_PID_ET5 0x0313u   /* Eye Tracker 5 */
#define TOBII_PID_SPARK 0x031Cu /* Pro Spark */
#define TOBII_IFACE 0u
#define TOBII_EP_OUT 0x05u /* bulk OUT: commands / init replay */
#define TOBII_EP_IN 0x83u  /* bulk IN: multiplexed stream */

/* ---- Vendor control requests (device.rs) ---- */
/* bmRequestType 0x41 (OUT) / 0xC1 (IN), bRequest below. */
#define TOBII_REQ_SCP 48u           /* SCP metadata (platform/firmware) */
#define TOBII_REQ_CHANNEL_OPEN 65u  /* open the TTP channel; starts EP 0x83 */
#define TOBII_REQ_CHANNEL_CLOSE 66u /* close it (Windows-style stop) */
#define TOBII_REQ_TIMESYNC 69u

/* ---- TTP message header (protocol.rs) ----
 * A bulk IN frame is: u32 LE channel mask, u32 LE total length, then the TTP
 * message. The TTP header fields are u32 BE at these offsets into the frame. */
#define TOBII_OFF_MARKER 8u  /* one of TOBII_MARKER_* */
#define TOBII_OFF_SEQ 12u
#define TOBII_OFF_STATUS 16u
#define TOBII_OFF_ID 20u /* command / stream / notification id */
#define TOBII_OFF_ERROR 24u
#define TOBII_OFF_LEN 28u    /* declared payload length */
#define TOBII_HEADER_LEN 32u /* payload starts here */

#define TOBII_MARKER_COMMAND 0x51u
#define TOBII_MARKER_RESPONSE 0x52u
#define TOBII_MARKER_STREAM 0x53u
#define TOBII_MARKER_NOTIFICATION 0x4Eu

/* ---- Stream ids (protocol.rs) ---- */
#define TOBII_STREAM_GAZE 0x500u
#define TOBII_STREAM_PRESENCE 0x504u

/* ---- Keyed-TLV payload (tlv.rs) ----
 * Payload = 00 00, then entries of { u8 type, u32 BE length, value }.
 * Keyed stream messages: a 0x05 header entry whose value is the field id
 * (n<<16)|0x0BB8 announces n fields; each field is `05 KEY_FIELD_ID`, a
 * `02 key` entry, then the value (scalar / string / `05 <id>` point). */
#define TOBII_TLV_U32_ALT 0x01u
#define TOBII_TLV_U32 0x02u
#define TOBII_TLV_FIXED16 0x03u /* 16.16 signed */
#define TOBII_TLV_FIXED32 0x04u /* 32.32 signed */
#define TOBII_TLV_FIELD_ID 0x05u
#define TOBII_TLV_U64 0x06u
#define TOBII_TLV_STRING 0x14u
#define TOBII_TLV_BYTES 0x15u
#define TOBII_TLV_LIST 0x17u

#define TOBII_KEYED_TAG 0x0BB8u /* low half of the keyed header field id */

/* ---- Gaze field keys (gaze83.rs) ---- */
#define TOBII_KEY_TIMESTAMP 0x01u
#define TOBII_KEY_COMBINED_GAZE_VALID 0x1Bu
#define TOBII_KEY_COMBINED_GAZE 0x1Cu /* 2-D gaze point, display frame ~[0,1] */

/* The combined gaze point of one 0x500 message. */
struct tobii_gaze {
    float x; /* display-frame X, normalised (0=left, 1=right) */
    float y; /* display-frame Y, normalised (0=top, 1=bottom) */
    bool valid;
};

/* True if `frame` (a bulk IN frame of `len` bytes) is a gaze (0x500) stream
 * message. `frame` must be at least TOBII_HEADER_LEN long. */
bool tobii_is_gaze_frame(const uint8_t *frame, size_t len);

/* Read big-endian header fields of a bulk IN frame. */
uint32_t tobii_frame_marker(const uint8_t *frame);
uint32_t tobii_frame_id(const uint8_t *frame);
uint32_t tobii_frame_len(const uint8_t *frame);

/*
 * Decode the combined gaze point from a gaze (0x500) bulk IN frame.
 * Returns true and fills *out on success.
 *
 * NOTE: the keyed-TLV walk follows tlv.rs / gaze83.rs but has not yet been
 * validated against a real capture on hardware — milestone 5. Verify the
 * component type and scaling of key 0x1C before trusting the coordinates.
 */
bool tobii_decode_gaze(const uint8_t *frame, size_t len, struct tobii_gaze *out);

#endif /* OPEN_TOBII_PROTO_H */
