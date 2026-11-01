//! The PRP server loop: tie a client's three channels to the TTP session.
//!
//! This first cut answers the connection and the property reads and
//! subscriptions PETM makes during discovery and device setup, serving live
//! values read from the tracker over TTP. Streams (gaze, position guide),
//! calibration and licence forwarding are not here yet; their requests are
//! acknowledged so the client is not left waiting.

use super::translate::{self, prop, EyeGaze, PropValue};
use super::transport::PrpClient;
use super::wire::{op, Message};
use crate::ttp::calibration::{self, cmd as calcmd};
use crate::ttp::gaze83::{self, EyeFrame};
use crate::ttp::session::Session;
use crate::ttp::{facts, protocol::cmd, protocol::parse_message, protocol::STREAM_ID_GAZE};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Which streams the client is subscribed to.
#[derive(Default)]
struct Subs {
    gaze: AtomicBool,
    position: AtomicBool,
    device_started: AtomicBool,
}

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
    let PrpClient { mut command, subscription, mut timesync, .. } = client;
    let subs = Arc::new(Subs::default());

    // Forward device gaze frames to the subscription channel, re-encoded as
    // the PRP streams the client asked for.
    if let Some(rx) = session.take_streams() {
        let subs = subs.clone();
        std::thread::spawn(move || {
            let sub = subscription;
            for msg in rx {
                let Some(parsed) = parse_message(&msg) else { continue };
                if parsed.id != STREAM_ID_GAZE {
                    continue;
                }
                let Some(frame) = gaze83::decode_gaze_frame(&parsed) else { continue };
                if subs.gaze.load(Ordering::Relaxed) {
                    let g = translate::advanced_gaze(
                        frame.device_ts_us,
                        &eye_gaze(&frame.left),
                        &eye_gaze(&frame.right),
                        frame.left.tracked,
                        frame.right.tracked,
                    );
                    if sub.write_frame(&g).is_err() {
                        break;
                    }
                }
                if subs.position.load(Ordering::Relaxed) {
                    let p = translate::user_position_guide(
                        frame.device_ts_us,
                        frame.left.position_guide.valid,
                        f3(frame.left.position_guide.value),
                        frame.right.position_guide.valid,
                        f3(frame.right.position_guide.value),
                    );
                    if sub.write_frame(&p).is_err() {
                        break;
                    }
                }
            }
        });
    }

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
            op::COMMAND_REQUEST => handle_command(&session, &subs, &req),
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

fn handle_command(session: &Session, subs: &Subs, req: &Message) -> Message {
    let tx = req.transaction;
    let command = cmd_id(req);
    let arg = property_of_arg(req);
    match command {
        // Connect: return the device's capabilities (templated) and move on.
        // The licence PETM sent is not acted on yet (it is never blanked or
        // forged); forwarding it to the device comes with the calibration
        // work.
        1 => translate::connect_response(tx),
        // Property notification start: acknowledge the subscription. Later
        // changes (e.g. calibration id) are pushed on the subscription
        // channel once notification forwarding lands.
        5 => translate::command_ok_echo(tx, command, arg),
        // Stream start (3) / stop (4): gaze (stream id 6).
        3 => {
            if arg == translate::STREAM_ADVANCED_GAZE {
                subs.gaze.store(true, Ordering::Relaxed);
                ensure_device_stream(session, subs);
            }
            translate::command_ok_echo(tx, command, arg)
        }
        4 => {
            if arg == translate::STREAM_ADVANCED_GAZE {
                subs.gaze.store(false, Ordering::Relaxed);
            }
            translate::command_ok_echo(tx, command, arg)
        }
        // Compound stream start (23) / stop (24): position guide (id 2).
        23 => {
            if arg == translate::COMPOUND_USER_POSITION_GUIDE {
                subs.position.store(true, Ordering::Relaxed);
                ensure_device_stream(session, subs);
            }
            translate::command_ok_echo(tx, command, arg)
        }
        24 => {
            if arg == translate::COMPOUND_USER_POSITION_GUIDE {
                subs.position.store(false, Ordering::Relaxed);
            }
            translate::command_ok_echo(tx, command, arg)
        }
        // Calibration. Each PRP command maps to its TTP command; the ported
        // `calibration` module builds the payloads. Best-effort: this path is
        // validated with PETM driving a real calibration, not locally.
        9 => calib(session, tx, command, calcmd::CLEAR, &[0, 0]),
        15 => calib(session, tx, command, calcmd::START, &[0, 0]),
        11 => {
            let x = f32::from_bits(req.parts.get(1).and_then(|s| s.word(0)).unwrap_or(0));
            let y = f32::from_bits(req.parts.get(1).and_then(|s| s.word(1)).unwrap_or(0));
            let mut payload = vec![0u8, 0];
            payload.extend_from_slice(&calibration::collect_payload(x, y, calibration::EYES_BOTH));
            calib(session, tx, command, calcmd::COLLECT_2D, &payload)
        }
        16 => calib(session, tx, command, calcmd::COMPUTE, &[0, 0]),
        17 => calibration_retrieve(session, tx, command),
        10 => calib(session, tx, command, calcmd::STOP, &[0, 0]),
        // Anything else (e.g. licence retrieve): acknowledge so the client is
        // not blocked. The bridge does not validate or forge any licence; the
        // device streams over TTP without a licence gate.
        _ => translate::command_ok(tx, command),
    }
}

/// Run a TTP calibration command and acknowledge the PRP command.
fn calib(session: &Session, tx: u32, prp_cmd: u32, ttp_cmd: u32, payload: &[u8]) -> Message {
    if let Err(e) = session.command(ttp_cmd, payload) {
        eprintln!("prp: calibration TTP command {ttp_cmd} failed: {e}");
    }
    translate::command_ok(tx, prp_cmd)
}

/// PRP calibration retrieve: read the active calibration (TTP 1100) and return
/// its blob as `{size, offset}` + the bytes, as the capture shows.
fn calibration_retrieve(session: &Session, tx: u32, prp_cmd: u32) -> Message {
    let blob = session
        .command(calcmd::READ, &[0, 0])
        .ok()
        .and_then(|b| parse_message(&b).and_then(|m| calibration::blob_from_payload(m.payload.get(2..)?).map(<[u8]>::to_vec)))
        .unwrap_or_default();
    let mut var = blob;
    while var.len() % 4 != 0 {
        var.push(0);
    }
    Message::new(
        tx,
        op::COMMAND_RESPONSE,
        vec![
            super::wire::Struct::new(vec![0, prp_cmd]),
            super::wire::Struct::with_var(vec![var.len() as u32, 4], var),
        ],
    )
}

/// Start the device's 0x500 gaze stream once; both PRP streams come from it.
fn ensure_device_stream(session: &Session, subs: &Subs) {
    if !subs.device_started.swap(true, Ordering::Relaxed) {
        if let Err(e) = session.start_stream(STREAM_ID_GAZE) {
            eprintln!("prp: starting the gaze stream failed: {e}");
            subs.device_started.store(false, Ordering::Relaxed);
        }
    }
}

/// Map a decoded TTP eye to the PRP `ADVANCED_GAZE` eye fields.
fn eye_gaze(e: &EyeFrame) -> EyeGaze {
    EyeGaze {
        origin_valid: e.origin_display_mm.valid,
        origin_mm: f3(e.origin_display_mm.value),
        track_box: f3(e.track_box.value),
        point_valid: e.gaze_point_norm.valid,
        point_mm: f3(e.gaze_point_tracker_mm.value),
        point_display: [e.gaze_point_norm.value[0] as f32, e.gaze_point_norm.value[1] as f32],
        eyeball_valid: e.eyeball_center_mm.valid,
        eyeball_mm: f3(e.eyeball_center_mm.value),
        pupil_valid: e.pupil_diameter_mm.valid,
        pupil_mm: e.pupil_diameter_mm.value as f32,
    }
}

fn f3(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

/// The single `u32` argument of a command (its second struct's first field).
fn property_of_arg(req: &Message) -> u32 {
    req.parts.get(1).and_then(|s| s.word(0)).unwrap_or(0)
}
