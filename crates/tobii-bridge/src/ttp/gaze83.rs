// Ported from tobii-linux's `tobii-proto` crate (MIT, Copyright (c) 2026
// Andrii Klaptsov; https://github.com/Krazubu/tobii-linux). Copied
// verbatim except: cross-module paths `crate::` -> `super::`, and the
// `#[cfg(test)]` module (which needs the crate's fixtures) removed.

//! The 0x500 gaze stream and the 0x504 presence stream, decoded by field key.
//!
//! Each 0x500 message carries 39 keyed fields (see [`super::tlv`]). Which key
//! is what was established from labelled captures, and the parts the Stream
//! Engine forwards were matched bit-for-bit against its own output: its
//! `gazePoint` is key `0x1c` / 1024 with validity `0x1b`, its `gazeOrigin` is
//! keys `0x22`/`0x24` / 1024 (display frame) with validity `0x21`/`0x23`, and
//! its timestamps are key `0x01` rebased. Like the Stream Engine, values are
//! passed through even when their validity flag is clear; the flag is
//! reported next to them. Names in quotes are the tracker's own, from its
//! column list for the stream (command 1210), less the eye's "Left" or
//! "Right". The list leaves `0x11` and the frame counter `0x14` unnamed and
//! does not list `0x0e`.
//!
//! The other per-eye 3-D positions, in mm, are in the tracker frame: the
//! cornea centres `0x02`/`0x08`, the eyeball centres `0x17`/`0x18` and the
//! 3-D gaze points `0x04`/`0x0a`, which on the display area of the capture
//! lie on the screen plane, where the 2-D gaze points `0x05`/`0x0b` put them.
//! Those 2-D points are normalised, not in mm, and so are two more per-eye
//! positions, each a point / 1024:
//!
//! - "Eye 3D Relative Position", `0x03`/`0x09` ([`EyeFrame::track_box`]),
//!   is the cornea centre p (`0x02`/`0x08`, mm) in the camera image and in
//!   range: `x = 0.499776 − 1.331633 · p.x / p.z`,
//!   `y = 0.499776 − 1.331633 · p.y / p.z` and `z = (|p| − 400) / 500`, to
//!   within 1e-6 on every recorded frame, so z is 0 at 400 mm and 1 at
//!   900 mm. Despite the field's name, it is not normalised to the track box
//!   the tracker reports (command 1400), which covers x 0.13..0.87,
//!   y 0.20..0.80 and z 0.10..1.11 of it: z is 0.10 and 1.0 at the centres
//!   of the box's front and back faces, and 1.11 at its back corners, 955 mm
//!   from the camera.
//! - "User 3D Position Guide", `0x25`/`0x27` with validity `0x26`/`0x28`
//!   ([`EyeFrame::position_guide`]), is the same image normalisation of the
//!   eye from a separate, noisier per-frame estimate, with a depth of its
//!   own.
//!
//! The two combined gaze points are display-normalised too, each a point
//! / 1024. The Stream Engine's `gazePoint`, the tracker's "Relative
//! Combined Gaze Point", is filtered; "Remote Gaze Point Responsive
//! Combined", `0x20` with validity `0x1f` ([`GazeFrame::gaze_raw`]), is
//! not: on every recorded frame it is the mean of the eyes' 2-D gaze points
//! (to within f32 rounding) while both eyes are tracked, the tracked eye's
//! while one is, and valid while either is; with neither tracked it is
//! (-1024, -1024), the sentinel, not valid. "eye 2D position relative to
//! sensor area", `0x19`/`0x1a`, is that sentinel on every recorded frame,
//! and is not decoded.
//!
//! "Absolute Eye Openness", `0x29`/`0x2b` (16.16) with validity
//! `0x2a`/`0x2c`, is always -1.0 with validity 0 on the ET5, and is not
//! decoded.
//!
//! The pupil diameters, in mm, are keys `0x06`/`0x0c` (16.16): the Stream
//! Engine hands them out as gaze data's `pupil_diameter_mm`, valid while the
//! eye's status (`0x07`/`0x0d`) is below 2 (0x18017199a..0x1801719af). They
//! do not shrink with the eye's distance, as a size in camera pixels would.
//!
//! The Stream Engine's own record of a frame, which `process_gaze`
//! (0x18018d9a0) fills and the raw gaze callback receives, takes keys
//! `0x01`..`0x0d`, `0x1b` and `0x1c` and, each behind a flag set when the
//! frame has it, keys `0x0e`, `0x11` and `0x14`..`0x18`; it reads no key
//! above `0x1c`. A [`GazeFrame`] holds all of it: the keys its other fields
//! reduce to a validity, or leave out, are kept as sent in
//! [`EyeFrame::status`] and [`GazeFrame::record`]. Neither the responsive
//! combined gaze point, [`GazeFrame::gaze_raw`] (key `0x20`), nor the
//! position guide is part of that record.

use super::protocol::{Message, STREAM_ID_GAZE, STREAM_ID_PRESENCE};
use super::tlv::{UNITS_PER_MM, keyed_fields};

/// Field keys of the 0x500 message.
pub mod key {
    /// `u64` device timestamp, µs.
    pub const TIMESTAMP: u32 = 0x01;
    /// `u32` frame counter (132 Hz, steps by 4 per output frame).
    pub const FRAME_COUNTER: u32 = 0x14;
    /// Validity of [`COMBINED_GAZE`].
    pub const COMBINED_GAZE_VALID: u32 = 0x1b;
    /// Filtered combined gaze point, display-normalised x1024.
    pub const COMBINED_GAZE: u32 = 0x1c;
    /// "Remote Gaze Point Responsive Combined Validity", of
    /// [`RAW_COMBINED_GAZE`].
    pub const RAW_COMBINED_GAZE_VALID: u32 = 0x1f;
    /// "Remote Gaze Point Responsive Combined": the frame's combined gaze
    /// point, unfiltered, `(left + right) / 2` of the eyes' 2-D gaze points
    /// or the tracked eye's alone, display-normalised x1024.
    pub const RAW_COMBINED_GAZE: u32 = 0x20;
    /// `u32` of unknown meaning, which the ET5 never sends; the Stream
    /// Engine's gaze record keeps it.
    pub const KEY_0E: u32 = 0x0e;
    /// `u32` of unknown meaning; the ET5 sends 4 in every frame.
    pub const KEY_11: u32 = 0x11;

    /// The keys of one eye; names in quotes are the tracker's own (see
    /// [`super::gaze83`]).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Eye {
        /// "Eye Validity", the tracking status: 0 tracked, 4 not tracked.
        pub status: u32,
        /// Pupil diameter, mm, 16.16.
        pub pupil: u32,
        /// Cornea centre (gaze origin), tracker frame.
        pub origin_tracker: u32,
        /// Validity of `origin_tracker`.
        pub origin_tracker_valid: u32,
        /// Gaze origin, display frame.
        pub origin_display: u32,
        /// Validity of `origin_display`.
        pub origin_display_valid: u32,
        /// "Eye 3D Relative Position": the cornea centre in the camera image
        /// and in range, normalised x1024.
        pub track_box: u32,
        /// 3-D gaze point on the screen plane, tracker frame.
        pub gaze_point_3d: u32,
        /// 2-D gaze point, display-normalised x1024.
        pub gaze_point_2d: u32,
        /// "Eye 3D Relative Position Validity", taken as the validity of both
        /// gaze points: on every recorded frame it is 1 exactly when the
        /// eye's status is 0.
        pub gaze_point_valid: u32,
        /// Eyeball rotation centre, tracker frame.
        pub eyeball: u32,
        /// "User 3D Position Guide", normalised x1024.
        pub position_guide: u32,
        /// "User 3D Position Guide Validity", of `position_guide`.
        pub position_guide_valid: u32,
    }

    /// The user's left eye.
    pub const LEFT: Eye = Eye {
        status: 0x07,
        pupil: 0x06,
        origin_tracker: 0x02,
        origin_tracker_valid: 0x16,
        origin_display: 0x22,
        origin_display_valid: 0x21,
        track_box: 0x03,
        gaze_point_3d: 0x04,
        gaze_point_2d: 0x05,
        gaze_point_valid: 0x1d,
        eyeball: 0x17,
        position_guide: 0x25,
        position_guide_valid: 0x26,
    };

    /// The user's right eye.
    pub const RIGHT: Eye = Eye {
        status: 0x0d,
        pupil: 0x0c,
        origin_tracker: 0x08,
        origin_tracker_valid: 0x15,
        origin_display: 0x24,
        origin_display_valid: 0x23,
        track_box: 0x09,
        gaze_point_3d: 0x0a,
        gaze_point_2d: 0x0b,
        gaze_point_valid: 0x1e,
        eyeball: 0x18,
        position_guide: 0x27,
        position_guide_valid: 0x28,
    };
}

/// A value and whether the device vouches for it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Valued<T> {
    /// Whether `value` is a measurement for this frame.
    pub valid: bool,
    /// The value, passed through even when invalid.
    pub value: T,
}

/// One eye of a [`GazeFrame`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EyeFrame {
    /// The device tracks this eye in this frame: its status is 0.
    pub tracked: bool,
    /// Tracking status as sent: the ET5 sends 0 for a tracked eye, 4 for a
    /// lost one. `None` when the frame lacks the key, which the Stream
    /// Engine's zeroed record would read as 0.
    pub status: Option<u32>,
    /// Cornea centre (gaze origin), tracker frame, mm.
    pub origin_tracker_mm: Valued<[f64; 3]>,
    /// Gaze origin, display frame, mm (the Stream Engine's `gazeOrigin`).
    pub origin_display_mm: Valued<[f64; 3]>,
    /// "Eye 3D Relative Position": the cornea centre's place in the camera
    /// image and its range, normalised to about `0..1` per axis (see
    /// [`super::gaze83`]). Valid while the eye is tracked and no component
    /// is a sentinel, which on every recorded frame is when the tracker's
    /// own flag for it (`0x1d`/`0x1e`) is 1.
    pub track_box: Valued<[f64; 3]>,
    /// Gaze point on the screen plane, tracker frame, mm.
    pub gaze_point_tracker_mm: Valued<[f64; 3]>,
    /// Gaze point, normalised display coordinates.
    pub gaze_point_norm: Valued<[f64; 2]>,
    /// Eyeball rotation centre, tracker frame, mm.
    pub eyeball_center_mm: Valued<[f64; 3]>,
    /// Pupil diameter, mm; valid, as the Stream Engine judges it, while the
    /// eye's status is below 2. Unlike it, not valid when the frame lacks
    /// either key (see [`decode_gaze_frame`]).
    pub pupil_diameter_mm: Valued<f64>,
    /// "User 3D Position Guide", keys `0x25`/`0x27` / 1024. x and y place the
    /// eye in the camera image as [`EyeFrame::track_box`] does, from a
    /// separate, noisier per-frame estimate. z rises with the eye's distance
    /// from the tracker: about 0.5 at 700 mm on the recorded fore-aft
    /// captures, which span about 500 to 800 mm (z up to 0.70 there; no
    /// valid z above 0.82 in any capture); a straight line fitted to them
    /// puts 0 near 480 mm and 1 near 920 mm. It fits `(R − 400) / 500` only
    /// approximately (within about 0.03 on most captures), for a range R in
    /// mm worked out from the eyes' separation in the image and an assumed
    /// distance between the eyes: a positioning value, not a distance. Valid
    /// while its own flag (`0x26`/`0x28`) is 1, whatever the eye's status;
    /// with nobody there the tracker sends (0, 0, 0) with the flag clear.
    /// Passed through when invalid.
    pub position_guide: Valued<[f64; 3]>,
}

/// One decoded 0x500 message.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GazeFrame {
    /// Device timestamp, µs.
    pub device_ts_us: u64,
    /// Device frame counter.
    pub frame_counter: u32,
    /// Filtered combined gaze point, normalised display coordinates,
    /// unclamped (the Stream Engine's `gazePoint`).
    pub gaze: Valued<[f64; 2]>,
    /// "Remote Gaze Point Responsive Combined" (key `0x20`): the frame's
    /// combined gaze point, unfiltered, normalised display coordinates.
    /// Despite the field's name, not part of the Stream Engine's gaze
    /// record, which is what its raw gaze hands out.
    pub gaze_raw: Valued<[f64; 2]>,
    /// The user's left eye.
    pub left: EyeFrame,
    /// The user's right eye.
    pub right: EyeFrame,
    /// The rest of what the Stream Engine's gaze record keeps of the frame.
    pub record: RecordKeys,
}

/// Keys of a 0x500 message that the Stream Engine's gaze record keeps as
/// sent and the rest of a [`GazeFrame`] does not: the combined gaze
/// validity, and the keys the record flags as present or not. `None`, or
/// `false`, for a key the frame lacks.
///
/// Where the record puts each (`process_gaze`, 0x18018d9a0, whose key
/// switch jumps through the table at RVA 0x18f5a8; the flagged keys' cases
/// span 0x18018dcf5..0x18018de01): the combined gaze point (key `0x1c`,
/// [`GazeFrame::gaze`]) at `+0x70`, its validity at `+0x78`, then flag and
/// value of `0x0e` at `+0x7c`, `0x11` at `+0x94`, `0x14` at `+0xac`, `0x16`
/// at `+0xb4`, `0x15` at `+0xbc`, and the eyeball centres `0x17` at `+0xc4`
/// and `0x18` at `+0xd4`, whose values are [`EyeFrame::eyeball_center_mm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecordKeys {
    /// Key `0x1b`, the combined gaze validity; [`GazeFrame::gaze`] is valid
    /// when it is 1. The record keeps it without a flag, 0 when absent.
    pub combined_gaze_validity: Option<u32>,
    /// Key `0x0e`, of unknown meaning; the ET5 never sends it.
    pub key_0e: Option<u32>,
    /// Key `0x11`, of unknown meaning; the ET5 sends 4.
    pub key_11: Option<u32>,
    /// Key `0x14`, the frame counter; [`GazeFrame::frame_counter`] is it,
    /// or 0.
    pub frame_counter: Option<u32>,
    /// Key `0x16`, the left eye's gaze-origin validity;
    /// [`EyeFrame::origin_tracker_mm`] is valid when it is 1.
    pub left_origin_flag: Option<u32>,
    /// Key `0x15`, the right eye's gaze-origin validity.
    pub right_origin_flag: Option<u32>,
    /// The frame has key `0x17`, the left eyeball centre, as a 3-D point.
    pub left_eyeball_sent: bool,
    /// The frame has key `0x18`, the right eyeball centre, as a 3-D point.
    pub right_eyeball_sent: bool,
}

/// Wire value of an invalid component: exactly 0 or ±1024 (±1 mm-unit scale
/// or ±1 normalised, depending on the field).
fn is_sentinel(v: f64) -> bool {
    v == 0.0 || v.abs() == UNITS_PER_MM
}

/// Decode a 0x500 message; `None` for any other message or one without a
/// device timestamp.
#[must_use]
pub fn decode_gaze_frame(msg: &Message<'_>) -> Option<GazeFrame> {
    if msg.id != STREAM_ID_GAZE {
        return None;
    }
    let fields = keyed_fields(msg.payload.get(2..)?);
    let device_ts_us = fields.scalar(key::TIMESTAMP)?;
    let word = |k: u32| fields.scalar(k).and_then(|v| u32::try_from(v).ok());
    let flag = |k: u32| word(k) == Some(1);
    let scaled = |v: [f64; 3]| v.map(|c| c / UNITS_PER_MM);

    let eye = |k: key::Eye| {
        let status = word(k.status);
        let tracked = status == Some(0);
        let point3 = |key: u32| fields.point::<3>(key);
        let flagged3 = |key: u32, valid: u32| {
            point3(key).map_or_else(Valued::default, |v| Valued {
                valid: flag(valid),
                value: scaled(v),
            })
        };
        // Fields without a flag of their own: trusted when the eye is
        // tracked and no component is a sentinel.
        let unflagged3 = |key: u32| {
            point3(key).map_or_else(Valued::default, |v| Valued {
                valid: tracked && !v.iter().copied().any(is_sentinel),
                value: scaled(v),
            })
        };
        EyeFrame {
            tracked,
            status,
            origin_tracker_mm: flagged3(k.origin_tracker, k.origin_tracker_valid),
            origin_display_mm: flagged3(k.origin_display, k.origin_display_valid),
            track_box: unflagged3(k.track_box),
            gaze_point_tracker_mm: flagged3(k.gaze_point_3d, k.gaze_point_valid),
            gaze_point_norm: fields
                .point::<2>(k.gaze_point_2d)
                .map_or_else(Valued::default, |v| Valued {
                    valid: flag(k.gaze_point_valid),
                    value: v.map(|c| c / UNITS_PER_MM),
                }),
            eyeball_center_mm: unflagged3(k.eyeball),
            // The Stream Engine's test (0x180171818, 0x180171844). A frame
            // without the status or the diameter, which the ET5 never sends,
            // gives none. The Stream Engine, reading its zeroed record
            // (0x18018dadd), would take a missing status as 0 and a missing
            // diameter as a valid 0.0 mm; libtobii reports neither as valid.
            pupil_diameter_mm: fields
                .fixed16(k.pupil)
                .map_or_else(Valued::default, |v| Valued {
                    valid: status.is_some_and(|s| s < 2),
                    value: v,
                }),
            position_guide: flagged3(k.position_guide, k.position_guide_valid),
        }
    };

    let gaze2 = |point: u32, valid: u32| {
        fields
            .point::<2>(point)
            .map_or_else(Valued::default, |v| Valued {
                valid: flag(valid),
                value: v.map(|c| c / UNITS_PER_MM),
            })
    };

    let record = RecordKeys {
        combined_gaze_validity: word(key::COMBINED_GAZE_VALID),
        key_0e: word(key::KEY_0E),
        key_11: word(key::KEY_11),
        frame_counter: word(key::FRAME_COUNTER),
        left_origin_flag: word(key::LEFT.origin_tracker_valid),
        right_origin_flag: word(key::RIGHT.origin_tracker_valid),
        left_eyeball_sent: fields.point::<3>(key::LEFT.eyeball).is_some(),
        right_eyeball_sent: fields.point::<3>(key::RIGHT.eyeball).is_some(),
    };

    Some(GazeFrame {
        device_ts_us,
        frame_counter: record.frame_counter.unwrap_or(0),
        gaze: gaze2(key::COMBINED_GAZE, key::COMBINED_GAZE_VALID),
        gaze_raw: gaze2(key::RAW_COMBINED_GAZE, key::RAW_COMBINED_GAZE_VALID),
        left: eye(key::LEFT),
        right: eye(key::RIGHT),
        record,
    })
}

/// Presence as the 0x504 stream reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceFrame {
    /// Device timestamp, µs.
    pub device_ts_us: u64,
    /// 1 away, 2 present (the Stream Engine's numbering).
    pub state: u32,
}

/// Decode a 0x504 message (key 1 = timestamp, key 2 = state). The device
/// sends one when the stream starts and then only on change.
#[must_use]
pub fn decode_presence_frame(msg: &Message<'_>) -> Option<PresenceFrame> {
    if msg.id != STREAM_ID_PRESENCE {
        return None;
    }
    let fields = keyed_fields(msg.payload.get(2..)?);
    Some(PresenceFrame {
        device_ts_us: fields.scalar(1)?,
        state: u32::try_from(fields.scalar(2)?).ok()?,
    })
}

