//! Minimal test client for ttpbridge: speaks to `TOBII-<serial>` the way
//! Tobii's client libraries do. It sends the mode byte, then the first TTP
//! commands the Windows Stream Engine sends an ET5 (from tobii-linux's
//! captured init_packets, without their 8-byte USB header), and prints the
//! answers. With `--timesync` it also tries one mode 3 exchange.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Command 1000, the protocol upgrade (9 entries, protocols 0..8 version 1).
const CMD_1000: &str = "000000510000000100000000000003e8000000000000002f0000170000002800000009000100000001000100010002000100030001000400010005000100060001000700010008";
/// Command 1420, device strings (serial, model, generation, firmware).
const CMD_1420: &str = "0000005100000003000000000000058c0000000000000000";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn be(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(b[off..off + 4].try_into().unwrap())
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: ttpclient <socket path> [--timesync]");
    let timesync = args.next().as_deref() == Some("--timesync");

    if timesync {
        let mut ts = UnixStream::connect(&path)?;
        ts.set_read_timeout(Some(Duration::from_secs(3)))?;
        ts.write_all(&[3])?;
        // libtobii_research's ttp_timesync_timestamp: u32 type 1, u32 sequence,
        // zeros; the answer carries seconds and microseconds in words 2 and 3.
        for seq in 1u32..=3 {
            let mut req = [0u8; 18];
            req[0..4].copy_from_slice(&1u32.to_le_bytes());
            req[4..8].copy_from_slice(&seq.to_le_bytes());
            ts.write_all(&req)?;
            let mut rsp = [0u8; 18];
            match ts.read_exact(&mut rsp) {
                Ok(()) => {
                    let w = |i: usize| u32::from_le_bytes(rsp[4 * i..4 * i + 4].try_into().unwrap());
                    println!(
                        "timesync reply: type {} seq {} device time {}.{:06} s",
                        w(0), w(1), w(2), w(3)
                    );
                }
                Err(e) => println!("timesync: no reply ({e})"),
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    let mut s = UnixStream::connect(&path)?;
    s.set_read_timeout(Some(Duration::from_millis(200)))?;
    s.write_all(&[1])?;
    for (name, msg) in [("1000", hex(CMD_1000)), ("1420", hex(CMD_1420))] {
        println!("-> command {name} ({} bytes)", msg.len());
        s.write_all(&msg)?;
    }

    let mut pending = Vec::new();
    let mut buf = [0u8; 65536];
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => {
                println!("bridge closed the connection");
                break;
            }
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
        while pending.len() >= 24 {
            let len = be(&pending, 20) as usize;
            if pending.len() < 24 + len {
                break;
            }
            let (marker, seq, status, id, err) =
                (be(&pending, 0), be(&pending, 4), be(&pending, 8), be(&pending, 12), be(&pending, 16));
            let payload = &pending[24..24 + len];
            let text: String = payload
                .iter()
                .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
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
