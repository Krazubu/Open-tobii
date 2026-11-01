//! Windows named-pipe transport for the PRP server.
//!
//! The server serves `\\.\pipe\TOBIIPRP-<serial>`. A client opens its command
//! connection there; the server mints two `prp-internal://<uuid>` endpoints,
//! serves them as `\\.\pipe\<uuid>`, and names them in the command
//! acknowledgement (see [`super::server`]). The client then opens its
//! subscription and time sync connections to those. Each connection uses one
//! pipe per direction, as Tobii's transport does: the client writes to our
//! pipe and reads from a pipe of its own, whose name it sends in its first
//! message.

use super::server::{self, ChannelKind};
use super::wire::{self, Message};
use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const PIPE_BUFFER: u32 = 0x80000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Pipe(HANDLE);
unsafe impl Send for Pipe {}
impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn security_attrs() -> (SECURITY_ATTRIBUTES, PSECURITY_DESCRIPTOR) {
    // Anyone may use the pipe: the bridge may run elevated, PETM as the user.
    let sddl = wide("D:(A;;GA;;;WD)(A;;GA;;;SY)(A;;GA;;;BA)");
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut());
    }
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd as *mut _,
        bInheritHandle: 0,
    };
    (attrs, sd)
}

fn inbound_instance(name: &str) -> io::Result<Pipe> {
    let (attrs, _sd) = security_attrs();
    let path = wide(&format!(r"\\.\pipe\{name}"));
    let h = unsafe {
        CreateNamedPipeW(
            path.as_ptr(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            0,
            PIPE_BUFFER,
            0,
            &attrs,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(Pipe(h))
}

fn connect(p: &Pipe) -> io::Result<()> {
    let ok = unsafe { ConnectNamedPipe(p.0, null_mut()) };
    if ok == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn open_for_write(name: &str) -> io::Result<Pipe> {
    let path = wide(&format!(r"\\.\pipe\{name}"));
    for _ in 0..5 {
        let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, null(), OPEN_EXISTING, 0, null_mut()) };
        if h != INVALID_HANDLE_VALUE {
            return Ok(Pipe(h));
        }
        if unsafe { GetLastError() } != ERROR_PIPE_BUSY {
            break;
        }
        unsafe { WaitNamedPipeW(path.as_ptr(), 1000) };
    }
    Err(io::Error::last_os_error())
}

/// Read one whole pipe message (handling `ERROR_MORE_DATA`).
fn read_message(p: &Pipe) -> io::Result<Vec<u8>> {
    let mut msg = Vec::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let mut n = 0u32;
        let ok = unsafe { ReadFile(p.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) };
        msg.extend_from_slice(&buf[..n as usize]);
        if ok != 0 {
            return Ok(msg);
        }
        if unsafe { GetLastError() } != ERROR_MORE_DATA {
            return Err(io::Error::last_os_error());
        }
    }
}

fn write_all(p: &Pipe, data: &[u8]) -> io::Result<()> {
    let mut n = 0u32;
    if unsafe { WriteFile(p.0, data.as_ptr(), data.len() as u32, &mut n, null_mut()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// One PRP connection: read frames from the server pipe, write frames to the
/// client's pipe. Frames may span several pipe messages, so reads accumulate
/// until a whole frame parses.
pub struct Channel {
    server: Pipe,
    client: Pipe,
    inbox: Vec<u8>,
}

impl Channel {
    /// Read one PRP message, blocking until a whole frame arrives.
    pub fn read_frame(&mut self) -> io::Result<Message> {
        loop {
            if let Some((msg, used)) = wire::parse_frame(&self.inbox) {
                self.inbox.drain(..used);
                return Ok(msg);
            }
            let chunk = read_message(&self.server)?;
            if chunk.is_empty() {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            self.inbox.extend_from_slice(&chunk);
        }
    }

    /// Write one PRP message to the client's pipe.
    pub fn write_frame(&self, msg: &Message) -> io::Result<()> {
        write_all(&self.client, &wire::write_frame(msg))
    }
}

/// A PRP client's three connections, tied together by the internal endpoints
/// the command connection minted.
pub struct PrpClient {
    pub command: Channel,
    pub subscription: Channel,
    pub timesync: Channel,
    pub pid: u32,
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Mint a fresh internal endpoint name. A UUID-shaped token is enough; the
/// client treats it as opaque.
fn mint_endpoint() -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("tobii-et5-bridge-{pid:08x}-{n:016x}")
}

/// Read the client's first message and its pid on a freshly connected server
/// pipe, returning the kind and the client's own pipe name.
fn read_hello(server: &Pipe) -> io::Result<(ChannelKind, String)> {
    let hello = read_message(server)?;
    server::parse_client_hello(&hello)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad PRP hello"))
}

fn read_pid(server: &Pipe) -> io::Result<u32> {
    let m = read_message(server)?;
    if m.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short pid"));
    }
    Ok(u32::from_le_bytes(m[0..4].try_into().unwrap()))
}

/// The PRP listener: serves `TOBIIPRP-<serial>` and accepts whole clients.
pub struct Listener {
    name: String,
}

impl Listener {
    pub fn bind(serial: &str) -> io::Result<Self> {
        let name = format!("TOBIIPRP-{serial}");
        // Fail now if the name is taken.
        drop(inbound_instance(&name)?);
        Ok(Listener { name })
    }

    pub fn describe(&self) -> String {
        format!(r"\\.\pipe\{}", self.name)
    }

    /// Accept one command connection and complete the whole client handshake:
    /// mint the two internal endpoints, serve them, and wait for the client's
    /// subscription and time sync connections. Returns the three channels.
    pub fn accept(&self) -> io::Result<PrpClient> {
        // Command connection on the main pipe.
        let cmd_server = inbound_instance(&self.name)?;
        connect(&cmd_server)?;
        let (kind, cmd_client_name) = read_hello(&cmd_server)?;
        if kind != ChannelKind::Command {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "first PRP connection is not the command channel"));
        }

        // Mint and pre-create the two internal pipes before naming them, so
        // the client can connect the instant it reads the acknowledgement.
        let sub_name = mint_endpoint();
        let ts_name = mint_endpoint();
        let sub_server = inbound_instance(&sub_name)?;
        let ts_server = inbound_instance(&ts_name)?;

        let cmd_client = open_for_write(&cmd_client_name)?;
        write_all(&cmd_client, &server::command_ack(&sub_name, &ts_name))?;
        let pid = read_pid(&cmd_server)?;

        // Subscription connection.
        connect(&sub_server)?;
        let (k, sub_client_name) = read_hello(&sub_server)?;
        if k != ChannelKind::Subscription {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected the subscription channel"));
        }
        let sub_client = open_for_write(&sub_client_name)?;
        write_all(&sub_client, &server::subscription_ack())?;
        let _ = read_pid(&sub_server)?;

        // Time sync connection.
        connect(&ts_server)?;
        let (k, ts_client_name) = read_hello(&ts_server)?;
        if k != ChannelKind::Timesync {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "expected the time sync channel"));
        }
        let ts_client = open_for_write(&ts_client_name)?;
        write_all(&ts_client, &server::timesync_ack())?;
        let _ = read_pid(&ts_server)?;

        Ok(PrpClient {
            command: Channel { server: cmd_server, client: cmd_client, inbox: Vec::new() },
            subscription: Channel { server: sub_server, client: sub_client, inbox: Vec::new() },
            timesync: Channel { server: ts_server, client: ts_client, inbox: Vec::new() },
            pid,
        })
    }
}
