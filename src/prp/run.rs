//! The PRP server loop: tie a client's three channels to the TTP session.
//!
//! This first cut answers the connection and the property reads and
//! subscriptions PETM makes during discovery and device setup, serving live
//! values read from the tracker over TTP. Streams (gaze, position guide),
//! calibration and licence forwarding are not here yet; their requests are
//! acknowledged so the client is not left waiting.

use super::transport::{Channel, Listener};
use super::translate::{self, prop, EyeGaze, PropValue};
use super::wire::{op, Message, Struct};
use crate::ttp::calibration::{self, cmd as calcmd};
use crate::ttp::gaze83::{self, EyeFrame};
use crate::ttp::session::Session;
use crate::ttp::{facts, protocol::cmd, protocol::parse_message, protocol::STREAM_ID_GAZE};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Which streams the client is subscribed to.
#[derive(Default)]
struct Subs {
    gaze: AtomicBool,
    position: AtomicBool,
    device_started: AtomicBool,
}

/// The subscription channel the stream forwarder writes to, when a client has
/// opened one.
type SubSink = Arc<Mutex<Option<Channel>>>;

/// The property values PETM reads, filled from the device at start-up.
#[derive(Debug)]
pub struct Properties {
    /// The display area PETM sees. PETM writes it (PROPERTY_SET) when a display
    /// setup is selected and reads it back to confirm; it must round-trip, so
    /// it is interior-mutable. Seeded from the device's own value.
    pub display_area: Mutex<[f32; 9]>,
    pub track_box: [f32; 24],
    pub output_frequency: f32,
    pub device_name: String,
    /// The active calibration's id. PETM reads it and expects it to change
    /// after a compute; the runtime generates it client-side (it is an opaque
    /// identifier for the calibration the device computed, not a licence), so
    /// the bridge assigns one on compute. Interior-mutable.
    pub calibration_id: AtomicU32,
    pub calibration_active: bool,
    pub enabled_eye: u32,
}

impl Properties {
    /// Read the device's geometry and identity once.
    pub fn read_from(session: &Session) -> Self {
        let mut p = Properties {
            display_area: Mutex::new([0.0; 9]),
            track_box: [0.0; 24],
            output_frequency: 33.0,
            device_name: "Tobii Eye Tracker 5".into(),
            calibration_id: AtomicU32::new(0),
            calibration_active: false,
            enabled_eye: 0,
        };
        if let Ok(b) = session.command(cmd::DISPLAY_AREA_GET, &[0, 0]) {
            if let Some(a) = parse_message(&b).and_then(|m| facts::display_area_9(&m)) {
                *p.display_area.lock().unwrap() = a;
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
            prop::DISPLAY_AREA => PropValue::Floats(self.display_area.lock().unwrap().to_vec()),
            prop::TRACK_BOX_COMBINED_GAZE => PropValue::Floats(self.track_box.to_vec()),
            prop::OUTPUT_FREQUENCY => PropValue::F32(self.output_frequency),
            prop::DEVICE_NAME => PropValue::Str(self.device_name.clone()),
            prop::CALIBRATION_ID => PropValue::U32(self.calibration_id.load(Ordering::Relaxed)),
            prop::CALIBRATION_ACTIVE => PropValue::U32(self.calibration_active.into()),
            prop::ENABLED_EYE => PropValue::U32(self.enabled_eye),
            prop::POWER_SAVE_ACTIVE | prop::DEVICE_PAUSED | prop::EXCLUSIVE_MODE => PropValue::U32(0),
            prop::FAULTS | prop::WARNINGS => PropValue::Str(String::new()),
            _ => return None,
        })
    }

    /// Store the display area PETM selected, so later reads of DISPLAY_AREA
    /// return it (PETM confirms the setup by reading back what it wrote).
    fn set_display_area(&self, area: [f32; 9]) {
        *self.display_area.lock().unwrap() = area;
    }

    /// Assign and store a new calibration id (what the runtime does after a
    /// compute). Returns it so the change can be notified. Never zero.
    fn new_calibration_id(&self) -> u32 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32)
            | 1;
        self.calibration_id.store(now, Ordering::Relaxed);
        now
    }
}

/// Run the PRP server: accept command connections (each served on its own
/// thread) and, in the background, the subscription and time sync channels and
/// the gaze-stream forwarder. The three channels are independent, so discovery
/// (command only) is never held up by the others. Loops forever.
pub fn run_server(session: Arc<Session>, listener: Listener, props: Arc<Properties>) -> std::io::Result<()> {
    let listener = Arc::new(listener);
    let subs = Arc::new(Subs::default());
    let sink: SubSink = Arc::new(Mutex::new(None));

    // Subscription channel: accept it and keep the latest as the stream sink.
    {
        let (listener, sink) = (listener.clone(), sink.clone());
        std::thread::spawn(move || loop {
            match listener.accept_sub() {
                Ok(ch) => *sink.lock().unwrap() = Some(ch),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        });
    }

    // Time sync channel: answer timestamp requests.
    {
        let listener = listener.clone();
        std::thread::spawn(move || loop {
            match listener.accept_timesync() {
                Ok(ch) => serve_timesync(ch),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        });
    }

    // Forwarder: decode device gaze and write the subscribed PRP streams to
    // the current subscription sink.
    if let Some(rx) = session.take_streams() {
        let (subs, sink) = (subs.clone(), sink.clone());
        std::thread::spawn(move || {
            // Gaze/stream forwarding is high-volume; keep it behind its own flag
            // so PRP_DEBUG alone shows only command/calibration activity.
            let debug = std::env::var("PRP_DEBUG_GAZE").is_ok();
            let (mut total, mut gaze_frames, mut fwd) = (0u64, 0u64, 0u64);
            for msg in rx {
                total += 1;
                let Some(parsed) = parse_message(&msg) else { continue };
                if debug && total <= 20 {
                    eprintln!("fwd: stream id {:#x} ({} B)", parsed.id, msg.len());
                }
                if parsed.id != STREAM_ID_GAZE {
                    continue;
                }
                gaze_frames += 1;
                let Some(frame) = gaze83::decode_gaze_frame(&parsed) else { continue };
                let gaze = subs.gaze.load(Ordering::Relaxed);
                let position = subs.position.load(Ordering::Relaxed);
                if debug && gaze_frames % 60 == 1 {
                    eprintln!("fwd: gaze#{gaze_frames} sub(g={gaze},p={position}) sink={} forwarded={fwd}", sink.lock().unwrap().is_some());
                }
                if !gaze && !position {
                    continue;
                }
                let mut guard = sink.lock().unwrap();
                let Some(ch) = guard.as_ref() else { continue };
                let mut ok = true;
                if gaze {
                    let g = translate::advanced_gaze(
                        frame.device_ts_us,
                        &eye_gaze(&frame.left),
                        &eye_gaze(&frame.right),
                        frame.left.tracked,
                        frame.right.tracked,
                    );
                    ok &= ch.write_frame(&g).is_ok();
                }
                if ok && position {
                    let p = translate::user_position_guide(
                        frame.device_ts_us,
                        frame.left.position_guide.valid,
                        f3(frame.left.position_guide.value),
                        frame.right.position_guide.valid,
                        f3(frame.right.position_guide.value),
                    );
                    ok &= ch.write_frame(&p).is_ok();
                }
                if ok {
                    fwd += 1;
                } else {
                    *guard = None; // sink gone; wait for the next subscription
                }
            }
        });
    }

    // Command channels: one PETM opens many (discovery), then a lasting one.
    // A failed accept (a probe that closed mid-handshake) must not stop the
    // server — log and carry on.
    loop {
        match listener.accept_command() {
            Ok(ch) => {
                let (session, props, subs, sink) = (session.clone(), props.clone(), subs.clone(), sink.clone());
                std::thread::spawn(move || serve_command(ch, &session, &props, &subs, &sink));
            }
            Err(e) => {
                eprintln!("prp: command accept failed: {e}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Serve one command connection until it closes.
fn serve_command(mut command: Channel, session: &Session, props: &Properties, subs: &Subs, sink: &SubSink) {
    let debug = std::env::var("PRP_DEBUG").is_ok();
    loop {
        let req = match command.read_frame() {
            Ok(r) => r,
            Err(e) => {
                if debug {
                    eprintln!("cmd: channel closed ({e})");
                }
                break;
            }
        };
        if debug {
            let detail = if req.operation == op::COMMAND_REQUEST { cmd_id(&req) } else { property_of(&req) };
            eprintln!("cmd: op {} detail {}", req.operation, detail);
        }
        let tx = req.transaction;
        let resp = match req.operation {
            op::PROPERTY_GET_REQUEST => {
                let property = property_of(&req);
                let v = props.value(property).unwrap_or(PropValue::U32(0));
                translate::property_get_response(tx, property, &v)
            }
            op::PROPERTY_ENUMERATE_REQUEST => {
                let property = property_of(&req);
                let vals = props.value(property).into_iter().collect::<Vec<_>>();
                translate::property_enumerate_response(tx, property, &vals)
            }
            // Property set: echo the property and the value back as accepted.
            // DISPLAY_AREA is the one PETM actually writes (selecting a display
            // setup): store it so reads return it, and notify the change, or
            // PETM reports "we don't recognize this display setup" and will not
            // let the setup be selected.
            op::PROPERTY_SET_REQUEST => {
                let property = property_of(&req);
                let value = req.parts.get(1).cloned().unwrap_or_default();
                if property == prop::DISPLAY_AREA {
                    if let Ok(area) = <[f32; 9]>::try_from(
                        value.words.iter().map(|w| f32::from_bits(*w)).collect::<Vec<_>>(),
                    ) {
                        props.set_display_area(area);
                        push_notification(sink, prop::DISPLAY_AREA, PropValue::Floats(area.to_vec()));
                        if debug {
                            eprintln!("display area set to {area:?}");
                        }
                    }
                }
                Message::new(tx, op::PROPERTY_SET_RESPONSE, vec![Struct::new(vec![0, property]), value])
            }
            op::COMMAND_REQUEST => handle_command(session, props, subs, sink, &req),
            other => {
                eprintln!("prp: unhandled operation {other}");
                continue;
            }
        };
        if debug {
            let summary: Vec<String> = resp
                .parts
                .iter()
                .take(3)
                .map(|s| format!("{:?}+{}v", s.words, s.var.len()))
                .collect();
            eprintln!("cmd> op {} resp {}", resp.operation, summary.join(" "));
        }
        if command.write_frame(&resp).is_err() {
            break;
        }
    }
}

/// Answer timestamp requests on a time sync channel until it closes.
fn serve_timesync(mut ch: Channel) {
    let start = Instant::now();
    while let Ok(req) = ch.read_frame() {
        if req.operation == op::COMMAND_REQUEST {
            let us = start.elapsed().as_micros() as u32;
            let resp = Message::new(
                req.transaction,
                op::COMMAND_RESPONSE,
                vec![super::wire::Struct::new(vec![0, cmd_id(&req)]), super::wire::Struct::new(vec![us])],
            );
            if ch.write_frame(&resp).is_err() {
                break;
            }
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

/// Push a property-change notification to the current subscription sink (tx 0,
/// as the runtime does). PETM waits for these during calibration (e.g.
/// CALIBRATION_ACTIVE after clear) before it will show the next screen.
fn push_notification(sink: &SubSink, property: u32, value: PropValue) {
    let debug = std::env::var("PRP_DEBUG").is_ok();
    match sink.lock().unwrap().as_ref() {
        Some(ch) => {
            let ok = ch.write_frame(&translate::property_notification(property, &value)).is_ok();
            if debug {
                eprintln!("notify: property {property} pushed (write ok={ok})");
            }
        }
        None => {
            if debug {
                eprintln!("notify: property {property} NOT pushed (no sub sink)");
            }
        }
    }
}

fn handle_command(session: &Session, props: &Properties, subs: &Subs, sink: &SubSink, req: &Message) -> Message {
    let tx = req.transaction;
    let command = cmd_id(req);
    let arg = property_of_arg(req);
    match command {
        // Connect: return the device's capabilities (templated) and move on.
        // The licence PETM sent is not acted on yet (it is never blanked or
        // forged); forwarding it to the device comes with the calibration
        // work.
        1 => {
            // The connect request's device-info struct (parts[1]) carries the
            // number of licences at word 1; the runtime's response grows by one
            // result per licence, so reply with the matching captured response.
            let licences = req.parts.get(1).and_then(|s| s.word(1)).unwrap_or(0);
            if std::env::var("PRP_DEBUG").is_ok() {
                eprintln!("connect: {licences} licence(s)");
            }
            translate::connect_response(tx, licences)
        }
        // Property notification start: reply with the property and its current
        // value (the runtime does; an acknowledgement alone makes PETM abort).
        5 => {
            let v = props.value(arg).unwrap_or(PropValue::U32(0));
            translate::notification_start_response(tx, arg, &v)
        }
        // Property notification stop (counterpart of 5): echo the property id
        // back with no value, as the runtime does (RESPONSE 6 err=0 -> id).
        // A bare ack without the echoed id makes PETM retry then drop the
        // device.
        6 => translate::command_ok_echo(tx, command, arg),
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
        // Calibration clear: the runtime marks calibration active and notifies
        // it before answering. PETM waits for this before showing the points.
        9 => {
            // PETM's "clear" arrives before the session is open; a device 1060
            // here is refused (BAD_STATE). The real runtime treats it as a
            // client-side prepare (tobii-linux tobiid: CALIBRATION_CLEAR =
            // prepare, no device command). Mark active and acknowledge; the
            // device's points are cleared by the start macro's own 1060.
            push_notification(sink, prop::CALIBRATION_ACTIVE, PropValue::U32(1));
            translate::command_ok(tx, command)
        }
        // Calibration start: the device needs the full captured session macro,
        // not 1010 alone — read the active calibration (1100), then 1010 start,
        // 1060 clear, 1110 write to seed the sample ring. Without the seed the
        // device refuses every collect with CALIBRATION_NOT_STARTED
        // (0x20000503). See ttp::calibration and tobii-linux tobiid::seed.
        15 => {
            push_notification(sink, prop::CALIBRATION_ID, PropValue::U32(0));
            calibration_session_start(session, tx, command)
        }
        11 => {
            let x = f32::from_bits(req.parts.get(1).and_then(|s| s.word(0)).unwrap_or(0));
            let y = f32::from_bits(req.parts.get(1).and_then(|s| s.word(1)).unwrap_or(0));
            eprintln!("calib: COLLECT_2D point ({x:.4}, {y:.4})");
            // collect_payload already begins with the `00 00` a payload starts
            // with (TlvWriter); do not prepend another.
            calib(session, tx, command, calcmd::COLLECT_2D, &calibration::collect_payload(x, y, calibration::EYES_BOTH))
        }
        // Calibration compute and apply: the device computes, then the runtime
        // assigns a new calibration id and notifies it. PETM waits for the id
        // notification before continuing (it fails after the first points
        // without it).
        16 => {
            let resp = calib(session, tx, command, calcmd::COMPUTE, &[0, 0]);
            push_notification(sink, prop::CALIBRATION_ID, PropValue::U32(props.new_calibration_id()));
            resp
        }
        17 => calibration_retrieve(session, tx, command),
        // Calibration stop: the runtime marks calibration inactive and notifies.
        10 => {
            push_notification(sink, prop::CALIBRATION_ACTIVE, PropValue::U32(0));
            calib(session, tx, command, calcmd::STOP, &[0, 0])
        }
        // Licence key retrieve: return the licence the device holds (replayed
        // from this device; not a fabricated verdict). PETM reads it during
        // discovery and will not list the device without it.
        20 => translate::license_retrieve_response(tx),
        // Anything else: acknowledge so the client is not blocked.
        _ => translate::command_ok(tx, command),
    }
}

/// Run a TTP calibration command and acknowledge the PRP command.
fn calib(session: &Session, tx: u32, prp_cmd: u32, ttp_cmd: u32, payload: &[u8]) -> Message {
    ttp_step(session, ttp_cmd, payload);
    translate::command_ok(tx, prp_cmd)
}

/// PRP calibration start: the full captured device macro that opens a usable
/// session — read the active calibration (1100), then 1010 start, 1060 clear,
/// 1110 write to seed the device's sample ring with that calibration. 1010
/// alone leaves the session in a state that refuses collects.
fn calibration_session_start(session: &Session, tx: u32, prp_cmd: u32) -> Message {
    // Seed with the calibration embedded in the init replay: the device holds
    // no user calibration to read back (a 1100 read is empty), but its sample
    // ring must be seeded with a valid blob or every collect is refused.
    let blob = crate::ttp::session::embedded_calibration_blob();
    eprintln!("calib: start macro (seed blob {} bytes)", blob.len());
    ttp_step(session, calcmd::START, &[0, 0]);
    ttp_step(session, calcmd::CLEAR, &[0, 0]);
    // write_payload already begins with `00 00`; pass it as-is.
    ttp_step(session, calcmd::WRITE, &calibration::write_payload(&blob));
    translate::command_ok(tx, prp_cmd)
}

/// Run one TTP command and log its response error (not the PRP ack).
fn ttp_step(session: &Session, ttp_cmd: u32, payload: &[u8]) {
    match session.command(ttp_cmd, payload) {
        Ok(b) => {
            let err = parse_message(&b).map(|m| m.error).unwrap_or(0);
            eprintln!("calib: TTP {ttp_cmd} -> error {err:#010x} ({} B response)", b.len());
        }
        Err(e) => eprintln!("prp: calibration TTP command {ttp_cmd} failed: {e}"),
    }
}

/// PRP calibration retrieve: read the active calibration (TTP 1100) and return
/// its blob as `{size, offset}` + the bytes, as the capture shows.
fn calibration_retrieve(session: &Session, tx: u32, prp_cmd: u32) -> Message {
    let blob = session
        .command(calcmd::READ, &[0, 0])
        .ok()
        // blob_from_payload skips the leading `00 00` itself (payload_tlvs);
        // pass the whole payload, as the working embedded-blob read does. The
        // earlier `get(2..)` skipped it twice and read an empty blob.
        .and_then(|b| parse_message(&b).and_then(|m| calibration::blob_from_payload(m.payload).map(<[u8]>::to_vec)))
        .unwrap_or_default();
    eprintln!("calib: RETRIEVE read blob {} bytes", blob.len());
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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The single `u32` argument of a command (its second struct's first field).
fn property_of_arg(req: &Message) -> u32 {
    req.parts.get(1).and_then(|s| s.word(0)).unwrap_or(0)
}
