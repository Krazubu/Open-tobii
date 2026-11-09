//! What clients outside this workspace take from this crate.
//!
//! tobii-hub, a Flutter plugin (on flutter-rs) kept in its own repository,
//! takes this crate by git at a release tag. The first `use` below imports
//! what it imports, `hub_sees` is its `dispatch` with the calls into Flutter
//! replaced by the value they would send, and the rest uses the items as its
//! worker does: the mask built up in a `u32`, the errors shown with `{e}`. So
//! a change that renames, retypes or removes one of them, or adds a field to
//! [`HeadPose`] (which the hub names field by field), fails here, in the same
//! change, and not later in a build nobody here runs. Either keep the item, or
//! make the change and port the hub to it; then change this file to match the
//! hub. When the hub uses more of the crate, add it here. Where this file asks
//! more than the hub does (it builds a `HeadPose`, and holds `read_frame` to a
//! `Vec<u8>`), a failure says to check the hub, not that the hub is broken.
//!
//! What it cannot see is a change of behaviour that keeps the items, such as a
//! daemon that acks a stream and then sends nothing for it.
//!
//! The gaze switcher extension (`gaze-switcher/`) reads the wire format in
//! GJS, which no gate runs. The one-byte SUBSCRIBE it sends is held by
//! `legacy_one_byte_subscribe_is_accepted` in the crate's own tests.

use std::os::unix::net::UnixStream;

// tobii-hub's import, item for item (its rust/src/lib.rs).
use tobii_ipc::{
    HeadPose, STREAM_GAZE, STREAM_HEAD_POSE, STREAM_PRESENCE, ServerMsg, connect_or_spawn,
    decode_server, encode_subscribe, read_frame, write_frame,
};
// What tobiid uses to read and write those frames.
use tobii_ipc::{
    PRESENCE_PRESENT, decode_subscribe, encode_gaze, encode_head_pose, encode_presence,
    encode_reply, encode_subscribed,
};

/// What tobii-hub sends Dart for one frame.
#[derive(Debug, PartialEq)]
enum Seen {
    Status(&'static str),
    Gaze {
        ts: i64,
        valid: bool,
        x: f64,
        y: f64,
    },
    Head {
        ts: i64,
        /// The position's validity, then each angle's.
        valid: [bool; 4],
        position: [f64; 3],
        rotation: [f64; 3],
    },
    Presence {
        ts: i64,
        status: i64,
    },
    Nothing,
}

/// tobii-hub's `dispatch`, match arm for match arm.
fn hub_sees(body: &[u8]) -> Seen {
    match decode_server(body) {
        Some(ServerMsg::Subscribed { ok }) => {
            Seen::Status(if ok { "subscribed" } else { "refused" })
        }
        Some(ServerMsg::Gaze {
            ts_us,
            valid,
            xy,
            pupil_mm: _,
        }) => Seen::Gaze {
            ts: ts_us,
            valid,
            x: f64::from(xy[0]),
            y: f64::from(xy[1]),
        },
        Some(ServerMsg::HeadPose(HeadPose {
            ts_us,
            position_valid,
            position_mm,
            rotation_valid,
            rotation_rad,
        })) => Seen::Head {
            ts: ts_us,
            valid: [
                position_valid,
                rotation_valid[0],
                rotation_valid[1],
                rotation_valid[2],
            ],
            position: position_mm.map(f64::from),
            rotation: rotation_rad.map(f64::from),
        },
        Some(ServerMsg::Presence { ts_us, status }) => Seen::Presence {
            ts: ts_us,
            status: i64::from(status),
        },
        Some(_) | None => Seen::Nothing,
    }
}

/// `body` written as a frame and read back as the hub's worker reads one, over
/// a Unix socket like theirs.
fn through_a_socket(body: &[u8]) -> Vec<u8> {
    let (mut sender, mut hub) = UnixStream::pair().expect("a socket pair");
    if let Err(e) = write_frame(&mut sender, body) {
        panic!("write failed: {e}");
    }
    match read_frame(&mut hub) {
        Ok(Some(body)) => body,
        Ok(None) => panic!("the sender closed the connection"),
        Err(e) => panic!("read error: {e}"),
    }
}

#[test]
fn tobii_hub_subscribes_as_tobiid_reads_it() {
    let mut mask = 0u32;
    mask |= STREAM_HEAD_POSE;
    mask |= STREAM_GAZE;
    mask |= STREAM_PRESENCE;
    let body = through_a_socket(&encode_subscribe(mask));

    assert_eq!(body, [0x01, 0x06, 0x02, 0x00, 0x00]);
    assert_eq!(decode_subscribe(&body), Some(mask));
    // The hub's worker connects so, starting a tobiid when none listens, which
    // a test must not do: the closure is type-checked and never called.
    let _connect = || -> Option<UnixStream> {
        let stream = match connect_or_spawn() {
            Ok(stream) => stream,
            Err(e) => panic!("connect failed: {e}"),
        };
        stream.try_clone().ok()
    };
}

#[test]
fn tobii_hub_reads_the_ack_and_each_stream() {
    let pose = HeadPose {
        ts_us: 7,
        position_valid: true,
        position_mm: [-12.5, 48.25, 612.75],
        rotation_valid: [true, false, true],
        rotation_rad: [0.125, -0.375, 0.0625],
    };
    let frames = [
        (encode_subscribed(true), Seen::Status("subscribed")),
        (encode_subscribed(false), Seen::Status("refused")),
        (
            encode_gaze(5, true, [0.25, 0.75], [3.5, f32::NAN]),
            Seen::Gaze {
                ts: 5,
                valid: true,
                x: 0.25,
                y: 0.75,
            },
        ),
        (
            encode_head_pose(&pose),
            Seen::Head {
                ts: 7,
                valid: [true, true, false, true],
                position: [-12.5, 48.25, 612.75],
                rotation: [0.125, -0.375, 0.0625],
            },
        ),
        (
            encode_presence(9, PRESENCE_PRESENT),
            Seen::Presence { ts: 9, status: 2 },
        ),
    ];

    for (body, seen) in frames {
        assert_eq!(hub_sees(&through_a_socket(&body)), seen);
    }
}

#[test]
fn tobii_hub_drops_what_it_did_not_ask_for() {
    // A reply answers a request, which the hub never makes; an empty body
    // decodes to None. Both take the hub's last arm.
    assert_eq!(
        hub_sees(&through_a_socket(&encode_reply(1, 0, &[]))),
        Seen::Nothing
    );
    assert_eq!(hub_sees(&through_a_socket(&[])), Seen::Nothing);
}
