//! macOS (Unix) endpoint: a stream socket `TOBII-<serial>` in `/var/run`,
//! where Tobii's libraries look for local trackers. A client's first byte is
//! its connection mode; after that the same stream carries TTP both ways.

use crate::Conn;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

pub struct Pending(UnixStream);

impl Listener {
    /// Bind `<dir>/<name>`, `dir` defaulting to `/var/run`.
    pub fn bind(name: &str, dir: Option<&str>) -> io::Result<Self> {
        let path = PathBuf::from(dir.unwrap_or("/var/run")).join(name);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        // The service runs as root; clients such as Eye Tracker Manager do not.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
        Ok(Listener { listener, path })
    }

    pub fn describe(&self) -> String {
        self.path.display().to_string()
    }

    pub fn accept(&mut self) -> io::Result<Pending> {
        self.listener.accept().map(|(stream, _)| Pending(stream))
    }
}

impl Pending {
    pub fn handshake(mut self) -> io::Result<Conn> {
        let mut mode = [0u8; 1];
        self.0.read_exact(&mut mode)?;
        let writer = self.0.try_clone()?;
        Ok(Conn {
            mode: u32::from(mode[0]),
            reader: Box::new(self.0),
            writer: Box::new(writer),
        })
    }
}
