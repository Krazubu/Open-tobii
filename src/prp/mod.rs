//! The PRP side of the bridge: the Platform Runtime Protocol that Tobii Pro
//! SDK 2.0 (Eye Tracker Manager 2.7.2) speaks, and its translation to the
//! TTP the Eye Tracker 5 speaks over USB.
//!
//! The official Tobii runtime is a PRP↔TTP translator; this reproduces the
//! translation so the SDK can reach the tracker with no runtime installed.
//! The device stays the authority for its own licence: a licence the SDK
//! sends is forwarded to the tracker's TTP licence command unchanged, and the
//! tracker's verdict is relayed back. The bridge never blanks, forges or
//! validates a licence itself.

pub mod wire;
pub mod server;
pub mod translate;

#[cfg(windows)]
#[path = "transport_windows.rs"]
pub mod transport;
#[cfg(unix)]
#[path = "transport_unix.rs"]
pub mod transport;
