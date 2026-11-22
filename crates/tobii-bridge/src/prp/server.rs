//! The PRP server's connection handshake, reproduced from the capture
//! (`win/prp-capture-notes.md`), as pure functions so they can be checked
//! against the captured bytes without a live pipe.
//!
//! A PRP client opens three connections, each tagged in its first message:
//!
//! * the client writes, to the server pipe, `[u32 1][u32 kind][its own pipe
//!   name, NUL-terminated]`, where `kind` is `0x4e21` command, `0x4e22`
//!   subscription or `0x4e23` time sync;
//! * the server opens the client's pipe and writes an acknowledgement that
//!   depends on the kind (below), then the client writes `[u32 pid]` back;
//! * PRP frames then flow, the server reading from its pipe and writing to the
//!   client's.
//!
//! On the command connection the acknowledgement carries the two internal
//! endpoints (`prp-internal://<uuid>`) the client must open next, for the
//! subscription and the time sync connections; the server serves those under
//! those names.

/// Which of a PRP client's three connections this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Command,
    Subscription,
    Timesync,
}

impl ChannelKind {
    /// The `kind` word of the client's first message.
    pub fn from_word(w: u32) -> Option<Self> {
        match w {
            0x4e21 => Some(ChannelKind::Command),
            0x4e22 => Some(ChannelKind::Subscription),
            0x4e23 => Some(ChannelKind::Timesync),
            _ => None,
        }
    }

    pub fn word(self) -> u32 {
        match self {
            ChannelKind::Command => 0x4e21,
            ChannelKind::Subscription => 0x4e22,
            ChannelKind::Timesync => 0x4e23,
        }
    }
}

/// Prefix of every internal endpoint URL.
pub const INTERNAL_PREFIX: &str = "prp-internal://";

/// Parse a client's first message `[u32 1][u32 kind][name\0]` into the channel
/// kind and the client's own pipe name.
pub fn parse_client_hello(msg: &[u8]) -> Option<(ChannelKind, String)> {
    if msg.len() < 8 {
        return None;
    }
    let version = u32::from_le_bytes(msg[0..4].try_into().ok()?);
    if version != 1 {
        return None;
    }
    let kind = ChannelKind::from_word(u32::from_le_bytes(msg[4..8].try_into().ok()?))?;
    let name_bytes = &msg[8..];
    let end = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    let name = String::from_utf8(name_bytes[..end].to_vec()).ok()?;
    Some((kind, name))
}

/// The acknowledgement the server writes on the command connection: `[u32 1]
/// [u32 tag]` then the subscription and time sync endpoints as NUL-terminated
/// `prp-internal://<uuid>` strings. The client reads these and opens its other
/// two connections to them.
///
/// The `tag` word differs by platform's runtime: the Windows named-pipe runtime
/// sends `4` (see `command_ack_matches_capture`, from `cap-272-main.txt`), but
/// the macOS AF_UNIX runtime sends `0` — captured from the official
/// `platform_runtime_*_MAC_x64_service` serving a Spark on 2026-11-08. Sending
/// Windows's `4` on macOS made the SDK's discovery read it as a failure and
/// drop the device (0 trackers), so each transport passes its own tag.
pub fn command_ack_tagged(tag: u32, sub_uuid: &str, ts_uuid: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&tag.to_le_bytes());
    v.extend_from_slice(format!("{INTERNAL_PREFIX}{sub_uuid}").as_bytes());
    v.push(0);
    v.extend_from_slice(format!("{INTERNAL_PREFIX}{ts_uuid}").as_bytes());
    v.push(0);
    v
}

/// The Windows command acknowledgement (tag `4`).
pub fn command_ack(sub_uuid: &str, ts_uuid: &str) -> Vec<u8> {
    command_ack_tagged(4, sub_uuid, ts_uuid)
}

/// The acknowledgement for the subscription connection: `[u32 1][u32 tag]
/// [u16 0]`. Like the command ack, the `tag` word is runtime-specific: Windows
/// sends `3`, the macOS AF_UNIX runtime sends `0` (captured from the official
/// platform runtime on 2026-11-08).
pub fn subscription_ack_tagged(tag: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&tag.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

/// The Windows subscription acknowledgement (tag `3`).
pub fn subscription_ack() -> Vec<u8> {
    subscription_ack_tagged(3)
}

/// The acknowledgement for the time sync connection: `[u32 1][u32 0][u16 0]`.
pub fn timesync_ack() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

/// The internal endpoint name (the part after `prp-internal://`) the client
/// should connect to, parsed from the command acknowledgement. Used when
/// relaying; the server itself picks the names.
pub fn internal_endpoints(ack: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for part in ack.get(8..).unwrap_or(&[]).split(|&b| b == 0) {
        if let Ok(s) = std::str::from_utf8(part)
            && let Some(rest) = s.strip_prefix(INTERNAL_PREFIX)
        {
            out.push(rest.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bytes taken from win/cap-272-main.txt, the …299 session.
    const HELLO_CMD: &[u8] = &[0x01, 0, 0, 0, 0x21, 0x4e, 0, 0];
    const SUB_UUID: &str = "cba5ac6b-fd07-45c5-87c2-c6d38af0fb20";
    const TS_UUID: &str = "025cbdd0-a05f-4e9d-bdf8-122c78c2452c";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn parses_the_three_hellos() {
        let mut m = HELLO_CMD.to_vec();
        m.extend_from_slice(b"client_TOBIIPRP-CAPTURE_000042bc_00007ff9fc6aec8f\0");
        let (kind, name) = parse_client_hello(&m).unwrap();
        assert_eq!(kind, ChannelKind::Command);
        assert_eq!(name, "client_TOBIIPRP-CAPTURE_000042bc_00007ff9fc6aec8f");
        assert_eq!(
            ChannelKind::from_word(0x4e22),
            Some(ChannelKind::Subscription)
        );
        assert_eq!(ChannelKind::from_word(0x4e23), Some(ChannelKind::Timesync));
    }

    #[test]
    fn command_ack_matches_capture() {
        let got = command_ack(SUB_UUID, TS_UUID);
        let want = hex(
            "01000000040000007072702d696e7465726e616c3a2f2f63626135616336622d666430372d343563352d383763322d6336643338616630666232300\
             07072702d696e7465726e616c3a2f2f30323563626464302d613035662d346539642d626466382d31323263373863323435326300",
        );
        assert_eq!(got, want);
        assert_eq!(internal_endpoints(&got), vec![SUB_UUID, TS_UUID]);
    }

    #[test]
    fn simple_acks_match_capture() {
        assert_eq!(subscription_ack(), hex("01000000030000000000"));
        assert_eq!(timesync_ack(), hex("01000000000000000000"));
    }
}
