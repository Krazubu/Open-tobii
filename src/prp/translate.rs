//! Translating PRP requests to TTP and the device's answers back to PRP.
//!
//! This module holds the pure encoders — the PRP responses the server emits —
//! each checked byte-for-byte against the capture. Wiring them to live TTP
//! values and to the pipe channels is done by the server loop.

use super::wire::{op, Message, Struct};

/// The connect response is a nested capabilities document. Rather than rebuild
/// that format, the bridge replays the ones the device's own runtime produced
/// (captured from this ET5): its stream and property lists are exactly the
/// device's. PETM makes two kinds of connect — a light discovery connect (no
/// licence) and, once it shows the device, a full one (with its licence) — and
/// the runtime answers each differently, so the bridge replays the matching
/// one. Only the transaction id is swapped.
pub fn connect_response(transaction: u32, licence_count: u32) -> Message {
    // The runtime's response carries one validation result per licence the
    // request sent, so it grows by 4 bytes each: PETM's discovery connect
    // sends none, its full connect sends its own licence(s). The bridge
    // replays the real runtime's response captured for that count — the real
    // verdict for these licences, not a reconstructed or faked one.
    const DISCOVERY: &str = include_str!("../../assets/prp_connect_response_discovery.hex"); // 0
    const ONE: &str = include_str!("../../assets/prp_connect_response_1lic.hex"); // 1
    const TWO: &str = include_str!("../../assets/prp_connect_response.hex"); // 2
    let hex = match licence_count {
        0 => DISCOVERY,
        1 => ONE,
        _ => TWO,
    }
    .trim();
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    let (mut msg, _) = super::wire::parse_frame(&bytes).expect("built-in connect template parses");
    msg.transaction = transaction;
    msg
}

/// Response to command 20 (licence key retrieve): the licence the device
/// holds in its own storage, which the runtime reads from the tracker and
/// PETM reads during discovery. The bridge replays the one captured from this
/// device — its real stored licence, not a fabricated verdict. Only the
/// transaction id is swapped.
pub fn license_retrieve_response(transaction: u32) -> Message {
    const TEMPLATE: &str = include_str!("../../assets/prp_license_retrieve.hex");
    let hex = TEMPLATE.trim();
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    let (mut msg, _) = super::wire::parse_frame(&bytes).expect("built-in licence template parses");
    msg.transaction = transaction;
    msg
}

/// A property's value, in the shapes the capture shows.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    /// A `u32` (also a bool, as 0/1), value struct `{v}`.
    U32(u32),
    /// An `f32`, value struct `{v}`.
    F32(f32),
    /// A list of `f32` laid out directly (display area, track box).
    Floats(Vec<f32>),
    /// A string in the value struct's variable area.
    Str(String),
}

impl PropValue {
    /// The value struct for a property response or notification.
    fn value_struct(&self) -> Struct {
        match self {
            PropValue::U32(v) => Struct::new(vec![*v]),
            PropValue::F32(v) => Struct::new(vec![v.to_bits()]),
            PropValue::Floats(xs) => Struct::new(xs.iter().map(|f| f.to_bits()).collect()),
            PropValue::Str(s) => {
                // One field, the offset of the string in the var area (4: the
                // first byte after the length word), then the NUL-terminated
                // string padded to a 4-byte boundary.
                let mut var = s.clone().into_bytes();
                var.push(0);
                while var.len() % 4 != 0 {
                    var.push(0);
                }
                Struct::with_var(vec![4], var)
            }
        }
    }
}

/// A property get response (op 6): `{error, property}` then the value.
pub fn property_get_response(transaction: u32, property: u32, value: &PropValue) -> Message {
    Message::new(
        transaction,
        op::PROPERTY_GET_RESPONSE,
        vec![Struct::new(vec![0, property]), value.value_struct()],
    )
}

/// A property enumerate response (op 8): `{error, property, count}` then the
/// listed values. For output frequency the device offers just `[33]`.
pub fn property_enumerate_response(transaction: u32, property: u32, values: &[PropValue]) -> Message {
    let mut parts = vec![Struct::new(vec![0, property, values.len() as u32])];
    parts.extend(values.iter().map(|v| v.value_struct()));
    Message::new(transaction, op::PROPERTY_ENUMERATE_RESPONSE, parts)
}

/// A property notification (op 2), sent on the subscription channel when a
/// value changes: `{property}` then the value.
pub fn property_notification(property: u32, value: &PropValue) -> Message {
    Message::new(0, op::PROPERTY_NOTIFICATION, vec![Struct::new(vec![property]), value.value_struct()])
}

/// A plain command response (op 10) with an empty result: `{error, command}`
/// then an empty results struct. The runtime always sends the results struct
/// (size 4, no fields) even when a command returns nothing; omitting it makes
/// PETM reject the response, which aborts calibration right after the clear.
pub fn command_ok(transaction: u32, command: u32) -> Message {
    Message::new(transaction, op::COMMAND_RESPONSE, vec![Struct::new(vec![0, command]), Struct::new(vec![])])
}

/// A command response that echoes a single `u32` argument (as the stream
/// start/stop responses do).
pub fn command_ok_echo(transaction: u32, command: u32, arg: u32) -> Message {
    Message::new(transaction, op::COMMAND_RESPONSE, vec![Struct::new(vec![0, command]), Struct::new(vec![arg])])
}

/// Response to a property notification start (command 5): `{error, 5}` then
/// the property id and its current value, as the runtime answers it.
pub fn notification_start_response(transaction: u32, property: u32, value: &PropValue) -> Message {
    Message::new(
        transaction,
        op::COMMAND_RESPONSE,
        vec![Struct::new(vec![0, 5]), Struct::new(vec![property]), value.value_struct()],
    )
}

/// One eye of an `ADVANCED_GAZE` frame, in the field order the SDK's
/// `copy_eye_v0` writes (19 words).
#[derive(Debug, Clone, Copy, Default)]
pub struct EyeGaze {
    pub origin_valid: bool,
    pub origin_mm: [f32; 3],
    pub track_box: [f32; 3],
    pub point_valid: bool,
    pub point_mm: [f32; 3],
    pub point_display: [f32; 2],
    pub eyeball_valid: bool,
    pub eyeball_mm: [f32; 3],
    pub pupil_valid: bool,
    pub pupil_mm: f32,
}

impl EyeGaze {
    fn push(&self, w: &mut Vec<u32>) {
        let f = |x: f32| x.to_bits();
        w.push(self.origin_valid as u32);
        w.extend(self.origin_mm.map(f));
        w.extend(self.track_box.map(f));
        w.push(self.point_valid as u32);
        w.extend(self.point_mm.map(f));
        w.extend(self.point_display.map(f));
        w.push(self.eyeball_valid as u32);
        w.extend(self.eyeball_mm.map(f));
        w.push(self.pupil_valid as u32);
        w.push(f(self.pupil_mm));
    }
}

/// Stream id of `ADVANCED_GAZE`.
pub const STREAM_ADVANCED_GAZE: u32 = 6;
/// Compound stream id of `USER_POSITION_GUIDE_XYZ`.
pub const COMPOUND_USER_POSITION_GUIDE: u32 = 2;

/// An `ADVANCED_GAZE` stream frame (op 1): `{stream id}` then the payload
/// `{u64 timestamp, left eye, right eye, left tracked, right tracked}`.
pub fn advanced_gaze(timestamp_us: u64, left: &EyeGaze, right: &EyeGaze, left_tracked: bool, right_tracked: bool) -> Message {
    let mut w = vec![(timestamp_us >> 32) as u32, timestamp_us as u32];
    left.push(&mut w);
    right.push(&mut w);
    w.push(left_tracked as u32);
    w.push(right_tracked as u32);
    Message::new(0, op::STREAM, vec![Struct::new(vec![STREAM_ADVANCED_GAZE]), Struct::new(w)])
}

/// A `USER_POSITION_GUIDE_XYZ` compound stream frame (op 11): `{compound id}`
/// then `{u64 timestamp, left valid, left xyz, right valid, right xyz}`.
pub fn user_position_guide(timestamp_us: u64, left_valid: bool, left: [f32; 3], right_valid: bool, right: [f32; 3]) -> Message {
    let f = |x: f32| x.to_bits();
    let mut w = vec![(timestamp_us >> 32) as u32, timestamp_us as u32];
    w.push(left_valid as u32);
    w.extend(left.map(f));
    w.push(right_valid as u32);
    w.extend(right.map(f));
    Message::new(0, op::COMPOUND_STREAM, vec![Struct::new(vec![COMPOUND_USER_POSITION_GUIDE]), Struct::new(w)])
}

// Property ids, from the runtime's strings and the capture.
pub mod prop {
    pub const DISPLAY_AREA: u32 = 1;
    pub const POWER_SAVE_ACTIVE: u32 = 2;
    pub const DEVICE_PAUSED: u32 = 4;
    pub const EXCLUSIVE_MODE: u32 = 5;
    pub const CALIBRATION_ID: u32 = 6;
    pub const CALIBRATION_ACTIVE: u32 = 7;
    pub const DEVICE_NAME: u32 = 8;
    pub const OUTPUT_FREQUENCY: u32 = 9;
    pub const TRACK_BOX_COMBINED_GAZE: u32 = 15;
    pub const ENABLED_EYE: u32 = 16;
    pub const FAULTS: u32 = 21;
    pub const WARNINGS: u32 = 22;
}

#[cfg(test)]
mod tests {
    use super::super::wire::write_frame;
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn connect_template_roundtrips() {
        // Rebuilding each template with its own transaction id reproduces the
        // captured frame exactly.
        assert_eq!(write_frame(&connect_response(1, 2)), hex(include_str!("../../assets/prp_connect_response.hex").trim()));
        assert_eq!(write_frame(&connect_response(1, 1)), hex(include_str!("../../assets/prp_connect_response_1lic.hex").trim()));
        assert_eq!(write_frame(&connect_response(1, 0)), hex(include_str!("../../assets/prp_connect_response_discovery.hex").trim()));
    }

    #[test]
    fn property_responses_match_capture() {
        // Bytes from win/cap-272-main.txt (…299 session).
        assert_eq!(
            write_frame(&property_get_response(2, prop::DEVICE_NAME, &PropValue::Str("IS5_Large_Eyetracker_5".into()))),
            hex("54505250480000001c5052500c0000000200000006000000040000000c00000000000000080000000400000008000000040000001c0000004953355f4c617267655f457965747261636b65725f35000000000000"),
        );
        assert_eq!(
            write_frame(&property_get_response(4, prop::OUTPUT_FREQUENCY, &PropValue::F32(33.0))),
            hex("5450525030000000645052500c00000004000000060000000400000 00c00000000000000090000000400000008000000000004420400000000000000".replace(' ', "").as_str()),
        );
        assert_eq!(
            write_frame(&property_get_response(8, prop::CALIBRATION_ID, &PropValue::U32(0x9a08954e))),
            hex("5450525030000000645052500c0000000800000006000000040000000c000000000000000600000004000000080000004e95089a0400000000000000"),
        );
        assert_eq!(
            write_frame(&property_get_response(9, prop::CALIBRATION_ACTIVE, &PropValue::U32(0))),
            hex("5450525030000000645052500c0000000900000006000000040000000c00000000000000070000000400000008000000000000000400000000000000"),
        );
        // Exact bits from the capture, so the test checks layout, not float
        // literal rounding (in use the floats come straight from the device).
        let display = PropValue::Floats(
            [0xc32a8059u32, 0x43553878, 0x428d89b0, 0x432dc9f2, 0x43553878, 0x428d89b0, 0xc32a8059, 0x412446fc, 0xc04666bb]
                .iter()
                .map(|&b| f32::from_bits(b))
                .collect(),
        );
        assert_eq!(
            write_frame(&property_get_response(3, prop::DISPLAY_AREA, &display)),
            hex("5450525050000000045052500c0000000300000006000000040000000c0000000000000001000000040000002800000059802ac378385543b0898d42f2c92d4378385543b0898d4259802ac3fc462441bb6646c00400000000000000"),
        );
    }

    fn f(bits: u32) -> f32 {
        f32::from_bits(bits)
    }
    fn eye(ws: &[u32]) -> EyeGaze {
        EyeGaze {
            origin_valid: ws[0] != 0,
            origin_mm: [f(ws[1]), f(ws[2]), f(ws[3])],
            track_box: [f(ws[4]), f(ws[5]), f(ws[6])],
            point_valid: ws[7] != 0,
            point_mm: [f(ws[8]), f(ws[9]), f(ws[10])],
            point_display: [f(ws[11]), f(ws[12])],
            eyeball_valid: ws[13] != 0,
            eyeball_mm: [f(ws[14]), f(ws[15]), f(ws[16])],
            pupil_valid: ws[17] != 0,
            pupil_mm: f(ws[18]),
        }
    }

    #[test]
    fn gaze_frame_roundtrips_capture() {
        // The ADVANCED_GAZE payload words of one captured frame (win/cap-272-main.txt).
        let w: Vec<u32> = vec![
            0, 2473579067, 1, 3243605202, 3246713653, 1143011825, 1057423814, 1057537336, 1056579260,
            1, 1077240227, 1126746513, 1113219805, 1057022612, 1046491150, 1, 3243416915, 3247430295,
            1143083727, 1, 1084556064, 0, 1134419813, 3242116962, 1142973377, 1053475898, 1057347742,
            1056556518, 0, 1081568182, 1125744272, 1112775478, 1057359476, 1047392650, 0, 1134381693,
            3243354058, 1143000899, 0, 1084894854, 1, 1,
        ];
        let ts = (u64::from(w[0]) << 32) | u64::from(w[1]);
        let built = advanced_gaze(ts, &eye(&w[2..21]), &eye(&w[21..40]), w[40] != 0, w[41] != 0);
        // Rebuild the exact payload words and compare.
        let rebuilt: Vec<u32> = match &built.parts[1] {
            Struct { words, .. } => words.clone(),
        };
        assert_eq!(rebuilt, w);
    }

    #[test]
    fn position_guide_roundtrips_capture() {
        // ts and per-eye valid+xyz from a captured USER_POSITION_GUIDE frame.
        let m = user_position_guide(
            2470497809,
            true,
            [f(0x3f08a4c0), f(0x3f084c40), f(0x3efcec00)],
            true,
            [f(0x3ece6900), f(0x3f04106c), f(0x3efe5300)],
        );
        // Shape: id struct {2}, payload 10 words.
        assert_eq!(m.operation, op::COMPOUND_STREAM);
        assert_eq!(m.parts[0].words, vec![2]);
        assert_eq!(m.parts[1].words.len(), 10);
        assert_eq!(m.parts[1].words[0..2], [0, 2470497809]);
    }
}
