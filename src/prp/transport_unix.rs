//! PRP transport on Unix — not implemented yet.
//!
//! On macOS, Pro SDK 2.0 reaches the Platform Runtime over POSIX sockets
//! (`prp_client`'s `client_posix.cpp`/`server_posix.cpp`), not the TTP Unix
//! socket the bridge already serves. That path has not been captured or
//! reversed, so the PRP server is Windows-only for now. The handshake and
//! wire logic in the sibling modules are platform-independent and are built
//! and tested on every host.

use super::wire::Message;
use std::io;

pub struct Channel;

impl Channel {
    pub fn read_frame(&mut self) -> io::Result<Message> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "PRP is not supported on this platform yet"))
    }
    pub fn write_frame(&self, _msg: &Message) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "PRP is not supported on this platform yet"))
    }
}

pub struct PrpClient {
    pub command: Channel,
    pub subscription: Channel,
    pub timesync: Channel,
    pub pid: u32,
}

pub struct Listener;

impl Listener {
    pub fn bind(_serial: &str) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "PRP is not supported on this platform yet"))
    }
    pub fn describe(&self) -> String {
        String::new()
    }
    pub fn accept(&self) -> io::Result<PrpClient> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "PRP is not supported on this platform yet"))
    }
}
