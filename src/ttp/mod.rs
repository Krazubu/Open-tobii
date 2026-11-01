//! The TTP (Tobii Transfer Protocol) the Eye Tracker 5 speaks over USB: the
//! wire formats the PRP translator needs. These are the pure, device-free
//! pieces of tobii-linux's `tobii-proto`, ported here so the bridge can drive
//! the device itself (the TTP relay never had to: the SDK did). See each
//! file's header for what was ported and how.

pub mod tlv;
pub mod protocol;
pub mod gaze83;
pub mod calibration;
pub mod facts;
pub mod session;
