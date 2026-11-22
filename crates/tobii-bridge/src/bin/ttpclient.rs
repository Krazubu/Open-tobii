//! Minimal test client for ttpbridge: speaks to `TOBII-<serial>` the way
//! Tobii's client libraries do. It connects (mode 1), then sends the first
//! TTP commands the Windows Stream Engine sends an ET5 (from tobii-linux's
//! captured init_packets, without their 8-byte USB header), and prints the
//! answers. With `--timesync` it also tries a few mode 3 exchanges.
//!
//! The endpoint is the socket path on macOS (`/var/run/TOBII-<serial>`) and
//! the pipe name on Windows (`TOBII-<serial>`).

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Command 1000, the protocol upgrade (9 entries, protocols 0..8 version 1).
const CMD_1000: &str = "000000510000000100000000000003e8000000000000002f0000170000002800000009000100000001000100010002000100030001000400010005000100060001000700010008";
/// Command 1420, device strings (serial, model, generation, firmware).
const CMD_1420: &str = "0000005100000003000000000000058c0000000000000000";

type Reader = Box<dyn Read + Send>;
type Writer = Box<dyn Write + Send>;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn be(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(b[off..off + 4].try_into().unwrap())
}

/// Everything the bridge sends, as it arrives, on a channel.
fn spawn_reader(mut reader: Reader) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    rx
}

#[cfg(unix)]
fn connect(endpoint: &str, mode: u32) -> std::io::Result<(Reader, Writer)> {
    let mut s = std::os::unix::net::UnixStream::connect(endpoint)?;
    s.write_all(&[mode as u8])?;
    let r = s.try_clone()?;
    Ok((Box::new(r), Box::new(s)))
}

#[cfg(windows)]
fn connect(endpoint: &str, mode: u32) -> std::io::Result<(Reader, Writer)> {
    win::connect(endpoint.trim_start_matches(r"\\.\pipe\"), mode)
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let endpoint = args
        .next()
        .expect("usage: ttpclient <socket path | pipe name> [--timesync]");
    let timesync = args.next().as_deref() == Some("--timesync");

    if timesync {
        let (r, mut w) = connect(&endpoint, 3)?;
        let rx = spawn_reader(r);
        // libtobii_research's ttp_timesync_timestamp: u32 type 1, u32 sequence,
        // zeros; the answer carries seconds and microseconds in words 2 and 3.
        for seq in 1u32..=3 {
            let mut req = [0u8; 18];
            req[0..4].copy_from_slice(&1u32.to_le_bytes());
            req[4..8].copy_from_slice(&seq.to_le_bytes());
            w.write_all(&req)?;
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(rsp) if rsp.len() >= 16 => {
                    let wd =
                        |i: usize| u32::from_le_bytes(rsp[4 * i..4 * i + 4].try_into().unwrap());
                    println!(
                        "timesync reply: type {} seq {} device time {}.{:06} s",
                        wd(0),
                        wd(1),
                        wd(2),
                        wd(3)
                    );
                }
                Ok(rsp) => println!("timesync: short reply ({} bytes)", rsp.len()),
                Err(e) => println!("timesync: no reply ({e})"),
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    let (r, mut w) = connect(&endpoint, 1)?;
    let rx = spawn_reader(r);
    for (name, msg) in [("1000", hex(CMD_1000)), ("1420", hex(CMD_1420))] {
        println!("-> command {name} ({} bytes)", msg.len());
        w.write_all(&msg)?;
    }

    let mut pending = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(chunk) => pending.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                println!("bridge closed the connection");
                break;
            }
        }
        while pending.len() >= 24 {
            let len = be(&pending, 20) as usize;
            if pending.len() < 24 + len {
                break;
            }
            let (marker, seq, status, id, err) = (
                be(&pending, 0),
                be(&pending, 4),
                be(&pending, 8),
                be(&pending, 12),
                be(&pending, 16),
            );
            let payload = &pending[24..24 + len];
            let text: String = payload
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        ' '
                    }
                })
                .collect::<String>()
                .split_whitespace()
                .filter(|w| w.len() > 2)
                .collect::<Vec<_>>()
                .join(" | ");
            println!(
                "<- marker {marker:#04x} seq {seq} status {status} id {id} error {err:#x} len {len}  {text}"
            );
            pending.drain(..24 + len);
        }
    }
    Ok(())
}

/// The client half of Tobii's Windows pipe transport.
#[cfg(windows)]
mod win {
    use super::{Reader, Writer};
    use std::io::{self, Read, Write};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, GENERIC_WRITE,
        GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, OPEN_EXISTING, PIPE_ACCESS_INBOUND, ReadFile, WriteFile,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    struct Pipe(HANDLE);
    unsafe impl Send for Pipe {}
    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut n = 0u32;
            let ok = unsafe {
                ReadFile(
                    self.0,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut n,
                    null_mut(),
                )
            };
            if ok != 0 {
                return Ok(n as usize);
            }
            match unsafe { GetLastError() } {
                ERROR_MORE_DATA => Ok(n as usize),
                ERROR_BROKEN_PIPE => Ok(0),
                e => Err(io::Error::from_raw_os_error(e as i32)),
            }
        }
    }
    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut n = 0u32;
            if unsafe { WriteFile(self.0, buf.as_ptr(), buf.len() as u32, &mut n, null_mut()) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(n as usize)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn connect(server: &str, mode: u32) -> io::Result<(Reader, Writer)> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let own = format!("client_{server}_{:08x}_{n:016x}", unsafe {
            GetCurrentProcessId()
        });

        let own_path = wide(&format!(r"\\.\pipe\{own}"));
        let inbound = unsafe {
            CreateNamedPipeW(
                own_path.as_ptr(),
                PIPE_ACCESS_INBOUND,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                0,
                0x80000,
                5000,
                null(),
            )
        };
        if inbound == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let inbound = Pipe(inbound);

        let server_path = wide(&format!(r"\\.\pipe\{server}"));
        let out = unsafe {
            CreateFileW(
                server_path.as_ptr(),
                GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if out == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut out = Pipe(out);

        let mut hello = mode.to_le_bytes().to_vec();
        hello.extend_from_slice(own.as_bytes());
        hello.push(0);
        out.write_all(&hello)?;

        // Wait for the server to open our pipe.
        if unsafe { ConnectNamedPipe(inbound.0, null_mut()) } == 0
            && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED
        {
            return Err(io::Error::last_os_error());
        }
        Ok((Box::new(inbound), Box::new(out)))
    }
}
