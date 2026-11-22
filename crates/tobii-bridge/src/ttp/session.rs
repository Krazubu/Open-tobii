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
use rusb::{DeviceHandle, GlobalContext, UsbContext};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
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
    /// and replay the init sequence from `init_packets` (already in bulk-OUT
    /// framing).
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
                .spawn(move || reader_loop(usb, shared))
                .expect("spawn ttp reader");
        }

        let session = Session {
            usb,
            seq: AtomicU32::new(1),
            shared,
            stream_rx: Mutex::new(Some(stream_rx)),
        };
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
        self.command(
            protocol::cmd::STREAM_START,
            &protocol::stream_start_payload(id),
        )
        .map(|_| ())
    }

    /// Stop a device stream (command 1230), fire and forget.
    pub fn stop_stream(&self, id: u32) -> rusb::Result<()> {
        let seq = self.next_seq();
        self.write_command(
            protocol::cmd::STREAM_STOP,
            seq,
            &protocol::stream_id_payload(id),
        )
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
        // A command may be split over continuation writes (the calibration
        // upload, command 1110, spans ~162 bulk writes). The device answers
        // only after the command's LAST piece, so the response wait must happen
        // there, not after the first piece. A command is complete when the next
        // packet starts a new command (or there is none); until then we keep
        // writing its pieces while the async reader drains EP 0x83 (the device
        // notifies after the first calibration piece and won't accept more
        // until the host has read it — the reader does that continuously).
        let mut awaiting: Option<(u32, u32, std::time::Instant, Receiver<DeviceMessage>)> = None;
        for (i, pkt) in packets.iter().enumerate() {
            // A packet is `[channel u32][len u32][ttp message]`; the message
            // starts at offset 8, its marker is a BE u32 there, and the command
            // id is the BE u32 at message offset 12 (marker, seq, status, id).
            if let Some(seq) = command_seq_of(pkt) {
                let cmd_id = pkt
                    .get(20..24)
                    .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                    .unwrap_or(0);
                let (tx, rx) = sync_channel(1);
                self.shared.waiters.lock().unwrap().insert(seq, tx);
                awaiting = Some((seq, cmd_id, std::time::Instant::now(), rx));
                // Keep seq counter ahead of the init packets' own numbers.
                self.seq.fetch_max(seq + 1, Ordering::SeqCst);
            }
            self.write_raw(pkt)?;
            let complete = packets
                .get(i + 1)
                .is_none_or(|n| command_seq_of(n).is_some());
            if complete && let Some((seq, cmd_id, t0, rx)) = awaiting.take() {
                match rx.recv_timeout(CMD_TIMEOUT) {
                    Ok(bytes) => {
                        let ms = t0.elapsed().as_millis();
                        let err = protocol::parse_message(&bytes).map_or(0, |m| m.error);
                        // A healthy init command answers in milliseconds
                        // with error 0; flag anything slow or failed.
                        if ms > 100 || err != 0 {
                            eprintln!("[init] cmd {cmd_id} seq {seq}: {ms} ms, error {err:#x}");
                        }
                    }
                    Err(_) => {
                        self.shared.waiters.lock().unwrap().remove(&seq);
                        eprintln!(
                            "[init] cmd {cmd_id} seq {seq}: TIMED OUT after {} ms (no response)",
                            t0.elapsed().as_millis()
                        );
                    }
                }
            }
            // Pace the writes as the Windows engine does; the device needs a
            // moment between calibration pieces.
            std::thread::sleep(std::time::Duration::from_millis(2));
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
    let init: [u8; 24] = [
        1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    usb.write_control(0x41, 48, 0, 0, &init, CTRL_TIMEOUT)?;
    let mut buf = [0u8; 512];
    usb.read_control(0xC1, 48, 0, 0, &mut buf, CTRL_TIMEOUT)?;
    let mut buf8 = [0u8; 8];
    usb.read_control(0xC1, 70, 1, 0, &mut buf8, CTRL_TIMEOUT)?;
    usb.write_control(0x41, 65, 0, 0, &[], CTRL_TIMEOUT)?;
    Ok(())
}

/// Number of EP 0x83 read transfers kept in flight, and each one's size. The
/// device (especially over USB/IP) only streams while reads stay queued; a
/// single synchronous read leaves gaps between calls where the pipe stalls, so
/// a ring of outstanding async transfers keeps it drained, as Tobii's own
/// runtime does.
const READ_TRANSFERS: usize = 16;
const READ_BUF_LEN: usize = 64 * 1024;

/// Shared state the transfer callbacks and the event loop use. Touched only on
/// the reader thread (libusb callbacks run inside `libusb_handle_events` on
/// that thread), so no locking is needed within it.
struct ReaderCtx {
    reasm: BulkReassembler,
    done: Vec<DeviceMessage>,
    shared: Arc<Shared>,
    running: bool,
}

extern "system" fn on_transfer(xfer: *mut libusb1_sys::libusb_transfer) {
    use libusb1_sys::constants::*;
    // Safety: `xfer` and its `user_data` live for the reader thread's lifetime.
    let t = unsafe { &mut *xfer };
    let ctx = unsafe { &mut *(t.user_data as *mut ReaderCtx) };
    match t.status {
        LIBUSB_TRANSFER_COMPLETED => {
            let n = t.actual_length as usize;
            let data = unsafe { std::slice::from_raw_parts(t.buffer, n) };
            ctx.reasm.push_into(data, &mut ctx.done);
            for msg in ctx.done.drain(..) {
                route(&ctx.shared, msg);
            }
        }
        LIBUSB_TRANSFER_TIMED_OUT => {}
        LIBUSB_TRANSFER_NO_DEVICE | LIBUSB_TRANSFER_CANCELLED => {
            ctx.running = false;
            return;
        }
        _ => {
            // A transient error: resync the reassembler and keep going.
            ctx.reasm = BulkReassembler::new();
        }
    }
    // Resubmit this transfer so a read is always queued.
    if unsafe { libusb1_sys::libusb_submit_transfer(xfer) } != 0 {
        ctx.running = false;
    }
}

/// Read EP 0x83 with a ring of asynchronous transfers, routing each completed
/// message. Runs until the device is gone.
fn reader_loop(usb: Usb, shared: Arc<Shared>) {
    let ctx = Box::new(ReaderCtx {
        reasm: BulkReassembler::new(),
        done: Vec::new(),
        shared,
        running: true,
    });
    let ctx_ptr = Box::into_raw(ctx);

    let mut buffers: Vec<Vec<u8>> = (0..READ_TRANSFERS)
        .map(|_| vec![0u8; READ_BUF_LEN])
        .collect();
    let mut transfers: Vec<*mut libusb1_sys::libusb_transfer> = Vec::with_capacity(READ_TRANSFERS);
    for buf in buffers.iter_mut() {
        let xfer = unsafe { libusb1_sys::libusb_alloc_transfer(0) };
        if xfer.is_null() {
            break;
        }
        unsafe {
            (*xfer).dev_handle = usb.as_raw();
            (*xfer).endpoint = EP_IN;
            (*xfer).transfer_type = libusb1_sys::constants::LIBUSB_TRANSFER_TYPE_BULK;
            (*xfer).timeout = 0; // no timeout: a queued read waits for data
            (*xfer).buffer = buf.as_mut_ptr();
            (*xfer).length = READ_BUF_LEN as i32;
            (*xfer).callback = on_transfer;
            (*xfer).user_data = ctx_ptr as *mut c_void;
            if libusb1_sys::libusb_submit_transfer(xfer) == 0 {
                transfers.push(xfer);
            } else {
                libusb1_sys::libusb_free_transfer(xfer);
            }
        }
    }

    let context = usb.context().as_raw();
    // `running` is cleared by `on_transfer`, which libusb calls from inside
    // `libusb_handle_events_timeout_completed` below, through `ctx_ptr`.
    #[allow(clippy::while_immutable_condition)]
    while unsafe { (*ctx_ptr).running } {
        let tv = libc::timeval {
            tv_sec: 1,
            tv_usec: 0,
        };
        unsafe {
            libusb1_sys::libusb_handle_events_timeout_completed(context, &tv, std::ptr::null_mut())
        };
    }

    // Best-effort cleanup.
    for xfer in transfers {
        unsafe {
            libusb1_sys::libusb_cancel_transfer(xfer);
            libusb1_sys::libusb_free_transfer(xfer);
        }
    }
    drop(buffers);
    drop(unsafe { Box::from_raw(ctx_ptr) });
}

fn route(shared: &Shared, msg: DeviceMessage) {
    let Some(parsed) = protocol::parse_message(&msg) else {
        return;
    };
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

/// The init replay shipped in the binary: `tobii-usb`'s, the one the daemon
/// replays, which carries the tracker's own power-up default calibration.
pub fn builtin_init_packets() -> Vec<Vec<u8>> {
    tobii_usb::calibration::embedded_packets()
        .expect("built-in init packets parse")
        .into_iter()
        .map(|p| p.data)
        .collect()
}

/// The calibration blob the init replay uploads (its command 1110): the
/// tracker's power-up default. Opening a calibration session seeds the
/// device's sample ring by writing this blob back (1110); the device holds no
/// user calibration to read, so a 1100 read comes back empty.
pub fn embedded_calibration_blob() -> Vec<u8> {
    tobii_usb::calibration::embedded_blob().unwrap_or_default()
}

#[cfg(test)]
mod embedded_blob_tests {
    #[test]
    fn embedded_blob_is_present() {
        let blob = super::embedded_calibration_blob();
        assert!(
            blob.len() > 100,
            "embedded blob should be a real calibration, got {}",
            blob.len()
        );
    }
}
