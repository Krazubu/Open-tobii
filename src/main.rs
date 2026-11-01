//! Prototype of Tobii's USB service for macOS and Windows.
//!
//! It opens the ET5 over USB and serves it under the name `TOBII-<serial>`
//! (a Unix socket in `/var/run` on macOS, a named pipe on Windows), which is
//! what Tobii's client libraries (Stream Engine, `libtobii_research` in Eye
//! Tracker Manager) discover as `tobii-ttp://<serial>` and connect to. USB
//! behaviour mirrors the Linux service 2.1.5 (`libtobii_libc.so`, reversed
//! statically):
//!
//! * a client's connection mode (first byte on macOS, first word on
//!   Windows): 1 = TTP, 3 = time sync, 4 = firmware upgrade, 6 = SCP (2 and
//!   5 accepted);
//! * mode 1 gets a TTP channel 0..31; the service opens it on the device with
//!   vendor control OUT 65 (wValue = channel) and closes it with 66;
//! * every client write goes to bulk EP 0x05 as `[u32 LE channel][u32 LE
//!   length]` + data, in pieces of at most 0xFF7 bytes;
//! * EP 0x83 carries `[u32 LE channel mask][u32 LE total length incl. these 8
//!   bytes]` + payload, sent raw to every client whose channel bit is set;
//! * mode 3 relays 18-byte time-sync packets through vendor request 69.
//!
//! Firmware upgrade (mode 4) is deliberately not implemented: those clients
//! are accepted and ignored. SCP (mode 6) is answered "not supported".

use rusb::{DeviceHandle, GlobalContext};
use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[path = "transport_unix.rs"]
mod transport;
#[cfg(windows)]
#[path = "transport_windows.rs"]
mod transport;

// The PRP server (Pro SDK 2.0 / PETM 2.7.2) is being built up; not yet wired
// into the client dispatch.
#[allow(dead_code)]
mod prp;
#[allow(dead_code)]
mod ttp;

const VID: u16 = 0x2104;
const PID: u16 = 0x0313;
const IFACE: u8 = 0;
const EP_OUT: u8 = 0x05;
const EP_IN: u8 = 0x83;
const CHANNELS: usize = 32;
/// Largest data piece per bulk OUT write, as the Windows captures and the
/// Spark runtime use (`f7 0f 00 00` in tobii-linux's init_packets).
const OUT_PIECE_MAX: usize = 0xFF7;
const CTRL_TIMEOUT: Duration = Duration::from_millis(5000);

const REQ_SCP: u8 = 48;
const REQ_CHANNEL_OPEN: u8 = 65;
const REQ_CHANNEL_CLOSE: u8 = 66;
const REQ_TIMESYNC: u8 = 69;

type Handle = Arc<DeviceHandle<GlobalContext>>;
type Writer = Box<dyn Write + Send>;
type Slots = Arc<Mutex<Vec<Option<Writer>>>>;

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

macro_rules! log {
    ($($arg:tt)*) => {{
        let t = $crate::START.get_or_init(std::time::Instant::now).elapsed();
        eprintln!("[{:>8.3}] {}", t.as_secs_f64(), format!($($arg)*));
    }};
}

/// A client after its handshake: its connection mode, what it sends and
/// where to answer it.
pub struct Conn {
    pub mode: u32,
    pub reader: Box<dyn Read + Send>,
    pub writer: Writer,
}

struct Bridge {
    usb: Handle,
    slots: Slots,
    next_channel: Mutex<usize>,
    /// Serialises bulk OUT writes so pieces of two clients never interleave.
    out_lock: Mutex<()>,
}

fn main() {
    START.get_or_init(Instant::now);
    if std::env::args().nth(1).as_deref() == Some("ttp-probe") {
        ttp_probe();
        return;
    }
    if std::env::args().any(|a| a == "--prp") {
        prp_server();
        return;
    }
    let mut dir = None;
    let mut name_override = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = Some(args.next().expect("--dir needs a path")),
            "--name" => name_override = Some(args.next().expect("--name needs a value")),
            _ => {
                eprintln!("usage: ttpbridge [--dir <socket dir, macOS only>] [--name <serial>]");
                std::process::exit(2);
            }
        }
    }

    let usb = match open_device() {
        Ok(h) => Arc::new(h),
        Err(e) => {
            log!("cannot open {VID:04x}:{PID:04x}: {e}");
            std::process::exit(1);
        }
    };
    let serial = name_override.unwrap_or_else(|| serial_number(&usb).unwrap_or_else(|| "UNKNOWN".into()));
    log!("device serial {serial}");
    scp_metadata(&usb);

    let mut listener = transport::Listener::bind(&format!("TOBII-{serial}"), dir.as_deref())
        .unwrap_or_else(|e| {
            log!("cannot create the TOBII-{serial} endpoint: {e}");
            std::process::exit(1);
        });
    log!("listening on {} (clients see tobii-ttp://{serial})", listener.describe());

    let bridge = Arc::new(Bridge {
        usb: usb.clone(),
        slots: Arc::new(Mutex::new((0..CHANNELS).map(|_| None).collect())),
        next_channel: Mutex::new(0),
        out_lock: Mutex::new(()),
    });

    {
        let usb = usb.clone();
        let slots = bridge.slots.clone();
        thread::spawn(move || usb_read_loop(&usb, &slots));
    }

    for n in 0.. {
        match listener.accept() {
            Ok(pending) => {
                let b = bridge.clone();
                thread::spawn(move || match pending.handshake() {
                    Ok(conn) => handle_client(&b, n, conn),
                    Err(e) => log!("client {n}: handshake failed ({e})"),
                });
            }
            Err(e) => {
                log!("accept failed: {e}");
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// `ttpbridge --prp`: serve the tracker to Pro SDK 2.0 / Eye Tracker Manager
/// 2.7.2 by translating PRP to TTP. Windows-only for now (macOS PRP transport
/// is not reversed); the device-facing TTP half runs on any platform.
fn prp_server() {
    let usb = match open_device() {
        Ok(h) => Arc::new(h),
        Err(e) => {
            log!("cannot open {VID:04x}:{PID:04x}: {e}");
            std::process::exit(1);
        }
    };
    let serial = serial_number(&usb).unwrap_or_else(|| "UNKNOWN".into());
    log!("device serial {serial}");

    let session = match ttp::session::Session::open(usb, &ttp::session::builtin_init_packets()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log!("TTP session open failed: {e}");
            std::process::exit(1);
        }
    };
    let props = Arc::new(prp::run::Properties::read_from(&session));
    log!("device ready: '{}', {} Hz", props.device_name, props.output_frequency);

    let listener = match prp::transport::Listener::bind(&serial) {
        Ok(l) => l,
        Err(e) => {
            log!("cannot serve TOBIIPRP-{serial}: {e}");
            std::process::exit(1);
        }
    };
    log!("listening on {} (clients see tobii-prp://{serial})", listener.describe());

    if let Err(e) = prp::run::run_server(session, listener, props) {
        log!("prp server stopped: {e}");
        std::process::exit(1);
    }
}

/// `ttpbridge ttp-probe`: bring the device up over TTP ourselves (no SDK) and
/// issue a few read commands, printing what comes back. The local bench for
/// the PRP translator's device-facing half.
fn ttp_probe() {
    let usb = match open_device() {
        Ok(h) => Arc::new(h),
        Err(e) => {
            log!("cannot open {VID:04x}:{PID:04x}: {e}");
            std::process::exit(1);
        }
    };
    log!("serial {}", serial_number(&usb).unwrap_or_default());
    let session = match ttp::session::Session::open(usb, &ttp::session::builtin_init_packets()) {
        Ok(s) => s,
        Err(e) => {
            log!("session open failed: {e}");
            std::process::exit(1);
        }
    };
    log!("session up; init replayed");

    use ttp::protocol::{cmd, parse_message};
    let streams = session.take_streams();
    for (name, command, payload) in [
        ("DEVICE_STRINGS 1420", cmd::DEVICE_STRINGS, vec![0u8, 0]),
        ("DISPLAY_AREA_GET 1430", cmd::DISPLAY_AREA_GET, vec![0u8, 0]),
        ("TRACK_BOX 1400", cmd::TRACK_BOX, vec![0u8, 0]),
        ("OUTPUT_RATE 1650", cmd::OUTPUT_RATE, vec![0u8, 0]),
        ("STATUS 1490", cmd::STATUS, vec![0u8, 0]),
    ] {
        match session.command(command, &payload) {
            Ok(bytes) => {
                let m = parse_message(&bytes);
                let (err, plen) = m.map(|m| (m.error, m.payload.len())).unwrap_or((0, 0));
                let body = m.map(|m| m.payload).unwrap_or(&[]);
                let strings: Vec<String> = body
                    .split(|&b| b == 0)
                    .filter(|s| s.len() >= 3 && s.iter().all(|&c| (0x20..0x7f).contains(&c)))
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect();
                log!("{name}: error {err:#x}, {plen} B payload{}", if strings.is_empty() { String::new() } else { format!(", strings {strings:?}") });
            }
            Err(e) => log!("{name}: {e}"),
        }
    }
    // Start the gaze stream and decode one frame, to confirm the live
    // TTP->values path the PRP translator relies on.
    let _ = session.start_stream(ttp::protocol::STREAM_ID_GAZE);
    thread::sleep(Duration::from_millis(400));
    let mut count = 0;
    let mut shown = false;
    if let Some(rx) = streams {
        while let Ok(msg) = rx.try_recv() {
            count += 1;
            if shown {
                continue;
            }
            if let Some(m) = parse_message(&msg) {
                if let Some(g) = ttp::gaze83::decode_gaze_frame(&m) {
                    let l = &g.left;
                    log!(
                        "gaze t={}us L point valid={} display=({:.3},{:.3}) pupil={:.2}mm guide valid={} ({:.3},{:.3},{:.3})",
                        g.device_ts_us, l.gaze_point_norm.valid, l.gaze_point_norm.value[0], l.gaze_point_norm.value[1],
                        l.pupil_diameter_mm.value, l.position_guide.valid,
                        l.position_guide.value[0], l.position_guide.value[1], l.position_guide.value[2],
                    );
                    shown = true;
                }
            }
        }
    }
    log!("streams/notifications received: {count}");
}

fn open_device() -> rusb::Result<DeviceHandle<GlobalContext>> {
    let h = rusb::open_device_with_vid_pid(VID, PID).ok_or(rusb::Error::NoDevice)?;
    let _ = h.set_auto_detach_kernel_driver(true);
    let _ = h.set_active_configuration(1);
    h.claim_interface(IFACE)?;
    Ok(h)
}

fn serial_number(h: &DeviceHandle<GlobalContext>) -> Option<String> {
    let desc = h.device().device_descriptor().ok()?;
    h.read_serial_number_string_ascii(&desc).ok()
}

/// Query the device's SCP metadata (request 48), as the services do at open,
/// and log it: it names the platform and firmware.
fn scp_metadata(h: &DeviceHandle<GlobalContext>) {
    let req: [u8; 24] = [1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut buf = [0u8; 512];
    let res = h
        .write_control(0x41, REQ_SCP, 0, 0, &req, CTRL_TIMEOUT)
        .and_then(|_| h.read_control(0xC1, REQ_SCP, 0, 0, &mut buf, CTRL_TIMEOUT));
    match res {
        Ok(n) if n > 24 => {
            // The 24-byte header ends with the text length; the rest of the
            // 512-byte reply is stale.
            let len = u32::from_le_bytes(buf[20..24].try_into().unwrap()) as usize;
            let end = (24 + len).min(n);
            let text = String::from_utf8_lossy(&buf[24..end]).replace(char::from(0), "");
            for line in text.lines().filter(|l| !l.is_empty()) {
                log!("scp: {line}");
            }
        }
        Ok(n) => log!("scp: short reply ({n} bytes)"),
        Err(e) => log!("scp query failed: {e}"),
    }
}

/// Read EP 0x83 forever, reassemble `[mask][total]` frames and deliver each
/// payload to the clients of the channels set in its mask.
fn usb_read_loop(usb: &Handle, slots: &Slots) {
    let mut buf = vec![0u8; 16 * 1024];
    let mut pending: Vec<u8> = Vec::with_capacity(128 * 1024);
    loop {
        match usb.read_bulk(EP_IN, &mut buf, Duration::from_millis(1000)) {
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(rusb::Error::Timeout) => continue,
            Err(e) => {
                log!("usb read failed: {e}; stopping");
                std::process::exit(1);
            }
        }
        while pending.len() >= 8 {
            let mask = u32::from_le_bytes(pending[0..4].try_into().unwrap());
            let total = u32::from_le_bytes(pending[4..8].try_into().unwrap()) as usize;
            if total < 8 || total > 512 * 1024 {
                log!("usb: bad frame header mask={mask:#x} total={total}; dropping {} bytes", pending.len());
                pending.clear();
                break;
            }
            if pending.len() < total {
                break;
            }
            deliver(slots, mask, &pending[8..total]);
            pending.drain(..total);
        }
    }
}

fn deliver(slots: &Slots, mask: u32, payload: &[u8]) {
    let mut slots = slots.lock().unwrap();
    for ch in 0..CHANNELS {
        if mask & (1 << ch) == 0 {
            continue;
        }
        let Some(writer) = slots[ch].as_mut() else { continue };
        if let Err(e) = writer.write_all(payload).and_then(|_| writer.flush()) {
            log!("channel {ch}: client write failed ({e}); dropping it");
            slots[ch] = None;
        }
    }
}

fn handle_client(b: &Bridge, n: usize, conn: Conn) {
    log!("client {n}: connection mode {}", conn.mode);
    match conn.mode {
        1 => ttp_client(b, n, conn),
        3 => timesync_client(b, n, conn),
        6 => scp_client(n, conn),
        2 | 4 | 5 => drain_client(n, conn),
        m => log!("client {n}: invalid mode {m}, closing"),
    }
}

fn ttp_client(b: &Bridge, n: usize, conn: Conn) {
    let Conn { mut reader, writer, .. } = conn;
    let Some(ch) = allocate_channel(b, writer) else {
        log!("client {n}: no free TTP channel");
        return;
    };
    if let Err(e) = b.usb.write_control(0x41, REQ_CHANNEL_OPEN, ch as u16, 0, &[], CTRL_TIMEOUT) {
        log!("client {n}: channel {ch} open failed: {e}");
        b.slots.lock().unwrap()[ch] = None;
        return;
    }
    log!("client {n}: TTP channel {ch} open");

    let mut buf = vec![0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(len) => {
                if let Err(e) = send_to_device(b, ch, &buf[..len]) {
                    log!("client {n}: usb write failed: {e}");
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                log!("client {n}: read failed: {e}");
                break;
            }
        }
    }
    b.slots.lock().unwrap()[ch] = None;
    let res = b.usb.write_control(0x41, REQ_CHANNEL_CLOSE, ch as u16, 0, &[], CTRL_TIMEOUT);
    log!("client {n}: TTP channel {ch} closed ({res:?})");
}

/// Next free channel, round robin as the Linux service does.
fn allocate_channel(b: &Bridge, writer: Writer) -> Option<usize> {
    let mut slots = b.slots.lock().unwrap();
    let mut next = b.next_channel.lock().unwrap();
    for i in 0..CHANNELS {
        let ch = (*next + i) % CHANNELS;
        if slots[ch].is_none() {
            slots[ch] = Some(writer);
            *next = (ch + 1) % CHANNELS;
            return Some(ch);
        }
    }
    None
}

fn send_to_device(b: &Bridge, ch: usize, data: &[u8]) -> rusb::Result<()> {
    let _guard = b.out_lock.lock().unwrap();
    for piece in data.chunks(OUT_PIECE_MAX) {
        let mut w = Vec::with_capacity(8 + piece.len());
        w.extend_from_slice(&(ch as u32).to_le_bytes());
        w.extend_from_slice(&(piece.len() as u32).to_le_bytes());
        w.extend_from_slice(piece);
        let mut off = 0;
        while off < w.len() {
            off += b.usb.write_bulk(EP_OUT, &w[off..], CTRL_TIMEOUT)?;
        }
    }
    Ok(())
}

/// Mode 3: each 18-byte request goes to the device through vendor request 69
/// (22 bytes: u32 client slot + the 18 bytes), and the 18 bytes after the
/// slot in the device's answer go back to the client.
fn timesync_client(b: &Bridge, n: usize, conn: Conn) {
    let Conn { mut reader, mut writer, .. } = conn;
    let mut req = [0u8; 18];
    while reader.read_exact(&mut req).is_ok() {
        let mut pkt = [0u8; 22];
        pkt[0..4].copy_from_slice(&(n as u32).to_le_bytes());
        pkt[4..22].copy_from_slice(&req);
        let res = b
            .usb
            .write_control(0x41, REQ_TIMESYNC, 1, 0, &pkt, CTRL_TIMEOUT)
            .and_then(|_| b.usb.read_control(0xC1, REQ_TIMESYNC, 1, 0, &mut pkt, CTRL_TIMEOUT));
        match res {
            Ok(22) => {
                if writer.write_all(&pkt[4..22]).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
            Ok(len) => {
                log!("client {n}: time sync short reply ({len} bytes)");
                break;
            }
            Err(e) => {
                log!("client {n}: time sync failed: {e}");
                break;
            }
        }
    }
    log!("client {n}: time sync client gone");
}

/// Mode 6: answer every 24-byte SCP request "not supported", as the Linux
/// service does for requests it does not serve itself.
fn scp_client(n: usize, conn: Conn) {
    let Conn { mut reader, mut writer, .. } = conn;
    let mut req = [0u8; 24];
    while reader.read_exact(&mut req).is_ok() {
        log!("client {n}: SCP request {:02x?}, answering not supported", &req);
        let mut rsp = [0u8; 24];
        rsp[0..4].copy_from_slice(&1u32.to_le_bytes());
        rsp[4..8].copy_from_slice(&1u32.to_le_bytes());
        rsp[16..20].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        if writer.write_all(&rsp).and_then(|_| writer.flush()).is_err() {
            break;
        }
    }
}

fn drain_client(n: usize, conn: Conn) {
    let Conn { mut reader, .. } = conn;
    let mut buf = [0u8; 4096];
    let mut total = 0usize;
    while let Ok(len) = reader.read(&mut buf) {
        if len == 0 {
            break;
        }
        total += len;
    }
    log!("client {n}: ignored mode client closed after {total} bytes");
}
