//! Translating PRP requests to TTP and the device's answers back to PRP.
//!
//! This module holds the pure encoders — the PRP responses the server emits —
//! each checked byte-for-byte against the capture. Wiring them to live TTP
//! values and to the pipe channels is done by the server loop.

use super::wire::{op, Message, Struct};

/// The connect response is a nested capabilities document. Rather than rebuild
/// that format, the bridge replays the one the device's own runtime produced
/// (captured from this ET5): its stream and property lists are exactly the
/// device's. Only the transaction id is swapped to match the request.
pub fn connect_response(transaction: u32) -> Message {
    const TEMPLATE: &str = include_str!("../../assets/prp_connect_response.hex");
    let bytes: Vec<u8> = {
        let hex = TEMPLATE.trim();
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    };
    let (mut msg, _) = super::wire::parse_frame(&bytes).expect("built-in connect template parses");
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

/// A plain command response (op 10) with no result payload: `{error, command}`.
pub fn command_ok(transaction: u32, command: u32) -> Message {
    Message::new(transaction, op::COMMAND_RESPONSE, vec![Struct::new(vec![0, command])])
}

/// A command response that echoes a single `u32` argument (as the stream
/// start/stop and notification-start responses do).
pub fn command_ok_echo(transaction: u32, command: u32, arg: u32) -> Message {
    Message::new(transaction, op::COMMAND_RESPONSE, vec![Struct::new(vec![0, command]), Struct::new(vec![arg])])
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
        // Rebuilding the template with its own transaction id reproduces the
        // captured frame exactly.
        let bytes = write_frame(&connect_response(1));
        let want = hex(include_str!("../../assets/prp_connect_response.hex").trim());
        assert_eq!(bytes, want);
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
}
