// Ported from tobii-linux's `tobii-proto` crate (MIT, Copyright (c) 2026
// Andrii Klaptsov; https://github.com/Krazubu/tobii-linux). Copied
// verbatim except: cross-module paths `crate::` -> `super::`, and the
// `#[cfg(test)]` module (which needs the crate's fixtures) removed.

//! The calibration commands, as captured from the Windows engine calibrating
//! seven points (calibration.pcapng):
//!
//! ```text
//! 1100 read      -> the active calibration blob (a chunked response)
//! 1010 start     -> notification 3220 with id 0
//! 1060 clear
//! 1110 write     -> the stored blob back (seeds the session's sample ring)
//! 1030 collect   x, y, eye mask 3   (blocks ~0.6-0.9 s)
//! 1070 compute   -> notification 3220 with the new id (~2 s)
//! 1100 read      -> the new blob
//! 1020 stop
//! ```
//!
//! Discarding a 2-D point is 1080 with the same payload as a collect. It
//! comes from the DLL (its builder at 0x1801805e0) and was never captured.
//!
//! Commands without parameters (start, stop, clear, compute, read) carry no
//! payload at all, not even the `00 00` a TLV payload starts with.
//!
//! The blob itself is opaque apart from its header and trailing point list;
//! see the `tobii-calib` crate.

use super::tlv::{TYPE_BYTES, TlvWriter, payload_tlvs};

/// Calibration command ids.
pub mod cmd {
    /// Begin a session.
    pub const START: u32 = 1010;
    /// End the session.
    pub const STOP: u32 = 1020;
    /// Collect one 2-D point.
    pub const COLLECT_2D: u32 = 1030;
    /// Clear the collected points.
    pub const CLEAR: u32 = 1060;
    /// Compute and apply.
    pub const COMPUTE: u32 = 1070;
    /// Discard the data collected at one 2-D point. From the DLL (builder
    /// 0x1801805e0); never captured.
    pub const DISCARD_2D: u32 = 1080;
    /// Read the active calibration.
    pub const READ: u32 = 1100;
    /// Write a calibration.
    pub const WRITE: u32 = 1110;
}

/// The eye mask every captured collect carries: both eyes.
pub const EYES_BOTH: u32 = 3;

/// A normalised coordinate as the device wants it: `x * 1024` computed in
/// `f32` (the fractional bits in the capture show f32 rounding), then widened
/// to 32.32 fixed point.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // reason: |x * 1024 * 2^32| < 2^63 for x in 0..=1
pub fn fixed_32_32(v: f32) -> i64 {
    (f64::from(v * 1024.0) * 4_294_967_296.0).round() as i64
}

/// Payload of command 1030 (collect a 2-D point).
#[must_use]
pub fn collect_payload(x: f32, y: f32, eye_mask: u32) -> Vec<u8> {
    point_payload(x, y, eye_mask)
}

/// Payload of command 1080 (discard a 2-D point): the same as a collect's,
/// so the device sees the very coordinate it collected.
#[must_use]
pub fn discard_payload(x: f32, y: f32, eye_mask: u32) -> Vec<u8> {
    point_payload(x, y, eye_mask)
}

/// A 2-D point and an eye mask: `x`, `y` as 32.32 fixed point, then the mask.
fn point_payload(x: f32, y: f32, eye_mask: u32) -> Vec<u8> {
    TlvWriter::new()
        .fixed32_raw(fixed_32_32(x))
        .fixed32_raw(fixed_32_32(y))
        .u32(eye_mask)
        .finish()
}

/// Payload of command 1110 (write a calibration): the blob as one byte
/// string.
#[must_use]
pub fn write_payload(blob: &[u8]) -> Vec<u8> {
    TlvWriter::new().bytes(blob).finish()
}

/// The blob inside a 1110 command payload or a 1100 response payload.
#[must_use]
pub fn blob_from_payload(payload: &[u8]) -> Option<&[u8]> {
    let entry = payload_tlvs(payload).next()?;
    (entry.typ == TYPE_BYTES).then(|| entry.bytes())?
}

