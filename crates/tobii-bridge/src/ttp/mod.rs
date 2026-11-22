//! The TTP (Tobii Transfer Protocol) the Eye Tracker 5 speaks over USB. The
//! wire formats are `tobii-proto`'s, the same ones the daemon uses; only the
//! [`session`] is the bridge's own, since the PRP translator drives the
//! device itself (the TTP relay never had to: the SDK did).

pub use tobii_proto::{calibration, facts, gaze83, protocol, tlv};

pub mod session;
