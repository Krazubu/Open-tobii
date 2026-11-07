//! Minimal parsers for the TTP responses the PRP translator needs, reading
//! the same TLV fields as tobii-linux's `facts` but returning plain values
//! (no geometry vocabulary). Point coordinates come as 32.32 fixed-point in
//! 1024ths of a millimetre and are returned in millimetres.

use super::protocol::Message;
use super::tlv::{Tlv, FIELD_POINT_3D, UNITS_PER_MM};

const FIELD_DISPLAY_ID: u32 = 0x0001_0100;
const FIELD_OUTPUT_RATE: u32 = 0x0002_2af8;

/// The 3-D points (mm) a geometry reply carries, in wire order.
pub fn points_mm(msg: &Message<'_>) -> Vec<[f32; 3]> {
    let entries: Vec<Tlv<'_>> = msg.tlvs().collect();
    entries
        .windows(4)
        .filter(|w| w[0].field_id() == Some(FIELD_POINT_3D))
        .filter_map(|w| {
            Some([
                (w[1].fixed32()? / UNITS_PER_MM) as f32,
                (w[2].fixed32()? / UNITS_PER_MM) as f32,
                (w[3].fixed32()? / UNITS_PER_MM) as f32,
            ])
        })
        .collect()
}

/// Command 1430: the display area as the nine floats PETM expects
/// (top-left, top-right, bottom-left, each x/y/z in mm).
pub fn display_area_9(msg: &Message<'_>) -> Option<[f32; 9]> {
    let p = points_mm(msg);
    let c = p.get(..3)?;
    Some([c[0][0], c[0][1], c[0][2], c[1][0], c[1][1], c[1][2], c[2][0], c[2][1], c[2][2]])
}

/// The display id a 1430 reply may carry.
pub fn display_id(msg: &Message<'_>) -> Option<u32> {
    let entries: Vec<Tlv<'_>> = msg.tlvs().collect();
    entries
        .windows(2)
        .find(|w| w[0].field_id() == Some(FIELD_DISPLAY_ID))
        .and_then(|w| w[1].u32())
}

/// Command 1400: the eight track-box corners as 24 floats (mm), in wire order.
pub fn track_box_24(msg: &Message<'_>) -> Option<[f32; 24]> {
    let p = points_mm(msg);
    let c = p.get(..8)?;
    let mut out = [0f32; 24];
    for (i, corner) in c.iter().enumerate() {
        out[i * 3..i * 3 + 3].copy_from_slice(corner);
    }
    Some(out)
}

/// Command 1650: the current output rate (Hz).
pub fn output_rate(msg: &Message<'_>) -> Option<u32> {
    // Command 1650 carries two words after the output-rate field id: the sensor
    // rate then the output rate in Hz (132, 33 on the ET5). PETM wants the
    // output rate, so take the second word, not the first (which gave a wrong
    // 132 that made PETM reject the device before opening the positioning
    // guide).
    let entries: Vec<Tlv<'_>> = msg.tlvs().collect();
    entries
        .windows(3)
        .find(|w| w[0].field_id() == Some(FIELD_OUTPUT_RATE))
        .and_then(|w| w[2].u32())
}

/// Command 1420: the device identity strings, in order: serial, model,
/// generation, firmware.
pub fn device_strings(msg: &Message<'_>) -> Vec<String> {
    msg.tlvs().filter_map(|t| t.string()).collect()
}
