//! Prototype of Tobii's USB service (`tobiiusbserviced`) for macOS.
//!
//! It opens the ET5 over USB and serves it on a Unix socket named
//! `TOBII-<serial>`, which is what Tobii's client libraries (Stream Engine,
//! `libtobii_research` in Eye Tracker Manager) discover as
//! `tobii-ttp://<serial>` and connect to. Behaviour mirrors the Linux service
//! 2.1.5 (`libtobii_libc.so`, reversed statically):
//!
//! * a client's first message is one byte, its connection mode:
//!   1 = TTP, 3 = time sync, 4 = firmware upgrade, 6 = SCP (2 and 5 accepted);
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
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
type Slots = Arc<Mutex<Vec<Option<UnixStream>>>>;

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

macro_rules! log {
    ($($arg:tt)*) => {{
        let t = START.get_or_init(Instant::now).elapsed();
        eprintln!("[{:>8.3}] {}", t.as_secs_f64(), format!($($arg)*));
    }};
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
    let mut dir = PathBuf::from("/var/run");
    let mut name_override = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = PathBuf::from(args.next().expect("--dir needs a path")),
            "--name" => name_override = Some(args.next().expect("--name needs a value")),
            _ => {
                eprintln!("usage: ttpbridge [--dir /var/run] [--name <serial>]");
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

    let path = dir.join(format!("TOBII-{serial}"));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap_or_else(|e| {
        log!("cannot bind {}: {e}", path.display());
        std::process::exit(1);
    });
    // The service runs as root; clients such as Eye Tracker Manager do not.
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    log!("listening on {} (clients see tobii-ttp://{serial})", path.display());

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

    for (n, conn) in listener.incoming().enumerate() {
        match conn {
            Ok(stream) => {
                let b = bridge.clone();
                thread::spawn(move || handle_client(&b, n, stream));
            }
            Err(e) => log!("accept failed: {e}"),
        }
    }
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
        let Some(stream) = slots[ch].as_mut() else { continue };
        if let Err(e) = stream.write_all(payload) {
            log!("channel {ch}: client write failed ({e}); dropping it");
            slots[ch] = None;
        }
    }
}

fn handle_client(b: &Bridge, n: usize, mut stream: UnixStream) {
    let mut mode = [0u8; 1];
    if let Err(e) = stream.read_exact(&mut mode) {
        log!("client {n}: no mode byte ({e})");
        return;
    }
    log!("client {n}: connection mode {}", mode[0]);
    match mode[0] {
        1 => ttp_client(b, n, stream),
        3 => timesync_client(b, n, stream),
        6 => scp_client(n, stream),
        2 | 4 | 5 => drain_client(n, stream),
        m => log!("client {n}: invalid mode {m}, closing"),
    }
}

fn ttp_client(b: &Bridge, n: usize, mut stream: UnixStream) {
    let Some(ch) = allocate_channel(b, &stream) else {
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
        match stream.read(&mut buf) {
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
fn allocate_channel(b: &Bridge, stream: &UnixStream) -> Option<usize> {
    let writer = stream.try_clone().ok()?;
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
fn timesync_client(b: &Bridge, n: usize, mut stream: UnixStream) {
    let mut req = [0u8; 18];
    while stream.read_exact(&mut req).is_ok() {
        let mut pkt = [0u8; 22];
        pkt[0..4].copy_from_slice(&(n as u32).to_le_bytes());
        pkt[4..22].copy_from_slice(&req);
        let res = b
            .usb
            .write_control(0x41, REQ_TIMESYNC, 1, 0, &pkt, CTRL_TIMEOUT)
            .and_then(|_| b.usb.read_control(0xC1, REQ_TIMESYNC, 1, 0, &mut pkt, CTRL_TIMEOUT));
        match res {
            Ok(22) => {
                if stream.write_all(&pkt[4..22]).is_err() {
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
fn scp_client(n: usize, mut stream: UnixStream) {
    let mut req = [0u8; 24];
    while stream.read_exact(&mut req).is_ok() {
        log!("client {n}: SCP request {:02x?}, answering not supported", &req);
        let mut rsp = [0u8; 24];
        rsp[0..4].copy_from_slice(&1u32.to_le_bytes());
        rsp[4..8].copy_from_slice(&1u32.to_le_bytes());
        rsp[16..20].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
        if stream.write_all(&rsp).is_err() {
            break;
        }
    }
}

fn drain_client(n: usize, mut stream: UnixStream) {
    let mut buf = [0u8; 4096];
    let mut total = 0usize;
    while let Ok(len) = stream.read(&mut buf) {
        if len == 0 {
            break;
        }
        total += len;
    }
    log!("client {n}: ignored mode client closed after {total} bytes");
}
