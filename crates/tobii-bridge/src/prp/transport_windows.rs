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

/// Accept one connection on a freshly made instance of `name`, check its kind,
/// send `ack`, and read the client's pid. Returns the channel.
fn accept_one(name: &str, expect: ChannelKind, ack: &[u8]) -> io::Result<Channel> {
    let server = inbound_instance(name)?;
    connect(&server)?;
    let hello = read_message(&server)?;
    if std::env::var("PRP_DEBUG").is_ok() {
        eprintln!("accept {name}: hello {} bytes: {:02x?}", hello.len(), &hello[..hello.len().min(16)]);
    }
    let (kind, client_name) = server::parse_client_hello(&hello)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad PRP hello"))?;
    if kind != expect {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected PRP channel kind"));
    }
    let debug = std::env::var("PRP_DEBUG").is_ok();
    let client = match open_for_write(&client_name) {
        Ok(c) => c,
        Err(e) => {
            if debug {
                eprintln!("accept {name}: open client pipe '{client_name}' failed: {e}");
            }
            return Err(e);
        }
    };
    if debug {
        eprintln!("accept {name}: client pipe opened");
    }
    // Order from the capture: server writes the `u32 1` success word, then the
    // client writes its pid, then the server writes the rest of the ack (the
    // internal endpoints for the command channel; a trailing word otherwise).
    write_all(&client, &ack[..4])?;
    let pid = read_pid(&server)?;
    if ack.len() > 4 {
        write_all(&client, &ack[4..])?;
    }
    if debug {
        eprintln!("accept {name}: pid {pid:#x} — handshake complete");
    }
    Ok(Channel { server, client, inbox: Vec::new() })
}

/// The PRP listener. The three channels are independent connections: PETM
/// opens the command channel alone to discover the device, and only opens the
/// subscription and time sync channels when it starts using it. So each is
/// accepted on its own, never waiting on the others. The internal endpoints
/// have fixed names (one PETM at a time), sent in every command
/// acknowledgement.
pub struct Listener {
    main_name: String,
    sub_name: String,
    ts_name: String,
}

impl Listener {
    pub fn bind(serial: &str) -> io::Result<Self> {
        // The command pipe is normally `TOBIIPRP-<serial>`, which PETM finds by
        // enumerating `TOBIIPRP-*`. PRP_MAIN_PIPE overrides it (to a name PETM
        // does not enumerate) so a logging relay can take the discoverable name
        // and forward to us. The sub/ts pipe names are unchanged: PETM learns
        // them from the command ack and reaches them directly.
        let main_name = std::env::var("PRP_MAIN_PIPE").unwrap_or_else(|_| format!("TOBIIPRP-{serial}"));
        drop(inbound_instance(&main_name)?); // fail now if the name is taken
        Ok(Listener {
            main_name,
            sub_name: format!("tobii-et5-bridge-{serial}-sub"),
            ts_name: format!("tobii-et5-bridge-{serial}-ts"),
        })
    }

    pub fn describe(&self) -> String {
        format!(r"\\.\pipe\{}", self.main_name)
    }

    /// Accept one command connection (the acknowledgement names the internal
    /// subscription and time sync endpoints). Fast: it never waits for the
    /// other two channels, so discovery probes are served at once.
    pub fn accept_command(&self) -> io::Result<Channel> {
        accept_one(&self.main_name, ChannelKind::Command, &server::command_ack(&self.sub_name, &self.ts_name))
    }

    /// Accept one subscription connection; the stream forwarder writes to it.
    pub fn accept_sub(&self) -> io::Result<Channel> {
        accept_one(&self.sub_name, ChannelKind::Subscription, &server::subscription_ack())
    }

    /// Accept one time sync connection.
    pub fn accept_timesync(&self) -> io::Result<Channel> {
        accept_one(&self.ts_name, ChannelKind::Timesync, &server::timesync_ack())
    }
}
