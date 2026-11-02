//! A TTP session with the Eye Tracker 5 over USB: open channel 0, replay the
//! init sequence, send commands and await their responses by sequence number,
//! and route the device's stream and notification messages to a handler.
//!
//! The TTP relay never needed this — the SDK drove the device. The PRP
//! translator does: it is the only thing talking to the tracker, so it brings
//! the device up and issues the commands itself. The USB framing matches
//! tobii-linux (and the bridge's own relay path): commands go to bulk EP 0x05
//! as `chunk_command` pieces, and EP 0x83 is reassembled into whole messages
//! by [`super::protocol::BulkReassembler`].

use super::protocol::{self, BulkReassembler, MARKER_NOTIFICATION, MARKER_RESPONSE, MARKER_STREAM};
use rusb::{DeviceHandle, GlobalContext};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const EP_OUT: u8 = 0x05;
const EP_IN: u8 = 0x83;
const CTRL_TIMEOUT: Duration = Duration::from_millis(5000);
// Generous: USB in a VM is slow (device bring-up takes tens of seconds), and a
// command whose response lags must not be given up on.
const CMD_TIMEOUT: Duration = Duration::from_millis(12000);

type Usb = Arc<DeviceHandle<GlobalContext>>;

/// A message the device sent that is not a command response: a stream (0x53)
/// or a notification (0x4e). The bytes include the 8-byte prefix, so
/// [`protocol::parse_message`] reads them.
pub type DeviceMessage = Vec<u8>;

struct Shared {
    /// Command responses awaited by sequence number.
    waiters: Mutex<HashMap<u32, SyncSender<DeviceMessage>>>,
    /// Stream and notification messages, for a consumer to receive. Dropped
    /// when the channel is full so a slow consumer never stalls the USB reader.
    streams: SyncSender<DeviceMessage>,
}

/// An open TTP session on channel 0.
pub struct Session {
    usb: Usb,
    seq: AtomicU32,
    shared: Arc<Shared>,
    stream_rx: Mutex<Option<Receiver<DeviceMessage>>>,
}

impl Session {
    /// Open channel 0 on an already-claimed device handle, start the reader,
    /// and replay the init sequence from `init_packets` (the lines of
    /// `assets/ttp_init_packets.txt`, already in bulk-OUT framing).
    pub fn open(usb: Usb, init_packets: &[Vec<u8>]) -> rusb::Result<Self> {
        vendor_control_init(&usb)?;

        let (stream_tx, stream_rx) = sync_channel(1024);
        let shared = Arc::new(Shared {
            waiters: Mutex::new(HashMap::new()),
            streams: stream_tx,
        });
        {
            let usb = usb.clone();
            let shared = shared.clone();
            thread::Builder::new()
                .name("ttp-reader".into())
                .spawn(move || reader_loop(&usb, &shared))
                .expect("spawn ttp reader");
        }

        let session = Session { usb, seq: AtomicU32::new(1), shared, stream_rx: Mutex::new(Some(stream_rx)) };
        session.replay(init_packets)?;
        Ok(session)
    }

    /// Take the receiver of stream and notification messages (once). The
    /// caller owns it and reads device streams from it.
    pub fn take_streams(&self) -> Option<Receiver<DeviceMessage>> {
        self.stream_rx.lock().unwrap().take()
    }

    /// Start a device stream (command 1220), waiting for its response so the
    /// device actually begins streaming (a fire-and-forget send did not take).
    pub fn start_stream(&self, id: u32) -> rusb::Result<()> {
        self.command(protocol::cmd::STREAM_START, &protocol::stream_start_payload(id)).map(|_| ())
    }

    /// Stop a device stream (command 1230), fire and forget.
    pub fn stop_stream(&self, id: u32) -> rusb::Result<()> {
        let seq = self.next_seq();
        self.write_command(protocol::cmd::STREAM_STOP, seq, &protocol::stream_id_payload(id))
    }

    /// The next command sequence number.
    fn next_seq(&self) -> u32 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a TTP command and wait for the matching `0x52` response. Returns
    /// the whole response bytes (prefix included); parse with
    /// [`protocol::parse_message`].
    pub fn command(&self, cmd: u32, payload: &[u8]) -> rusb::Result<DeviceMessage> {
        let seq = self.next_seq();
        let (tx, rx) = sync_channel(1);
        self.shared.waiters.lock().unwrap().insert(seq, tx);
        if let Err(e) = self.write_command(cmd, seq, payload) {
            self.shared.waiters.lock().unwrap().remove(&seq);
            return Err(e);
        }
        match rx.recv_timeout(CMD_TIMEOUT) {
            Ok(bytes) => Ok(bytes),
            Err(_) => {
                self.shared.waiters.lock().unwrap().remove(&seq);
                Err(rusb::Error::Timeout)
            }
        }
    }

    /// Send a command without waiting for its response (used by the init
    /// replay for the non-command packets and by fire-and-forget commands).
    fn write_command(&self, cmd: u32, seq: u32, payload: &[u8]) -> rusb::Result<()> {
        for piece in protocol::chunk_command(cmd, seq, payload) {
            self.write_bulk_all(&piece)?;
        }
        Ok(())
    }

    /// Write one already-framed bulk-OUT packet (channel + length + data).
    fn write_raw(&self, packet: &[u8]) -> rusb::Result<()> {
        self.write_bulk_all(packet)
    }

    fn write_bulk_all(&self, data: &[u8]) -> rusb::Result<()> {
        let mut off = 0;
        while off < data.len() {
            off += self.usb.write_bulk(EP_OUT, &data[off..], CTRL_TIMEOUT)?;
        }
        Ok(())
    }

    /// Replay the init sequence. Each packet is already in bulk-OUT framing; a
    /// `0x51` command packet is followed by waiting for its response so the
    /// device stays in step, as the Windows engine does.
    fn replay(&self, packets: &[Vec<u8>]) -> rusb::Result<()> {
        for pkt in packets {
            // A packet is `[channel u32][len u32][ttp message]`; the message
            // starts at offset 8, its marker is a BE u32 there.
            let seq = command_seq_of(pkt);
            if let Some(seq) = seq {
                let (tx, rx) = sync_channel(1);
                self.shared.waiters.lock().unwrap().insert(seq, tx);
                self.write_raw(pkt)?;
                if rx.recv_timeout(CMD_TIMEOUT).is_err() {
                    self.shared.waiters.lock().unwrap().remove(&seq);
                    // The device sometimes answers an init command with
                    // nothing; keep going rather than abort the whole bring-up.
                }
                // Keep seq counter ahead of the init packets' own numbers.
                self.seq.fetch_max(seq + 1, Ordering::SeqCst);
            } else {
                self.write_raw(pkt)?;
            }
        }
        Ok(())
    }

}

/// The sequence number a bulk-OUT packet's command expects echoed, or `None`
/// if it is not a `0x51` command. The TTP message starts at offset 8 (after
/// the `[channel][len]` framing), marker and seq are BE u32 there.
fn command_seq_of(pkt: &[u8]) -> Option<u32> {
    let msg = pkt.get(8..)?;
    let marker = u32::from_be_bytes(msg.get(0..4)?.try_into().ok()?);
    if marker != protocol::MARKER_COMMAND {
        return None;
    }
    Some(u32::from_be_bytes(msg.get(4..8)?.try_into().ok()?))
}

/// Control-transfer bring-up, mirroring tobii-linux `vendor_control_init`:
/// request 48 (SCP init) out/in, request 70 in, then request 65 wValue 0 to
/// open channel 0.
fn vendor_control_init(usb: &DeviceHandle<GlobalContext>) -> rusb::Result<()> {
    let init: [u8; 24] = [1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    usb.write_control(0x41, 48, 0, 0, &init, CTRL_TIMEOUT)?;
    let mut buf = [0u8; 512];
    usb.read_control(0xC1, 48, 0, 0, &mut buf, CTRL_TIMEOUT)?;
    let mut buf8 = [0u8; 8];
    usb.read_control(0xC1, 70, 1, 0, &mut buf8, CTRL_TIMEOUT)?;
    usb.write_control(0x41, 65, 0, 0, &[], CTRL_TIMEOUT)?;
    Ok(())
}

/// Read EP 0x83 forever, reassemble whole messages, and route each: command
/// responses (0x52) to their waiter by sequence, streams (0x53) and
/// notifications (0x4e) to the inbox.
fn reader_loop(usb: &DeviceHandle<GlobalContext>, shared: &Shared) {
    let mut reasm = BulkReassembler::new();
    let mut buf = vec![0u8; 128 * 1024];
    let mut done: Vec<DeviceMessage> = Vec::new();
    loop {
        let n = match usb.read_bulk(EP_IN, &mut buf, Duration::from_millis(1000)) {
            Ok(n) => n,
            Err(rusb::Error::Timeout) => continue,
            Err(_) => return,
        };
        reasm.push_into(&buf[..n], &mut done);
        for msg in done.drain(..) {
            route(shared, msg);
        }
    }
}

fn route(shared: &Shared, msg: DeviceMessage) {
    let Some(parsed) = protocol::parse_message(&msg) else { return };
    match parsed.marker {
        MARKER_RESPONSE => {
            if let Some(tx) = shared.waiters.lock().unwrap().remove(&parsed.seq) {
                let _ = tx.send(msg);
            }
        }
        MARKER_STREAM | MARKER_NOTIFICATION => {
            // Drop rather than block the USB reader if the consumer is behind.
            let _ = shared.streams.try_send(msg);
        }
        _ => {}
    }
}

/// Load the init packets shipped in the binary.
pub fn builtin_init_packets() -> Vec<Vec<u8>> {
    const TEXT: &str = include_str!("../../assets/ttp_init_packets.txt");
    protocol::parse_init_packets(TEXT).expect("built-in init packets parse").into_iter().map(|p| p.data).collect()
}
