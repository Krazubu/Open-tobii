# ttpbridge (formerly tobii-et5-bridge)

> Part of [open-tobii](../../README.md), in `crates/tobii-bridge`. It shares
> the daemon's protocol code (`tobii-proto`) and init replay (`tobii-usb`).
> Paths below are relative to this directory; run `cargo` from the
> repository root.

Use a **Tobii Eye Tracker 5** with **Tobii Pro Eye Tracker Manager** (PETM) on
macOS.

Tobii ships no macOS support for the Eye Tracker 5, and PETM does not see it.
`ttpbridge` fills the gap: it opens the tracker over USB and serves it on the
Unix socket that Tobii's own USB service would create, so PETM's SDK
(`libtobii_research`) discovers it, applies its licence and talks to it as to
any other tracker.

> Status: prototype. Tested with one Eye Tracker 5 `IS5FF-…` (USB
> `2104:0313`, firmware `02a1a6a977`) on macOS 10.14.6 (Intel) with PETM
> 2.6.1, on a MacBook Pro M4 with PETM 2.6.1 and 2.7.2, and on Windows 11
> ARM64 with PETM 2.6.1. Discovery, licensing, the position guide, gaze
> visualisation and calibration work.
> Independently written; not affiliated with or endorsed by Tobii.

## What you need

1. `ttpbridge`, running as root on the Mac the tracker is plugged into.
2. PETM 2.6.1 (or 2.7.2 plus a copy of 2.6.1) installed in a folder whose
   path does **not** contain the word `test` (see [Gotchas](#gotchas)).
3. PETM patched so that its interface lists the Eye Tracker 5
   (`scripts/patch-petm.sh`).

Nothing else from Tobii is needed or included here.

## Build

Requires a Rust toolchain. With `--features vendored`, libusb is compiled
from source, so there is nothing else to install (without it, the system's
libusb is used).

```sh
cargo build --release -p ttpbridge --features vendored
```

This gives `target/release/ttpbridge` (the bridge) and
`target/release/ttpclient` (a small test client). Build on the Mac that will
run it, or pass `--target` for the other architecture: PETM 2.6.1 is an Intel
app that runs under Rosetta on Apple Silicon, but the bridge can be native
since the two only meet through a socket.

## Run

Quit anything else that holds the tracker (for instance `tobiid` from
this repository), then:

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

### Tobii SDK 3 (Eye Tracker Browser, `TobiiSDK.framework`, `tobiictl`)

The same `ttpbridge` also serves the tracker to the older Tobii SDK 3
(`TobiiSDK.framework` 3.0, the Analytics SDK 3.x `libtetio`, and the apps
built on them: Eye Tracker Browser, `tobiictl`, Quartz Composer plug-ins…), as
if it were one of Tobii's network trackers (X2-60, TX300…):

```
[   0.880] tetserver: listening on TCP port 4455, announced as tetserverTPE01-100205013153._tetserver._tcp (Tobii SDK 3 sees TPE01-100205013153) at lo0 127.0.0.1, en0 192.168.1.79
```

Those trackers speak the very same TTP protocol, over TCP port 4455, and are
found over Bonjour as `_tetserver._tcp` services named `tetserver<serial>`
whose TXT record gives `pid`, `model`, `name`, `version`, `generation` and
`platformtype`. The bridge announces itself that way (model "Tobii Pro Spark"
or "Tobii Eye Tracker 5", firmware and platform from the tracker) and serves
each TCP connection as one more TTP channel, without translating anything. An
SDK 3 tracking session is a protocol negotiation (command 1000) and the start
of gaze stream `0x500` (1220); its gaze messages carry the X2-60's columns
plus more, which SDK 3 skips.

- SDK 3 never initialises the tracker (Eye Tracker Manager does), and an
  uninitialised tracker streams nothing but "eye not found". So, with the
  tetserver on, the bridge replays the built-in init (the daemon's, with the
  power-up default calibration) on a channel of its own at start-up, before
  announcing it: `tetserver: tracker initialised for SDK 3 (…)`. Checked with
  the original TobiiToQC2 Quartz Composer plug-in on a Pro Spark.
- Port 4455 listens on every interface, as a network tracker does, so other
  machines on the network can see and use the tracker too. `--no-tetserver`
  turns this off; `--tetserver-port <port>` moves it.
- The service points at a host name of the bridge's own,
  `tetserver-<serial>.local`, whose only addresses are the Mac's IPv4 ones
  (each interface announces its own, `127.0.0.1` locally). SDK 3 knows only
  IPv4: pointed at the Mac's own name, which also resolves to IPv6 addresses,
  it never connects and its `createEyeTracker` never returns, which freezes
  any app that calls it on the main thread (the original TobiiToQC2 plug-in
  does).
- Bonjour announcing is macOS only for now; elsewhere SDK 3 can connect by
  address (`tobiictl -t <host>`).
- A difference with the X2-60: when an eye is not found, the Spark gives
  (0, 0) as its 2D gaze point where the X2-60 gives (−1, −1). The validity
  codes are the same (4 = not found), so rely on those.
- Not checked yet: calibration and time synchronisation through SDK 3.

### Windows

The same program runs on Windows, where it serves the tracker on the named
pipe `\\.\pipe\TOBII-<serial>` instead. Tested on Windows 11 ARM64 (a
Parallels VM on a MacBook Pro M4), where Tobii's own driver package has no
ARM64 support, with Tobii's Stream Engine 4.25 (x64, emulated): discovery,
device info and gaze points at 33 Hz work.

1. Give the tracker's interface the in-box WinUSB driver: Device Manager ›
   "EyeChip" › Update driver › Browse my computer › Let me pick › Universal
   Serial Bus devices › "WinUsb Device". (On x64 Windows with Tobii's
   software installed, it already has WinUSB.)
2. Stop anything else holding that interface: the legacy "Tobii Service"
   (EyeX) opens it, and the bridge then fails with "Access denied".
3. Run `ttpbridge.exe` (no administrator rights needed) with `libunwind.dll`
   next to it if you built with llvm-mingw.
4. Use PETM **2.6.1** for Windows, patched so that its interface lists the
   tracker (same five bytes as on macOS, at another offset; checks the
   SHA-256, keeps `app.asar.orig`, `-Restore` undoes it; quit PETM first):

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\patch-petm.ps1
   ```

   Tested on Windows 11 ARM64: PETM applies its licence, initialises the
   tracker and shows gaze and the position guide.

Build from macOS with [llvm-mingw](https://github.com/mstorsjo/llvm-mingw):

```sh
rustup target add aarch64-pc-windows-gnullvm
PATH="$HOME/opt/llvm-mingw/bin:$PATH" \
CC_aarch64_pc_windows_gnullvm=aarch64-w64-mingw32-clang \
AR_aarch64_pc_windows_gnullvm=llvm-ar \
cargo build --release -p ttpbridge --features vendored --target aarch64-pc-windows-gnullvm
cp ~/opt/llvm-mingw/aarch64-w64-mingw32/bin/libunwind.dll target/aarch64-pc-windows-gnullvm/release/
```

(`x86_64-pc-windows-gnullvm` the same way for x64 Windows.) Tobii's
Windows transport uses one pipe per direction: the client creates its own
inbound pipe `client_TOBII-<serial>_<pid>_<n>`, connects to the server pipe
and sends `[u32 mode][its pipe name]`; the bridge then opens the client's
pipe to answer. PETM 2.7.2 for Windows ships Pro SDK 2.0 like the macOS
build, so it cannot see the bridge; the SDK-module swap used on macOS has
not been tried on Windows.

### Start at boot (macOS)

To start the bridge at boot, see
[`packaging/com.github.krazubu.tobii-et5-bridge.plist`](packaging/com.github.krazubu.tobii-et5-bridge.plist)
(not tested yet).

## Patch PETM

PETM's interface hides any tracker whose serial prefix is not on its list of
supported trackers, and `IS5FF` is on neither the built-in list nor the one it
downloads at start-up. The script checks the SHA-256 of `app.asar`, refuses
any version it does not know, keeps backups (`app.asar.orig`,
`SdkJsBinding.app.orig`) and can put them back with `--restore`.

**PETM 2.6.1**: one obsolete entry of the built-in list (`TX300`) becomes
`IS5FF`, five bytes, same length.

```sh
scripts/patch-petm.sh "/Applications/Tobii Pro Eye Tracker Manager.app"
```

**PETM 2.7.2**: the list has no obsolete entry left, so the interface's
filter function is rewritten at the same length to accept `IS5FF-*` on top
of the list (every other tracker is filtered exactly as before). This version
also ships Tobii Pro SDK 2.0, which only talks PRP to Tobii's Platform Runtime
and dropped TTP altogether, so it cannot see `ttpbridge`. The script puts
PETM 2.6.1's SDK module (`stage/SdkJsBinding.app`, Pro SDK 1.11, which still
talks TTP) in its place, copied from your own 2.6.1 install. The messages
exchanged between the interface and the SDK module are the same in both
versions.

```sh
scripts/patch-petm.sh "/Applications/Tobii Pro Eye Tracker Manager.app" --sdk-from "/path/to/PETM 2.6.1.app"
```

Undo with `scripts/patch-petm.sh --restore <app>`.

Quit PETM before running it. On recent macOS, the terminal you run it from
needs the *App Management* permission (System Settings › Privacy &
Security) to modify an app in `/Applications`; otherwise `cp` fails with
"Operation not permitted" and nothing is changed. These patches only change
which products the interface displays and which SDK module it starts;
licensing is untouched: PETM applies its own licence to the tracker, and that
succeeds.

## How it works

`ttpbridge` reproduces the client side of Tobii's USB service for Linux
(`tobiiusbserviced` 2.1.5), recovered by static analysis of that service and
of Tobii's macOS Platform Runtime, and checked against USB captures of the
Windows software (the ones the rest of this repository is built on).

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
- `tools/se_probe.c`: loads Tobii's Stream Engine DLL on Windows and checks
  discovery, device info and gaze through the bridge (build it with
  `x86_64-w64-mingw32-clang` for the x64 DLL).
- `tools/research_probe.c`: asks `libtobii_research` (from your own PETM
  install) what it sees: discovery, identity, gaze. Build instructions are in
  the file. Outside PETM, gaze is refused for lack of a licence; that is
  expected.

## Licence

MIT. Tobii, Eye Tracker 5 and Eye Tracker Manager are trademarks of Tobii AB.
