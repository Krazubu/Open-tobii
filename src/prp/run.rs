//! The PRP server loop: tie a client's three channels to the TTP session.
//!
//! This first cut answers the connection and the property reads and
//! subscriptions PETM makes during discovery and device setup, serving live
//! values read from the tracker over TTP. Streams (gaze, position guide),
//! calibration and licence forwarding are not here yet; their requests are
//! acknowledged so the client is not left waiting.

use super::translate::{self, prop, PropValue};
use super::transport::PrpClient;
use super::wire::{op, Message};
use crate::ttp::session::Session;
use crate::ttp::{facts, protocol::cmd, protocol::parse_message};
use std::sync::Arc;
use std::time::Instant;

/// The property values PETM reads, filled from the device at start-up.
#[derive(Debug, Clone)]
pub struct Properties {
    pub display_area: [f32; 9],
    pub track_box: [f32; 24],
    pub output_frequency: f32,
    pub device_name: String,
    pub calibration_id: u32,
    pub calibration_active: bool,
    pub enabled_eye: u32,
}

impl Properties {
    /// Read the device's geometry and identity once.
    pub fn read_from(session: &Session) -> Self {
        let mut p = Properties {
            display_area: [0.0; 9],
            track_box: [0.0; 24],
            output_frequency: 33.0,
            device_name: "Tobii Eye Tracker 5".into(),
            calibration_id: 0,
            calibration_active: false,
            enabled_eye: 0,
        };
        if let Ok(b) = session.command(cmd::DISPLAY_AREA_GET, &[0, 0]) {
            if let Some(a) = parse_message(&b).and_then(|m| facts::display_area_9(&m)) {
                p.display_area = a;
            }
        }
        if let Ok(b) = session.command(cmd::TRACK_BOX, &[0, 0]) {
            if let Some(a) = parse_message(&b).and_then(|m| facts::track_box_24(&m)) {
                p.track_box = a;
            }
        }
        if let Ok(b) = session.command(cmd::OUTPUT_RATE, &[0, 0]) {
            if let Some(hz) = parse_message(&b).and_then(|m| facts::output_rate(&m)) {
                p.output_frequency = hz as f32;
            }
        }
        if let Ok(b) = session.command(cmd::DEVICE_STRINGS, &[0, 0]) {
            if let Some(m) = parse_message(&b) {
                let s = facts::device_strings(&m);
                // serial, model, generation, firmware: PETM's device name is
                // the model.
                if let Some(model) = s.get(1) {
                    p.device_name = model.clone();
                }
            }
        }
        p
    }

    /// The PRP value for a property id, or `None` if we do not serve it.
    fn value(&self, property: u32) -> Option<PropValue> {
        Some(match property {
            prop::DISPLAY_AREA => PropValue::Floats(self.display_area.to_vec()),
            prop::TRACK_BOX_COMBINED_GAZE => PropValue::Floats(self.track_box.to_vec()),
            prop::OUTPUT_FREQUENCY => PropValue::F32(self.output_frequency),
            prop::DEVICE_NAME => PropValue::Str(self.device_name.clone()),
            prop::CALIBRATION_ID => PropValue::U32(self.calibration_id),
            prop::CALIBRATION_ACTIVE => PropValue::U32(self.calibration_active.into()),
            prop::ENABLED_EYE => PropValue::U32(self.enabled_eye),
            prop::POWER_SAVE_ACTIVE | prop::DEVICE_PAUSED | prop::EXCLUSIVE_MODE => PropValue::U32(0),
            prop::FAULTS | prop::WARNINGS => PropValue::Str(String::new()),
            _ => return None,
        })
    }
}

/// Serve one PRP client: the command channel (connection, property reads and
/// subscriptions, commands) and the time sync channel. Blocks until the
/// command channel closes.
pub fn serve(session: Arc<Session>, client: PrpClient, props: Arc<Properties>) {
    let PrpClient { mut command, subscription: _subscription, mut timesync, .. } = client;

    // Time sync on its own thread: answer timestamp requests with a device
    // time. Approximate for now (a monotonic microsecond counter); the real
    // device clock comes with the stream work.
    std::thread::spawn(move || {
        let start = Instant::now();
        while let Ok(req) = timesync.read_frame() {
            if req.operation == op::COMMAND_REQUEST {
                let us = start.elapsed().as_micros() as u32;
                let resp = Message::new(
                    req.transaction,
                    op::COMMAND_RESPONSE,
                    vec![super::wire::Struct::new(vec![0, cmd_id(&req)]), super::wire::Struct::new(vec![us])],
                );
                if timesync.write_frame(&resp).is_err() {
                    break;
                }
            }
        }
    });

    loop {
        let req = match command.read_frame() {
            Ok(r) => r,
            Err(_) => break,
        };
        let tx = req.transaction;
        let resp = match req.operation {
            op::PROPERTY_GET_REQUEST => {
                let property = property_of(&req);
                match props.value(property) {
                    Some(v) => translate::property_get_response(tx, property, &v),
                    None => translate::property_get_response(tx, property, &PropValue::U32(0)),
                }
            }
            op::PROPERTY_ENUMERATE_REQUEST => {
                let property = property_of(&req);
                // The device offers a single output frequency.
                let vals = props.value(property).into_iter().collect::<Vec<_>>();
                translate::property_enumerate_response(tx, property, &vals)
            }
            op::COMMAND_REQUEST => handle_command(&session, &props, &req),
            other => {
                eprintln!("prp: unhandled operation {other}");
                continue;
            }
        };
        if command.write_frame(&resp).is_err() {
            break;
        }
    }
}

/// The command id of an op-9 request (first field of its first struct).
fn cmd_id(req: &Message) -> u32 {
    req.parts.first().and_then(|s| s.word(0)).unwrap_or(0)
}

/// The property id of an op-5/op-7 request.
fn property_of(req: &Message) -> u32 {
    req.parts.first().and_then(|s| s.word(0)).unwrap_or(0)
}

fn handle_command(_session: &Session, _props: &Properties, req: &Message) -> Message {
    let tx = req.transaction;
    let command = cmd_id(req);
    match command {
        // Connect: return the device's capabilities (templated) and move on.
        // The licence PETM sent is not acted on yet (it is never blanked or
        // forged); forwarding it to the device comes with the calibration
        // work.
        1 => translate::connect_response(tx),
        // Property notification start: acknowledge the subscription. The
        // initial value and later changes are pushed on the subscription
        // channel once stream/notification forwarding lands.
        5 => translate::command_ok_echo(tx, command, property_of_arg(req)),
        // Stream and compound-stream start/stop: acknowledge; forwarding the
        // frames is the next step.
        3 | 4 | 23 | 24 => translate::command_ok_echo(tx, command, property_of_arg(req)),
        // Anything else (calibration, licence retrieve, …): acknowledge for
        // now so the client is not blocked.
        _ => translate::command_ok(tx, command),
    }
}

/// The single `u32` argument of a command (its second struct's first field).
fn property_of_arg(req: &Message) -> u32 {
    req.parts.get(1).and_then(|s| s.word(0)).unwrap_or(0)
}
