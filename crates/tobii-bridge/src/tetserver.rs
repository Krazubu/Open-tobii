//! The tracker as a network "tetserver", the way Tobii's older trackers
//! (X60/X120, T60/T120, X2-60, TX300…) present themselves to Tobii SDK 3
//! (`TobiiSDK.framework`, the Analytics SDK's `libtetio`, Eye Tracker
//! Browser, `tobiictl`).
//!
//! Those trackers speak TTP over TCP on port 4455, with the same 24-byte
//! header (marker, seq, status, id, error, length) and TLV payloads as the
//! Eye Tracker 5 and Pro Spark speak over USB, and are found through Bonjour:
//! a `_tetserver._tcp` service named `tetserver<product id>` whose TXT record
//! carries `pid`, `model`, `name`, `version`, `generation` and
//! `platformtype`. SDK 3 takes the product id from the service name (after
//! its 9-character `tetserver` prefix).
//!
//! A tracking session from SDK 3 is the protocol negotiation (command 1000)
//! followed by a stream start (1220) of the gaze stream 0x500, whose 0x53
//! messages SDK 3 decodes by column: the Spark's carry the same columns as an
//! X2-60's, plus more that SDK 3 skips. So each TCP connection is served as
//! one more TTP channel on the device, exactly like a mode-1 client of the
//! local socket, with nothing translated.

use crate::{Bridge, Conn, handle_client};
use std::io::{self, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

/// Port Tobii's network trackers serve TTP on.
pub const DEFAULT_PORT: u16 = 4455;

/// Accept SDK 3 clients on `port` (all interfaces, as a network tracker
/// does: SDK 3 connects to the address Bonjour resolves) and serve each as a
/// TTP channel. Returns once the socket is bound; clients are served on
/// their own threads. IPv4 only: SDK 3 knows nothing else.
pub fn serve(bridge: Arc<Bridge>, port: u16, first_client: usize) -> io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    thread::spawn(move || {
        let mut n = first_client;
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    log!("tetserver: accept failed: {e}");
                    continue;
                }
            };
            let peer = stream
                .peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_default();
            // Gaze is a 60 Hz trickle of small messages: do not batch them.
            let _ = stream.set_nodelay(true);
            let writer = match stream.try_clone() {
                Ok(w) => w,
                Err(e) => {
                    log!("tetserver: {peer}: {e}");
                    continue;
                }
            };
            let b = bridge.clone();
            let id = n;
            n = n.wrapping_add(1);
            log!("client {id}: SDK 3 connection from {peer}");
            thread::spawn(move || {
                handle_client(
                    &b,
                    id,
                    Conn {
                        mode: 1,
                        reader: Box::new(stream),
                        writer: Box::new(writer),
                    },
                );
            });
        }
    });
    Ok(())
}

/// Bring the tracker up the way the daemon does: replay the built-in init
/// (display area, power-up default calibration, stream set-up…) on a channel
/// of the bridge's own. Eye Tracker Manager does its own init, but SDK 3 only
/// negotiates and starts the gaze stream, and an uninitialised tracker
/// streams nothing but "eye not found". Returns (commands sent, commands that
/// failed or went unanswered).
pub fn prime(bridge: &Bridge) -> Result<(usize, usize), String> {
    let (tx, rx) = sync_channel::<Vec<u8>>(256);
    let ch = crate::allocate_channel(bridge, Box::new(ChannelWriter(tx)))
        .ok_or("no free TTP channel")?;
    let close = |b: &Bridge| {
        b.slots.lock().unwrap()[ch] = None;
        let _ = b.usb.write_control(
            0x41,
            crate::REQ_CHANNEL_CLOSE,
            ch as u16,
            0,
            &[],
            crate::CTRL_TIMEOUT,
        );
    };
    if let Err(e) = bridge.usb.write_control(
        0x41,
        crate::REQ_CHANNEL_OPEN,
        ch as u16,
        0,
        &[],
        crate::CTRL_TIMEOUT,
    ) {
        bridge.slots.lock().unwrap()[ch] = None;
        return Err(format!("channel {ch} open failed: {e}"));
    }

    let packets = crate::ttp::session::builtin_init_packets();
    let mut replies = Replies {
        rx,
        buf: Vec::new(),
    };
    let (mut sent, mut failed) = (0, 0);
    let mut awaiting: Option<u32> = None;
    for (i, pkt) in packets.iter().enumerate() {
        // `[u32 channel][u32 len][data]`: the bridge frames for its channel.
        let Some(data) = pkt.get(8..) else { continue };
        if let Some(seq) = command_seq(data) {
            awaiting = Some(seq);
        }
        if let Err(e) = crate::send_to_device(bridge, ch, data) {
            close(bridge);
            return Err(format!("usb write failed: {e}"));
        }
        // A command may span several writes; the answer follows the last.
        let complete = packets
            .get(i + 1)
            .and_then(|n| n.get(8..))
            .is_none_or(|n| command_seq(n).is_some());
        if complete && let Some(seq) = awaiting.take() {
            sent += 1;
            match replies.wait(seq, Duration::from_secs(5)) {
                Some(0) => {}
                // The two ET5 licence commands come back UNAUTHORIZED on a
                // Spark; harmless (docs/spark.md).
                Some(_) | None => failed += 1,
            }
        }
    }
    close(bridge);
    Ok((sent, failed))
}

/// Seq of a TTP command message (marker 0x51), if `data` starts one.
fn command_seq(data: &[u8]) -> Option<u32> {
    let word = |i: usize| Some(u32::from_be_bytes(data.get(i..i + 4)?.try_into().ok()?));
    (word(0)? == 0x51 && word(8)? == 0)
        .then(|| word(4))
        .flatten()
}

/// Where the bridge delivers what the device sends on [`prime`]'s channel.
struct ChannelWriter(SyncSender<Vec<u8>>);

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        // Never block the USB reader: drop what nobody waits for.
        let _ = self.0.try_send(data.to_vec());
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Replies {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
}

impl Replies {
    /// The error word of the response (marker 0x52) to command `seq`.
    fn wait(&mut self, seq: u32, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            while self.buf.len() >= 24 {
                let word = |i: usize| u32::from_be_bytes(self.buf[i..i + 4].try_into().unwrap());
                let total = 24 + word(20) as usize;
                if self.buf.len() < total {
                    break;
                }
                let (marker, s, err) = (word(0), word(4), word(16));
                self.buf.drain(..total);
                if marker == 0x52 && s == seq {
                    return Some(err);
                }
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            self.buf.extend(self.rx.recv_timeout(left).ok()?);
        }
    }
}

/// What the Bonjour TXT record says about the tracker.
pub struct Advert {
    pub serial: String,
    pub model: String,
    pub firmware: String,
    pub platform: String,
}

impl Advert {
    /// Model name SDK 3 shows, from the serial's model prefix.
    pub fn model_for(serial: &str) -> &'static str {
        match serial.split('-').next().unwrap_or("") {
            "TPE01" => "Tobii Pro Spark",
            "IS5FF" => "Tobii Eye Tracker 5",
            _ => "Tobii",
        }
    }

    fn txt_record(&self) -> Vec<u8> {
        let entries = [
            ("model", self.model.as_str()),
            ("pid", self.serial.as_str()),
            ("name", self.model.as_str()),
            ("version", self.firmware.as_str()),
            // Both are IS5 platforms; SDK 3 only compares the generation with
            // a firmware upgrade package's, which never applies here.
            ("generation", "IS5"),
            ("platformtype", self.platform.as_str()),
        ];
        let mut txt = Vec::new();
        for (k, v) in entries {
            let entry = format!("{k}={v}");
            let bytes = &entry.as_bytes()[..entry.len().min(255)];
            txt.push(bytes.len() as u8);
            txt.extend_from_slice(bytes);
        }
        txt
    }
}

/// Announce `_tetserver._tcp` through the system's mDNSResponder, on a host
/// name of the bridge's own that has only IPv4 addresses (see
/// [`crate::bonjour`]). The registration lasts as long as the process.
/// Returns the service name and the addresses announced.
#[cfg(target_os = "macos")]
pub fn advertise(advert: &Advert, port: u16) -> Result<(String, Vec<String>), String> {
    let name = format!("tetserver{}", advert.serial);
    let host = format!("tetserver-{}", advert.serial);
    let addresses =
        crate::bonjour::register(&name, "_tetserver._tcp", &host, port, &advert.txt_record())?;
    Ok((name, addresses))
}

/// Elsewhere there is no system Bonjour API to call: SDK 3 clients can still
/// connect by address (`tobiictl -t <host>`).
#[cfg(not(target_os = "macos"))]
pub fn advertise(_advert: &Advert, _port: u16) -> Result<(String, Vec<String>), String> {
    Err("Bonjour advertising is only implemented on macOS".into())
}
