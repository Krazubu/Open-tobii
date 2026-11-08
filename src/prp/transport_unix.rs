//! PRP transport on macOS (and other Unix): AF_UNIX sockets.
//!
//! Tobii Pro SDK 2.0 reaches the Platform Runtime over POSIX sockets, not the
//! Windows named pipes. Reversed from `libtobii_research.2.0.1.dylib`
//! (`tobii_client_posix` / `tobii_server_posix`, `client_handshake_prp`,
//! `transport_create_socket_address`):
//!
//! * A URL name `N` maps to the socket path `/var/run/N` (or `/tmp/N` for
//!   "visibility 2"); `sun_family` is `AF_UNIX`. The SDK's discovery scans
//!   `/var/run/TOBIIPRP-*`, so the command socket must live there — which
//!   needs root to create (run the bridge with sudo).
//! * The handshake is the same bytes as Windows, but the socket is
//!   bidirectional, so there is one socket per channel (no separate inbound
//!   pipe): the client connects, writes `[u32 1][u32 kind][name\0]`, reads
//!   `[u32 1]`, writes `[u32 pid]`, then reads the rest of the acknowledgement,
//!   all on the same connection. The command ack names the subscription and
//!   time sync endpoints as `prp-internal://<name>`; the client opens those as
//!   `/var/run/<name>`, so the server binds them under those names too.
//!
//! The wire, handshake and acknowledgement logic is shared with Windows (see
//! [`super::server`]); only the byte transport differs.

use super::server::{self, ChannelKind};
use super::wire::{self, Message};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// Where the sockets live. The SDK uses `/var/run` for default visibility;
/// `PRP_SOCKET_DIR` overrides it (e.g. for a relay or a rootless test).
fn socket_dir() -> PathBuf {
    PathBuf::from(std::env::var("PRP_SOCKET_DIR").unwrap_or_else(|_| "/var/run".into()))
}

fn socket_path(name: &str) -> PathBuf {
    socket_dir().join(name)
}

/// Bind a listening AF_UNIX socket at `/var/run/<name>`, replacing any stale
/// socket file, and make it connectable by the user's PETM (the bridge runs as
/// root to write `/var/run`, PETM as the user).
fn bind_listener(name: &str) -> io::Result<UnixListener> {
    let path = socket_path(name);
    // A stale socket file from a previous run makes bind fail with EADDRINUSE.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777))?;
    Ok(listener)
}

/// One PRP connection: a single bidirectional stream. Frames may span several
/// reads, so reads accumulate until a whole frame parses.
pub struct Channel {
    sock: UnixStream,
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
            let mut buf = [0u8; 65536];
            let n = self.sock.read(&mut buf)?;
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            self.inbox.extend_from_slice(&buf[..n]);
        }
    }

    /// Write one PRP message to the connection.
    pub fn write_frame(&self, msg: &Message) -> io::Result<()> {
        (&self.sock).write_all(&wire::write_frame(msg))
    }
}

/// Read exactly `n` bytes from the stream.
fn read_exact(sock: &mut UnixStream, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    sock.read_exact(&mut buf)?;
    Ok(buf)
}

/// Read the client's hello `[u32 1][u32 kind][name\0]`: the 8 fixed bytes, then
/// bytes up to and including the NUL. The client blocks for the server's `[1]`
/// before sending anything more, so this never over-reads.
fn read_hello(sock: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut hello = read_exact(sock, 8)?;
    let mut one = [0u8; 1];
    loop {
        sock.read_exact(&mut one)?;
        hello.push(one[0]);
        if one[0] == 0 {
            return Ok(hello);
        }
        if hello.len() > 0x200 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "PRP hello name too long"));
        }
    }
}

/// Accept one connection on `listener`, check its kind, and run the handshake:
/// server writes `[u32 1]`, client writes `[u32 pid]`, server writes the rest
/// of the ack (the internal endpoints for the command channel; a trailing word
/// otherwise). Returns the channel.
fn accept_one(listener: &UnixListener, expect: ChannelKind, ack: &[u8]) -> io::Result<Channel> {
    let (mut sock, _addr) = listener.accept()?;
    let hello = read_hello(&mut sock)?;
    let debug = std::env::var("PRP_DEBUG").is_ok();
    if debug {
        eprintln!("accept {expect:?}: hello {} bytes: {:02x?}", hello.len(), &hello[..hello.len().min(16)]);
    }
    let (kind, _client_name) = server::parse_client_hello(&hello)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad PRP hello"))?;
    if kind != expect {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected PRP channel kind"));
    }
    // Same order as the capture: the `u32 1` success word, then the client's
    // pid, then the rest of the ack. Unlike Windows these all go on the one
    // bidirectional socket.
    sock.write_all(&ack[..4])?;
    let pid = u32::from_le_bytes(read_exact(&mut sock, 4)?.try_into().unwrap());
    if ack.len() > 4 {
        sock.write_all(&ack[4..])?;
    }
    if debug {
        eprintln!("accept {expect:?}: pid {pid:#x} — handshake complete");
    }
    Ok(Channel { sock, inbox: Vec::new() })
}

/// The PRP listener: three bound AF_UNIX sockets, one per channel. PETM opens
/// the command socket alone to discover the device and the other two only when
/// it starts using it, so each is accepted independently.
pub struct Listener {
    main: UnixListener,
    sub: UnixListener,
    timesync: UnixListener,
    main_name: String,
    sub_name: String,
    ts_name: String,
}

impl Listener {
    pub fn bind(serial: &str) -> io::Result<Self> {
        // The command socket is normally /var/run/TOBIIPRP-<serial>, which the
        // SDK finds by scanning /var/run/TOBIIPRP-*. PRP_MAIN_PIPE overrides the
        // name (for a logging relay). The sub/ts names are learned by PETM from
        // the command ack, so they can be anything the server also binds.
        let main_name = std::env::var("PRP_MAIN_PIPE").unwrap_or_else(|_| format!("TOBIIPRP-{serial}"));
        let sub_name = format!("tobii-et5-bridge-{serial}-sub");
        let ts_name = format!("tobii-et5-bridge-{serial}-ts");
        let main = bind_listener(&main_name)?;
        let sub = bind_listener(&sub_name)?;
        let timesync = bind_listener(&ts_name)?;
        Ok(Listener { main, sub, timesync, main_name, sub_name, ts_name })
    }

    pub fn describe(&self) -> String {
        socket_path(&self.main_name).display().to_string()
    }

    /// Accept one command connection (its ack names the internal subscription
    /// and time sync endpoints).
    pub fn accept_command(&self) -> io::Result<Channel> {
        accept_one(&self.main, ChannelKind::Command, &server::command_ack(&self.sub_name, &self.ts_name))
    }

    /// Accept one subscription connection; the stream forwarder writes to it.
    pub fn accept_sub(&self) -> io::Result<Channel> {
        accept_one(&self.sub, ChannelKind::Subscription, &server::subscription_ack())
    }

    /// Accept one time sync connection.
    pub fn accept_timesync(&self) -> io::Result<Channel> {
        accept_one(&self.timesync, ChannelKind::Timesync, &server::timesync_ack())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        for name in [&self.main_name, &self.sub_name, &self.ts_name] {
            let _ = std::fs::remove_file(socket_path(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::wire::{op, Struct};

    /// A client connects to the command socket, runs the reversed handshake,
    /// then a PRP frame round-trips: the server reads it and echoes it back.
    /// Exercises the socket bind, the hello read, the ack order and framing
    /// without a device.
    #[test]
    fn handshake_and_frame_roundtrip() {
        let dir = std::env::temp_dir().join(format!("prp-unix-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Only this test touches these, so the process-wide env is safe here.
        unsafe {
            std::env::set_var("PRP_SOCKET_DIR", &dir);
            std::env::set_var("PRP_MAIN_PIPE", "TESTMAIN");
        }
        let listener = Listener::bind("TESTSERIAL").unwrap();

        let server = std::thread::spawn(move || {
            let mut ch = listener.accept_command().unwrap();
            let msg = ch.read_frame().unwrap();
            ch.write_frame(&msg).unwrap(); // echo
        });

        let mut c = UnixStream::connect(dir.join("TESTMAIN")).unwrap();
        // Hello: [u32 1][u32 command kind][name\0].
        let mut hello = Vec::new();
        hello.extend_from_slice(&1u32.to_le_bytes());
        hello.extend_from_slice(&ChannelKind::Command.word().to_le_bytes());
        hello.extend_from_slice(b"client_test\0");
        c.write_all(&hello).unwrap();

        // Server success word, then our pid, then the rest of the ack.
        let mut one = [0u8; 4];
        c.read_exact(&mut one).unwrap();
        assert_eq!(u32::from_le_bytes(one), 1);
        c.write_all(&4321u32.to_le_bytes()).unwrap();
        // Ack rest begins with the endpoint-count word 4; read and sanity-check.
        let mut rest = [0u8; 4];
        c.read_exact(&mut rest).unwrap();
        assert_eq!(u32::from_le_bytes(rest), 4);
        // Drain the two endpoint strings so the socket buffer is clear.
        let mut endpoints = vec![0u8; 0];
        let mut b = [0u8; 1];
        let mut nuls = 0;
        while nuls < 2 {
            c.read_exact(&mut b).unwrap();
            endpoints.push(b[0]);
            if b[0] == 0 {
                nuls += 1;
            }
        }
        assert!(String::from_utf8_lossy(&endpoints).contains("prp-internal://"));

        // A PRP frame round-trips.
        let sent = Message::new(7, op::COMMAND_RESPONSE, vec![Struct::new(vec![0, 9]), Struct::new(vec![])]);
        c.write_all(&wire::write_frame(&sent)).unwrap();
        let (echoed, _n) = {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 256];
            loop {
                let n = c.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(parsed) = wire::parse_frame(&buf) {
                    break parsed;
                }
            }
        };
        assert_eq!(echoed.transaction, 7);
        assert_eq!(echoed.operation, op::COMMAND_RESPONSE);

        server.join().unwrap();
    }
}
