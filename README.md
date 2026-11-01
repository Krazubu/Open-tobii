# tobii-et5-bridge

Use a **Tobii Eye Tracker 5** with **Tobii Pro Eye Tracker Manager** (PETM) on
macOS.

Tobii ships no macOS support for the Eye Tracker 5, and PETM does not see it.
`ttpbridge` fills the gap: it opens the tracker over USB and serves it on the
Unix socket that Tobii's own USB service would create, so PETM's SDK
(`libtobii_research`) discovers it, applies its licence and talks to it as to
any other tracker.

> Status: prototype. Tested on one setup only: macOS 10.14.6 (Intel), Eye
> Tracker 5 `IS5FF-…` (USB `2104:0313`, firmware `02a1a6a977`), PETM 2.6.1.
> Discovery, licensing, the position guide, gaze visualisation and
> calibration work there. Independently written; not affiliated with or
> endorsed by Tobii.

## What you need

1. `ttpbridge`, running as root on the Mac the tracker is plugged into.
2. PETM 2.6.1 installed in a folder whose path does **not** contain the word
   `test` (see [Gotchas](#gotchas)).
3. PETM patched so that its interface lists the Eye Tracker 5
   (`scripts/patch-petm.sh`).

Nothing else from Tobii is needed or included here.

## Build

Requires a Rust toolchain. libusb is compiled from source, so there is nothing
else to install.

```sh
cargo build --release
```

This gives `target/release/ttpbridge` (the bridge) and
`target/release/ttpclient` (a small test client). Build on the Mac that will
run it, or pass `--target` for the other architecture: PETM 2.6.1 is an Intel
app that runs under Rosetta on Apple Silicon, but the bridge can be native
since the two only meet through a socket.

## Run

Quit anything else that holds the tracker (for instance `tobiid` from
[tobii-linux](https://github.com/Krazubu/tobii-linux)), then:

```sh
sudo target/release/ttpbridge
```

```
[   0.872] device serial IS5FF-100203736265
[   0.875] scp: /hw/platformtype=IS5LEYETRACKER5
[   0.875] scp: /fw/fwversion=02a1a6a977
[   0.875] listening on /var/run/TOBII-IS5FF-100203736265 (clients see tobii-ttp://IS5FF-100203736265)
```

Root is needed because Tobii's libraries only look for trackers in
`/var/run`, and only for real sockets there (a symlink is ignored). The socket
is made world-writable so PETM, running as you, can connect.

`--dir <path>` puts the socket elsewhere, which is handy for testing without
root:

```sh
target/release/ttpbridge --dir /tmp &
target/release/ttpclient /tmp/TOBII-<serial> --timesync
```

To start the bridge at boot, see
[`packaging/com.github.krazubu.tobii-et5-bridge.plist`](packaging/com.github.krazubu.tobii-et5-bridge.plist)
(not tested yet).

## Patch PETM

PETM's interface hides any tracker whose serial prefix is not on its list of
supported trackers, and `IS5FF` is on neither the built-in list nor the one it
downloads at start-up. The script swaps one obsolete entry of the built-in
list (`TX300`) for `IS5FF`: five bytes in the app's own `app.asar`, same
length, with a backup kept as `app.asar.orig`. It checks the SHA-256 of
`app.asar` first and refuses anything but PETM 2.6.1.

```sh
scripts/patch-petm.sh "/Applications/Tobii Pro Eye Tracker Manager.app"
scripts/patch-petm.sh --restore "/Applications/Tobii Pro Eye Tracker Manager.app"
```

Quit PETM before running it; use `sudo` if the app is in `/Applications`.
This patch only changes which products the interface displays. Licensing is
untouched: PETM applies its own licence to the tracker, and that succeeds.

## How it works

`ttpbridge` reproduces the client side of Tobii's USB service for Linux
(`tobiiusbserviced` 2.1.5), recovered by static analysis of that service and
of Tobii's macOS Platform Runtime, and checked against USB captures of the
Windows software from tobii-linux.

| Step | Detail |
|---|---|
| Socket | `/var/run/TOBII-<serial>`; Tobii's libraries map `tobii-ttp://<serial>` to it |
| Handshake | the client's first byte is its mode: 1 TTP, 3 time sync, 4 firmware, 6 SCP |
| Open a channel | vendor control OUT, request 65, `wValue` = channel (0…31) |
| Client → tracker | bulk EP `0x05`: `u32 LE channel`, `u32 LE length`, then data, in pieces of at most `0xFF7` bytes |
| Tracker → clients | bulk EP `0x83`: `u32 LE channel mask`, `u32 LE total length` (header included), then the payload, sent raw to every client whose bit is set |
| Close a channel | vendor control OUT, request 66, `wValue` = channel |
| Time sync | 18-byte requests relayed through vendor request 69 (OUT then IN, 22 bytes: a `u32` slot then the request) |
| Metadata | vendor request 48 returns `key=value` lines (platform type, firmware version), logged at start-up |

The messages inside the channels are Tobii's TTP protocol, which the tracker
already speaks over USB; the bridge never interprets them.

Not implemented: firmware upgrade (mode 4 clients are accepted and ignored,
on purpose) and SCP (mode 6 is answered "not supported").

## Gotchas

- **PETM treats itself as a test build when any of its launch arguments
  contains `test`**, including its own path (`~/tobii-test/…`), and then never
  starts its SDK module, so no tracker ever shows up.
- Only one program can hold the tracker's USB interface at a time.
- `ttpbridge` exits when the tracker is unplugged; restart it (or let launchd
  do it) once it is back.
- Never point Tobii's Platform Runtime for other trackers at an Eye Tracker
  5: it compares the device firmware with the one it embeds and reflashes on
  mismatch.

## Tools

- `src/bin/ttpclient.rs`: connects like Tobii's libraries do, sends the first
  TTP commands the Windows software sends (protocol negotiation, device
  identity) and prints the answers; `--timesync` also tries the time sync.
- `tools/research_probe.c`: asks `libtobii_research` (from your own PETM
  install) what it sees: discovery, identity, gaze. Build instructions are in
  the file. Outside PETM, gaze is refused for lack of a licence; that is
  expected.

## Licence

MIT. Tobii, Eye Tracker 5 and Eye Tracker Manager are trademarks of Tobii AB.
