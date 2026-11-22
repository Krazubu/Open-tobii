# Open-tobii — fork changelog

Everything this fork added on top of [Tobii's `tobii-linux`](https://github.com/Skyggedans/tobii-linux),
starting from the **0.4** base. The upstream project's own history is in
[`CHANGELOG.md`](CHANGELOG.md); this file lists only the fork's work.

Upstream was a Linux-only daemon for the **Tobii Eye Tracker 5**. This fork adds
macOS and Android support, a second tracker (the **Tobii Pro Spark**), a
companion bridge for Windows/macOS, multi-OS release binaries, and a rebrand to
**open-tobii** — then re-synced with upstream up to v0.6.0.

---

## New platform: macOS

- **Apple Silicon (`aarch64-apple-darwin`)** and **Intel 64-bit
  (`x86_64-apple-darwin`)** builds of the daemon (`tobiid`) and the calibration
  tool (`tobii-calibrate`). The USB transport, the TTP engine and the winit
  calibration UI all run on macOS.
- **Intel 32-bit (`i686-apple-darwin`)**, on the `macos-i386` branch: `tobiid`
  and a drop-in `libtobii` so a 32-bit C program on macOS 10.14 can read gaze
  (tier-3 target; needs a nightly toolchain and the macOS 10.13 SDK).
- **Gaze-only builds** where ONNX Runtime ships no prebuilt binary: head-pose
  is made optional so the daemon still streams gaze and calibrates. `tobiid`
  gained a `head-pose` feature (on by default).
- **Head pose without ONNX Runtime**: `tobii-pose` can run its two models on
  [tract](https://github.com/sonos/tract) (pure Rust) instead, the same
  outputs to about 10⁻³ px but some 2.5x slower. The Intel macOS release of
  `tobiid` uses it (`--features head-pose-tract`), where it was gaze-only.
  tract 0.23 needs Rust 1.91; the builds without it keep the 1.88 minimum.
- Cross-platform gating of the Linux-only paths (Wayland/X11 window naming and
  startup tokens, `POLLRDHUP`) so the same tree builds on macOS.

## New platform: Android

- A **proof-of-concept Android app** and the `tobii-android` crate: the Eye
  Tracker 5's gaze stream inside an app's own process, **no root**, over the
  file descriptor `android.hardware.usb.UsbManager` hands out.
- An on-device **calibration screen** that runs the same captured command
  sequence as the daemon, directly against the engine.
- Native library built for **`arm64-v8a` and `armeabi-v7a`**, shipped as a
  universal APK.
- **Head pose**: the `arm64-v8a` library links ONNX Runtime (`ort`'s
  prebuilt binary, with the NDK 27's `libc++_shared.so` beside it), the
  `armeabi-v7a` one runs the models on tract; the app gains a *Head pose*
  button that turns the inference on. Both checked with an Eye Tracker 5 on
  a Retroid Pocket 5 (Android 13).

## New hardware: Tobii Pro Spark

- **Recognition** of the Pro Spark (USB `2104:031C`, platform `IS5LPROENTRY` /
  codename *Castor* / firmware family `TPSP1`) via a new model registry
  (`crates/tobii-usb/src/models.rs`); device discovery now matches a table of
  models instead of a single hardcoded id.
- **Driving** the Spark with the shared Eye Tracker 5 init: verified live on
  hardware — the device comes up, streams **gaze at 60 Hz** and a head-pose
  image stream, and a full **14-point calibration** round-trips and saves.
- Reverse-engineering notes and the known protocol gaps in
  [`docs/spark.md`](docs/spark.md).

## The bridge: `ttpbridge` (formerly `tobii-et5-bridge`)

Now in this repository, as [`crates/tobii-bridge`](crates/tobii-bridge), with
its history (it started as the separate
[`tobii-et5-bridge`](https://github.com/Krazubu/tobii-et5-bridge) repo). It
serves a tracker to Tobii's own Eye Tracker Manager and was used to capture the
Spark's protocol. Since the move it uses `tobii-proto` instead of its own copy
of the wire modules, and `tobii-usb`'s init replay — the one that carries the
tracker's power-up default calibration — instead of the older 202-piece replay
it shipped.

- **Windows** support (named-pipe transport) for **ARM64, x64 and x86**, plus
  **macOS** (Apple Silicon and Intel 64-bit).
- Pro Spark recognition and an optional **channel-traffic capture** of the
  init/stream/calibration sequence an SDK session drives.
- **Tobii SDK 3 support** (`TobiiSDK.framework`, the Analytics SDK's
  `libtetio`, Eye Tracker Browser, `tobiictl`): the bridge also serves the
  tracker as a network "tetserver" — TTP over TCP port 4455, announced over
  Bonjour as `_tetserver._tcp` — the way the X2-60 and other older trackers
  do. Those trackers speak the same TTP, so nothing is translated: a gaze
  session through SDK 3 was checked on a Pro Spark (`--no-tetserver` turns it
  off). The service is announced on a host name of its own with IPv4
  addresses only, since SDK 3 hangs on the IPv6 addresses of the Mac's name.
  As SDK 3 never initialises the tracker, the bridge replays the built-in init
  at start-up; gaze from a Spark then reaches SDK 3 apps (checked in Quartz
  Composer with TobiiToQC2).

## Packaging, CI and releases

- **Multi-OS release workflows** (GitHub Actions, on a `v*` tag): the daemon and
  tools for Linux (deb/rpm/tarball), macOS (Apple Silicon + Intel) and the
  Android APK, with a combined `SHA256SUMS`; Windows and macOS binaries for the
  bridge.
- Published releases: **Open-tobii v0.4.2** (first multi-OS build + Spark
  recognition), **v0.4.3** (full open-tobii package identity), and
  **tobii-et5-bridge v0.1.0** (Windows + macOS).
- **Rebrand `tobii-linux` → `open-tobii`**: the installable package identity
  (apt/dnf name, AUR recipe `open-tobii-bin`, systemd units, install scripts,
  paths) and every release file and title; project URLs point at
  `github.com/Krazubu/Open-tobii`.

## Sync with upstream (v0.5.0 + v0.6.0)

Merged Tobii upstream's later work into the fork, keeping all of the above:

- **Built-in default calibration at power-up** — the tracker starts on its own
  default rather than the capture author's personal calibration blob (a privacy
  fix, and a better default for the Pro Spark).
- A **responsive (low-latency) gaze point**, **power-save** robustness in the
  daemon, and **`tobii-calibrate`** reminders, `--status`, and startup-token
  handling.
- The workspace version follows upstream's **0.6.0**.

---

*Not affiliated with or endorsed by Tobii.*
