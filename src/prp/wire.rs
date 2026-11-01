//! The PRP (Platform Runtime Protocol) wire format, as Tobii Pro SDK 2.0 and
//! the Platform Runtime use it, recovered from captures and from the SDK's
//! `deserialize_property`/`serialize_operation` (see
//! `win/prp-capture-notes.md`).
//!
//! A frame is `[u32 "TPRP" = 0x50525054][u32 body length][u32 length ^ magic]`
//! then the body, all little-endian. The body is a sequence of versioned
//! structs, each followed by its variable-length area, closed by a `u32 0`:
//!
//! * struct = `[u32 size in bytes, this word included][fields…]`. A reader
//!   only uses the fields the size covers, so a struct may grow between
//!   versions.
//! * var = `[u32 length, this word included][data]`, `4` when empty. A string
//!   or array is `[u32 count-or-length][u32 offset]` in the struct, the offset
//!   counting from the start of the var area.
//! * a `u64` is two `u32`, high word first.
//!
//! The first struct is the header `{0xc, transaction id, operation}`.

/// Frame magic, `"TPRP"` little-endian.
pub const MAGIC: u32 = 0x5052_5054;

/// PRP operations (the header's third word).
pub mod op {
    pub const STREAM: u32 = 1;
    pub const PROPERTY_NOTIFICATION: u32 = 2;
    pub const PROPERTY_SET_REQUEST: u32 = 3;
    pub const PROPERTY_SET_RESPONSE: u32 = 4;
    pub const PROPERTY_GET_REQUEST: u32 = 5;
    pub const PROPERTY_GET_RESPONSE: u32 = 6;
    pub const PROPERTY_ENUMERATE_REQUEST: u32 = 7;
    pub const PROPERTY_ENUMERATE_RESPONSE: u32 = 8;
    pub const COMMAND_REQUEST: u32 = 9;
    pub const COMMAND_RESPONSE: u32 = 10;
    pub const COMPOUND_STREAM: u32 = 11;
}

/// One struct of a body: its fields as raw little-endian words, plus the
/// variable-length bytes that followed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Struct {
    /// The fixed fields, one `u32` each, the size word excluded.
    pub words: Vec<u32>,
    /// The variable-length area that followed, the length word excluded.
    pub var: Vec<u8>,
}

impl Struct {
    /// A struct with these words and no variable area.
    pub fn new(words: Vec<u32>) -> Self {
        Struct { words, var: Vec::new() }
    }

    /// A struct with these words and this variable area.
    pub fn with_var(words: Vec<u32>, var: Vec<u8>) -> Self {
        Struct { words, var }
    }

    /// Field `i`, or `None` when the struct is shorter (a reader treats a
    /// missing field as absent, like the SDK's size check).
    pub fn word(&self, i: usize) -> Option<u32> {
        self.words.get(i).copied()
    }

    /// A `u64` field at word `i` (high word first), or `None`.
    pub fn u64(&self, i: usize) -> Option<u64> {
        Some(u64::from(self.word(i)?) << 32 | u64::from(self.word(i + 1)?))
    }

    /// The NUL-terminated string a `[len][offset]` pair at word `i` points to
    /// in the var area, or `None` when the words or the offset are out of
    /// range.
    pub fn var_str(&self, i: usize) -> Option<&[u8]> {
        let offset = self.word(i + 1)? as usize;
        // The offset counts from the start of the var area, length word
        // included; `var` here already excludes that word.
        let start = offset.checked_sub(4)?;
        let slice = self.var.get(start..)?;
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        Some(&slice[..end])
    }
}

/// A whole PRP message: the header words (transaction id and operation) and
/// the structs that follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub transaction: u32,
    pub operation: u32,
    /// The structs after the header, in order.
    pub parts: Vec<Struct>,
}

impl Message {
    pub fn new(transaction: u32, operation: u32, parts: Vec<Struct>) -> Self {
        Message { transaction, operation, parts }
    }
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Parse the body of one frame (the bytes after the 12-byte frame header)
/// into its structs. Returns `None` on any inconsistent length.
pub fn parse_body(body: &[u8]) -> Option<Vec<Struct>> {
    let mut parts = Vec::new();
    let mut i = 0;
    while i + 4 <= body.len() {
        let size = read_u32(body, i)? as usize;
        if size == 0 {
            // The closing `u32 0`.
            break;
        }
        if size < 4 || i + size > body.len() {
            return None;
        }
        let mut words = Vec::with_capacity(size / 4 - 1);
        for w in 1..size / 4 {
            words.push(read_u32(body, i + w * 4)?);
        }
        i += size;
        // Optional var area.
        let mut var = Vec::new();
        if i + 4 <= body.len() {
            let vlen = read_u32(body, i)? as usize;
            if vlen >= 4 && i + vlen <= body.len() {
                var = body[i + 4..i + vlen].to_vec();
                i += vlen;
            } else if vlen >= 4 {
                return None;
            }
        }
        parts.push(Struct { words, var });
    }
    Some(parts)
}

/// Parse one whole frame (magic included). Returns the message and the number
/// of bytes consumed, or `None` if the buffer does not hold a complete, valid
/// frame yet.
pub fn parse_frame(buf: &[u8]) -> Option<(Message, usize)> {
    let magic = read_u32(buf, 0)?;
    let body_len = read_u32(buf, 4)? as usize;
    let check = read_u32(buf, 8)?;
    if magic != MAGIC || check != body_len as u32 ^ MAGIC {
        return None;
    }
    if buf.len() < 12 + body_len {
        return None;
    }
    let parts = parse_body(&buf[12..12 + body_len])?;
    let header = parts.first()?;
    let msg = Message {
        transaction: header.word(0)?,
        operation: header.word(1)?,
        parts: parts[1..].to_vec(),
    };
    Some((msg, 12 + body_len))
}

/// Serialise one struct (size word, fields, then its var area with its length
/// word) into `out`.
fn write_struct(out: &mut Vec<u8>, s: &Struct) {
    let size = 4 + s.words.len() * 4;
    out.extend_from_slice(&(size as u32).to_le_bytes());
    for w in &s.words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    let vlen = 4 + s.var.len();
    out.extend_from_slice(&(vlen as u32).to_le_bytes());
    out.extend_from_slice(&s.var);
}

/// Serialise a whole message into a frame (magic, length, check, body).
pub fn write_frame(msg: &Message) -> Vec<u8> {
    let mut body = Vec::new();
    write_struct(&mut body, &Struct::new(vec![msg.transaction, msg.operation]));
    for p in &msg.parts {
        write_struct(&mut body, p);
    }
    body.extend_from_slice(&0u32.to_le_bytes()); // closing word

    let mut frame = Vec::with_capacity(12 + body.len());
    frame.extend_from_slice(&MAGIC.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&((body.len() as u32) ^ MAGIC).to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrips() {
        let msg = Message::new(
            7,
            op::COMMAND_REQUEST,
            vec![
                Struct::new(vec![9]),
                Struct::with_var(vec![0xc, 0], b"human\0".to_vec()),
            ],
        );
        let bytes = write_frame(&msg);
        let (back, used) = parse_frame(&bytes).expect("parses");
        assert_eq!(used, bytes.len());
        assert_eq!(back, msg);
    }

    #[test]
    fn empty_var_is_four() {
        // A struct with no var area still writes the length word `4`.
        let mut out = Vec::new();
        write_struct(&mut out, &Struct::new(vec![1, 2]));
        // size 0xc (3 words), two fields, then the var length 4.
        assert_eq!(out, [0x0c, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0]);
    }

    #[test]
    fn bad_check_rejected() {
        let mut bytes = write_frame(&Message::new(1, 5, vec![Struct::new(vec![8])]));
        bytes[8] ^= 0xff;
        assert!(parse_frame(&bytes).is_none());
    }
}
