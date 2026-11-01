//! Windows endpoint: the named pipe `\\.\pipe\TOBII-<serial>`, as Tobii's
//! libraries expect (reversed from Stream Engine 4.25 x64).
//!
//! Tobii's Windows transport uses one pipe per direction. The client creates
//! its own inbound message pipe, named `client_TOBII-<serial>_<pid>_<n>`,
//! connects for writing to `\\.\pipe\TOBII-<serial>` and sends one message:
//! `[u32 LE connection mode][its pipe name, NUL-terminated]`. It then waits
//! up to 3 s for the server to open that pipe. After that it writes TTP to
//! the server pipe and reads the answers from its own.

use crate::Conn;
use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

/// In-buffer size the clients use for their own pipe.
const PIPE_BUFFER: u32 = 0x80000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// An owned pipe handle.
struct Pipe(HANDLE);

// Pipe handles may be used from any thread.
unsafe impl Send for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Read for Pipe {
    /// Byte-stream view of a message pipe: a message larger than `buf`
    /// continues on the next call.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut n = 0u32;
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr(), len, &mut n, null_mut()) };
        if ok != 0 {
            return Ok(n as usize);
        }
        match unsafe { GetLastError() } {
            ERROR_MORE_DATA => Ok(n as usize),
            ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => Ok(0),
            e => Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
}

impl Write for Pipe {
    /// Each call is one message.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut n = 0u32;
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ok = unsafe { WriteFile(self.0, buf.as_ptr(), len, &mut n, null_mut()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        unsafe { FlushFileBuffers(self.0) };
        Ok(())
    }
}

pub struct Listener {
    name: String,
    path: Vec<u16>,
    attrs: SECURITY_ATTRIBUTES,
}

pub struct Pending(Pipe);

impl Listener {
    pub fn bind(name: &str, _dir: Option<&str>) -> io::Result<Self> {
        // Anyone may write to the pipe: the bridge may run as an administrator
        // while Eye Tracker Manager runs as the user.
        let sddl = wide("D:(A;;GA;;;WD)(A;;GA;;;SY)(A;;GA;;;BA)");
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd as *mut c_void,
            bInheritHandle: 0,
        };
        let listener = Listener { name: name.to_string(), path: wide(&format!(r"\\.\pipe\{name}")), attrs };
        // Fail now rather than at the first client if the name is taken.
        drop(listener.instance()?);
        Ok(listener)
    }

    pub fn describe(&self) -> String {
        format!(r"\\.\pipe\{}", self.name)
    }

    fn instance(&self) -> io::Result<Pipe> {
        let h = unsafe {
            CreateNamedPipeW(
                self.path.as_ptr(),
                PIPE_ACCESS_INBOUND,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                PIPE_BUFFER,
                0,
                &self.attrs,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Pipe(h))
    }

    pub fn accept(&mut self) -> io::Result<Pending> {
        let pipe = self.instance()?;
        let ok = unsafe { ConnectNamedPipe(pipe.0, null_mut()) };
        if ok == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            return Err(io::Error::last_os_error());
        }
        Ok(Pending(pipe))
    }
}

impl Pending {
    pub fn handshake(mut self) -> io::Result<Conn> {
        let msg = read_message(&mut self.0)?;
        if msg.len() < 5 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short handshake"));
        }
        let mode = u32::from_le_bytes(msg[0..4].try_into().unwrap());
        let name_bytes = &msg[4..];
        let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
        let writer = open_client_pipe(&name)?;
        Ok(Conn { mode, reader: Box::new(self.0), writer: Box::new(writer) })
    }
}

/// One whole message from a message-mode pipe.
fn read_message(pipe: &mut Pipe) -> io::Result<Vec<u8>> {
    let mut msg = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let mut n = 0u32;
        let ok = unsafe { ReadFile(pipe.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) };
        msg.extend_from_slice(&buf[..n as usize]);
        if ok != 0 {
            return Ok(msg);
        }
        if unsafe { GetLastError() } != ERROR_MORE_DATA {
            return Err(io::Error::last_os_error());
        }
    }
}

/// Open the client's own pipe for writing our answers.
fn open_client_pipe(name: &str) -> io::Result<Pipe> {
    let path = wide(&format!(r"\\.\pipe\{name}"));
    for _ in 0..4 {
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
