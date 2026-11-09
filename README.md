# open-tobii

[![ci](https://github.com/Krazubu/Open-tobii/actions/workflows/ci.yml/badge.svg)](https://github.com/Krazubu/Open-tobii/actions/workflows/ci.yml)
[![latest release](https://img.shields.io/github/v/release/Krazubu/Open-tobii?label=release)](https://github.com/Krazubu/Open-tobii/releases/latest)
[![licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)

![A Tobii Eye Tracker 5 on a mount at the bottom edge of a monitor](docs/images/tobii-eye-tracker-5.jpg)
<sub>Photo: [jwanga](https://www.printables.com/@jwanga_350626), from the Printables model
[“Tobii Eye Tracker mount for curved and flat monitors”](https://www.printables.com/model/663142-tobii-eye-tracker-mount-for-curved-and-flat-monito),
[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/), cropped.</sub>

A Linux driver stack for the **Tobii Eye Tracker 5** (USB `2104:0313`), built
by reverse-engineering the device's USB protocol. It gives you gaze, user
presence and gaze-independent 6-DOF head pose — all at 33 Hz, all at the same
time — plus calibration, through a daemon, a drop-in `libtobii.so` for Tobii's
Stream Engine 4.25 C API, and a few thin clients.

Tobii ships no Linux support for this device. Everything here was recovered
from USB captures of the Windows Stream Engine.

> Status: works daily on the author's desk. Independently reimplemented from
> USB captures; not affiliated with or endorsed by Tobii.

**Install** the latest [release](https://github.com/Krazubu/Open-tobii/releases/latest)
— `.deb`, `.rpm` or tarball — then plug the tracker in:

```sh
sudo dnf install ./open-tobii-*.x86_64.rpm   # or: sudo apt install ./open-tobii_*_amd64.deb
tobii-calibrate                               # calibrate once; it starts the daemon itself
```

Calibrating is once per user, and it is what puts gaze where you look:
games and applications reading gaze through `libtobii.so` need it, and so
does gaze-switcher. Head pose (OpenTrack) and `tobii-gaze-keys`' arrows work
without it. Until you calibrate, a notification at login offers it.

Gaze to arrow keys and head pose to OpenTrack are opt-in, one `systemctl
--user enable` each — [INSTALL.md §3](INSTALL.md) says what each one needs
(the key injection wants the `input` group, and says what that costs).

[Full install and setup](INSTALL.md) · [what the C API answers](docs/stream-engine-api.md)
· [changelog](CHANGELOG.md) · [contributing](CONTRIBUTING.md)

## What it does

| Signal | Rate | Source |
|---|---|---|
| Gaze point + validity | 33 Hz | the device's processed `0x500` stream |
| Per-eye gaze origin, eye position, user position guide, gaze data | 33 Hz | the same stream |
| Raw gaze: the Stream Engine's own record of each gaze frame | 33 Hz | the same stream, every value as sent |
| User presence | on change | the `0x504` stream |
| Head pose, 6 DOF | 33 Hz | face landmarks on the device's own IR frames |
| IR camera image, 280×280 | 33 Hz | the `0x50e` stream |
| The Stream Engine's custom stream 0 (no data on the ET5) | ~133 Hz as the ET5 sends it, while subscribed | the `0x1770` stream, which the daemon starts for its subscribers |
| Device info, track box, display area, the stream catalogue and each stream's columns | on request | the device's own answers |
| Notifications: display area, calibration, pause, faults, warnings | on change | the device's own, and the daemon's for calibration and pause |
| Calibration | on demand | `tobii-calibrate`, saved per user |

Everything the Stream Engine reports from the tracker's data is reproduced
bit for bit: replaying a captured Windows session through the decoder gives
exactly the gaze points and gaze origins the Stream Engine delivered for it
(5 284 of 5 284 frames, error 0; `tobii5-init-replay compare-dll`). The head
pose is computed, not decoded, on Windows too: libtobii's position follows the
Stream Engine's to a median of a few millimetres, while its yaw and pitch, by
design, follow the head's true turn, which the Stream Engine under-reads: yaw
turns 1.3 to 1.4 times as far as the Stream Engine's and pitch 0.7 to 0.9
times as far (Architecture, *Head pose*).

Against a laser fixed to the head (one person, three sessions, Tobii's
runtime 4.72 in a Windows 11 virtual machine), libtobii's yaw moves 0.99 to
1.01 times as far as the head turned and the Stream Engine's 0.72 to 0.79,
mostly through a gain fitted to that measurement; without the gain,
libtobii's scale also held steadier from one session to the next. That is
where it measured better. The Stream Engine's pose is quieter at rest
(libtobii's yaw is up to 1.8 times as noisy), comes out sooner after each
image (about 2 ms on Windows, about 11 ms here, measured separately) and
keeps the face at large turns a little more often (Architecture, *Head
pose*; `tools/headpose/README.md`, "Laser truth").

The head pose is **gaze-independent**: turning your eyes does not move it.

### The interesting part

The device multiplexes a second stream, `0x50e` `primary_camera_image`, on the
same bulk endpoint as gaze — a 280×280 8-bit IR frame of your face at 33 Hz.
The Windows driver subscribes to it with command 1220 right after init; nothing
in the documented API hints it exists. It is what the Stream Engine's head pose
is made of: no head pose crosses the USB, and Tobii's runtime computes one on
the host from every frame.

That matters because the eye-position fields in the gaze stream **cannot** give
you a signed head yaw. Both eyes are reconstructed at equal range from the
camera, so the per-eye depth is synthetic and any "yaw" computed from it is
lateral head position in disguise. Feeding the IR frames to a face-landmark
model instead gives a real signed yaw at ~2° resolution, and it costs about
6 ms of CPU per frame. That conclusion came out of a field-by-field study of
11 800 recorded frames, cross-checked against head poses read off the IR images
themselves; the tooling for it is in `tobii-tools`.

## Quick start

Each [release](https://github.com/Krazubu/Open-tobii/releases) carries
ready-made packages for x86_64: a `.deb` (Debian 12, Ubuntu 22.04 and newer),
an `.rpm` (Fedora, openSUSE) and a tar.gz with an `install.sh` for any other
distribution with systemd and glibc 2.35 or newer (INSTALL.md §0). They install the
daemon, `libtobii.so` and its headers, the clients, the udev rules and the
user units, and put "Tobii Calibration" among the desktop's applications.

The packages are x86_64 only; the driver is not. On another architecture,
build it from source (below) — **aarch64 runs it**, confirmed on a Raspberry
Pi 4 under Ubuntu Server 24.04
([#2](https://github.com/Skyggedans/tobii-linux/issues/2), thanks Krazubu),
where the ONNX Runtime the head pose links against downloads an aarch64
build of its own.

From source:

```bash
cargo build --release --workspace     # or: make build
make install                          # binaries + libtobii.so + headers + udev rules + desktop entries + user units
make enable                           # start the daemon now
tobii-calibrate                       # calibrate for your eyes (once)
```

Then either link against `libtobii.so`, or run a client:

```bash
tobii-opentrack                       # head pose -> OpenTrack UDP 127.0.0.1:4242
tobii-gaze-keys                       # screen edge + hold Super -> arrow keys; double wink -> Super+arrow
```

`tobii-gaze-keys` needs the `input` group (`sudo usermod -aG input "$USER"`,
then a reboot), which lets every program you run read all keyboards:
INSTALL.md §3 weighs it. `tobii-opentrack` sends the Stream Engine's head
pose, the one `libtobii.so` delivers, each axis as OpenTrack's own
`tracker-tobii` plugin hands it over, so one OpenTrack profile fits either;
OpenTrack centres it, at the first pose and on its *Center* shortcut. Both
clients also come as user services: `systemctl --user start tobii-opentrack`
while you play (`enable --now` for every login), and
`tobii-gaze-keys.service` likewise.

`libtobii.so` is API-compatible with Tobii's Stream Engine 4.25: it exports
the same 199 functions as `tobii_stream_engine.dll` 4.25.0.3, with the DLL's
signatures (`tools/abi/dll_abi.py headers` checks every prototype), error
codes, enum values and struct layouts (the size of `tobii_stream_type_t` is
inferred, `tobii_hardware_configuration_t` keeps 4.1.0.3's two-entry layout,
and what its fields hold is provisional, as the headers say), so a program
written for Stream Engine 4.1 builds against its headers and runs unchanged
unless it calls the undocumented `tobii_multiple_faces_position_subscribe`
or `_unsubscribe`, which 4.25 renamed `tobii_multiple_faces_subscribe` and
`_unsubscribe`; Architecture says what each function does with an ET5.
OpenTrack's own Tobii tracker, the `tracker-tobii` plugin, is one once it is
brought to Stream Engine 4: built on Linux from the `tracker-tobii-linux`
branch of the
[OpenTrack fork](https://github.com/Skyggedans/opentrack/tree/tracker-tobii-linux),
it reads the head pose through libtobii, with no UDP bridge. The fork lets
the plugin build on Linux and against Stream Engine 4, reconnect after a
daemon restart and hold its last pose while the face is lost (INSTALL.md
§8).

Full instructions, tuning knobs and troubleshooting are in
**[INSTALL.md](INSTALL.md)**.

Requirements: a stable Rust toolchain, libusb, and the tracker plugged in, on
x86_64 or aarch64. The face models and the init capture are embedded at build
time, so the installed binaries need no runtime data files.

## Architecture

```
libtobii.so ─────┐
tobii-opentrack ─┤
tobii-gaze-keys ─┼── unix socket ──► tobiid ──USB──► Tobii ET5
tobii-calibrate ─┘                      │
                              EP 0x83: gaze 0x500, presence 0x504, IR image 0x50e,
                                       custom stream 0x1770 while a client holds it
```

`tobiid` is the only process that claims the device; everyone else is a client,
so several consumers can read gaze and head pose at once.

`libtobii.so` exports **all 199 entry points** of Tobii's
`tobii_stream_engine.dll` 4.25.0.3, and no others, with its signatures, error
numbering and struct layouts but one (`tobii_hardware_configuration_t` keeps
4.1.0.3's two entries and 2472 bytes, where 4.25.0.3's holds four and is
inferred to be 3960) — so any Stream Engine client links and runs. The ABI
was recovered from the two DLLs themselves — 4.1.0.3's in `tools/abi/`, what
4.25.0.3 adds or changes in the comments of `crates/tobii-ffi/` (`src/` and
`include/tobii/`) — and the archived 4.1.0 reference. Three struct layouts
follow the DLLs rather than that reference:
`tobii_gaze_data_eye_t` carries a fifth validity the 4.1 documentation omits
(80 bytes per eye, not 76), `tobii_gaze_raw_t` is 4.25's 248-byte record
instead of 4.1's 232, and `tobii_wearable_advanced_data_t` is 4.25's 216-byte
one instead of 4.1's 208 — the changelog says why.

Of those 199: most are served by the daemon from the tracker's own data, some
are answered locally because the Stream Engine answers them locally too,
**five of the 4.1 surface remain unimplemented refusals** — reading and
listing the illumination modes, arming and disarming remote wake (the tracker
refuses all four itself, so the answer is right) and the configuration key,
which needs a realm no entry point can open — and twenty-nine of the
forty-six entry points 4.25 adds are refusals too (Limitations says which,
and why).

Where an answer cannot be the Stream Engine's it diverges deliberately: the
combined-gaze HID track box is the tracker's measured one rather than the
constants the DLL holds, both custom-command payloads are bounded where the
DLL bounds neither, seven entry points keep the DLL's licence refusal, and
seven of the thirteen notification types are delivered.

**[docs/stream-engine-api.md](docs/stream-engine-api.md) says what the entry
points answer and why**, by name or by family, divergence by divergence, the
48 names 4.25 added included. `tools/abi/README.md` carries 4.1.0.3's
derivation, with the DLL addresses each claim was read at, and the comments in
`crates/tobii-ffi/` (`src/` and `include/tobii/`) carry that of what 4.25.0.3
adds or changes, with the DLL addresses where they give them.

The headers are in `crates/tobii-ffi/include/tobii/` (installed to
`/usr/include/tobii/` by the packages, `/usr/local/include/tobii/` by `make
install` and the tarball); OpenTrack's `tracker-tobii` plugin builds against
them from the `tracker-tobii-linux` branch of the
[OpenTrack fork](https://github.com/Skyggedans/opentrack/tree/tracker-tobii-linux),
which brings it to Stream Engine 4 and lets it configure on Linux
(INSTALL.md §8).

### Workspace

Eleven crates, split so the driver never compiles the research tooling and
`libtobii.so` links neither ONNX Runtime nor the embedded assets (it is 0.45 MB
rather than 20 MB).

| Crate | Holds | Heavy deps |
|---|---|---|
| `tobii-proto` | wire formats: framing, TLV, commands, the gaze/presence/image/custom streams, device facts, the capture log | none |
| `tobii-pose` | face landmarks, face detection and the Stream Engine's head pose; owns the models | `ort` |
| `tobii-usb` | USB transport and the live `0x83` engine; owns the init capture | `rusb` |
| `tobii-ipc` | the daemon protocol and its deadlines, the display geometry and the host clock | none (`libc` only) |
| `tobii-calib` | the calibration blob format and the per-user store | none (std only) |
| `tobii-log` | shared `tracing` setup | — |
| `tobiid` | the daemon | — |
| `tobii-ffi` | `libtobii.so` (cdylib) | — |
| `tobii-clients` | `tobii-opentrack`, `tobii-gaze-keys` | — |
| `tobii-calibrate` | the calibration window | `winit`, `softbuffer` |
| `tobii-tools` | `tobii5-init-replay`: log analysis, UVC camera, diagnostics | all of the above |

`make check` runs fmt, clippy with `-D warnings`, the tests and rustdoc.
`make verify-abi` asserts `libtobii.so` exports exactly the 199 symbols in
`crates/tobii-ffi/abi-symbols.txt`, then compiles and runs
`crates/tobii-ffi/abi-smoke.c` against the headers: it takes the address of
every symbol through them and checks versions, rejections and struct layouts.

## Research tooling

`tobii5-init-replay` is built but not installed — none of it is needed to run
the driver. It is the bench the protocol was worked out on, and it is still the
regression harness:

```bash
target/release/tobii5-init-replay image83 --secs 10 --pose   # live A/B of both streams
target/release/tobii5-init-replay probe                      # 0x83 vs UVC concurrency
target/release/tobii5-init-replay image83-replay log.bin --csv out.csv
target/release/tobii5-init-replay analyze-log log.bin        # blind field scan
target/release/tobii5-init-replay head-axes yaw:a.bin roll:b.bin
target/release/tobii5-init-replay compare-dll session.bin session.jsonl  # decoder vs the DLL
target/release/tobii5-init-replay ipc-probe --secs 5         # ask the running daemon everything
target/release/tobii5-init-replay stream-probe 0x501 --log p.bin  # start one catalogue stream, log it (tobiid stopped)
tools/abi/dll_abi.py headers                                 # headers vs tobii_stream_engine.dll
tools/headpose/fit.py --session LOG JSONL FITS ...           # refit the head pose constants
tools/headpose/evaluate.py --session LOG JSONL FITS ...      # head pose (no gains) vs the DLL's
tools/headpose/truth.py --session LOG JSONL FITS ...         # yaw and pitch vs laser-measured turns
```

`tools/headpose/README.md` has the head pose's refit and acceptance procedure.

## Extras

- **[gaze-switcher/](gaze-switcher/)** — a GNOME Shell extension that lets you
  pick a window in the Alt+Tab switcher by looking at it.
- **[tools/splice_calibration.py](tools/splice_calibration.py)** — injects a
  calibration blob into a captured init sequence (superseded by
  `tobii-calibrate`, kept for research).

## Limitations

- One process owns the device. `tobiid` arbitrates; run everything else as a
  client.
- The UVC camera interface and the `0x83` streams are mutually exclusive in
  firmware. Nothing in the driver uses UVC any more, but keep the shipped
  `99-tobii-no-uvcvideo.rules` in place so nothing else grabs it.
- Head pose needs a face in frame. Without one, or with one at the edge of
  the frame, `libtobii.so` marks its head pose invalid, as the Stream Engine
  does; `tobii-opentrack` then sends nothing, and OpenTrack holds the last
  pose.
- Until you run `tobii-calibrate`, the calibration in use is the tracker's
  own power-up default (id 0), which `init_packets_ep.txt` uploads: coarse,
  made for no one in particular, so gaze lands roughly. The display area is
  the one that capture carries, 633.6 × 334.3 mm (a 28.2" screen, wider than
  16:9), whatever your monitor is. The tracker scales gaze to that area, so
  on any other screen gaze misses the edges: looking at the left and right
  edges of a 24" 16:9 monitor reads about 0.08 and 0.92, of a 49" 32:9 one
  about −0.44 and 1.44. `tobii-calibrate` sets your area first (you line two
  ticks up with the marks on the tracker), and the daemon keeps whatever
  display area is set (`~/.config/tobii/display-area`). A calibration made
  with `--no-display-setup`, or before 0.4.1 (whose display setup never
  succeeded), was made on whatever area the tracker had, usually the
  capture's: calibrate again.
- **Only the Eye Tracker 5** (`2104:0313`). A 5L, 4C or anything else answers
  a different stream catalogue and a different generation, and none has been
  seen; see [CONTRIBUTING.md](CONTRIBUTING.md) for what a capture of one would
  need to contain.
- **Five of the 4.1 C entry points are unimplemented refusals**: reading and
  listing the illumination modes and arming and disarming remote wake — for
  which `TOBII_ERROR_NOT_SUPPORTED` is nevertheless the right answer, the
  tracker refusing all four itself — and the configuration key, which needs a
  realm no entry point can open.
- **Twenty-nine of the forty-six entry points 4.25 adds are refusals too**:
  streams, properties and commands the DLL itself serves only behind the
  Tobii service, for hardware an Eye Tracker 5 is not, or, for absolute eye
  openness, gates on a licence a consumer tracker lacks — the wearable and
  camera-image streams, the facial landmarks, the device log, absolute eye
  openness, the algorithm-debug stream, the remote cameras, reading and
  writing the extended lens configuration, the ambient light level (whose
  handler in 4.25's own tracker module is a stub that refuses it too),
  proximity data collection, `tobii_force_frame_sequence` and
  `tobii_set_algo_box_mode`. Each checks its arguments, and none invents data.
  The other seventeen give real answers: the fault and warning lists and their
  streams, 4.25's presence stream, the responsive gaze point (the tracker's
  own unfiltered point, key `0x20`), the output-frequency stream, whether the
  extended lens configuration can be written (never), and the four signing
  calls, which need no device at all.
  [docs/stream-engine-api.md](docs/stream-engine-api.md#what-425-added) says
  what each answers and where that differs from the DLL.
- The calibration sequence mirrors the one captured from Windows; 3-D and
  per-eye calibration are not supported, as the Stream Engine answers for
  an ET5 (per-eye calls check their own arguments first, as 4.25 does, and
  are then `TOBII_ERROR_CALIBRATION_NOT_STARTED` outside the handle's own
  session, as there). Discarding a
  2-D point and pausing the tracker send the commands the DLL sends for them
  (1080, and 3100 with 1), which were never captured.

## License

MIT ([LICENSE](LICENSE)), except for the MediaPipe material that `tobii-pose`
embeds, and that `tobiid` and `tobii5-init-replay` carry with it: the
face-landmark and face-detection models
(`crates/tobii-pose/models/face_landmarks.onnx` and
`blaze_face_short_range.onnx`, converted from TensorFlow Lite to ONNX) and the
canonical face mesh in `crates/tobii-pose/src/canonical.rs`. Those are
Apache-2.0. The licence text, the notice and where they come from are in
[crates/tobii-pose/models/](crates/tobii-pose/models/) (`LICENSE`, `NOTICE`,
`README.md`).
