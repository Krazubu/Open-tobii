# open-tobii on STM32F411 — autonomous gaze reader

A standalone firmware that lets an **STM32F411** read the **gaze point** from a
Tobii tracker (Eye Tracker 5 `2104:0313`, Pro Spark `2104:031C`) over USB,
**with no PC** — the tracker plugs straight into the MCU — and optionally shows
the point on a small OLED.

> Status: **works on hardware.** A Black Pill F411 enumerates an Eye Tracker 5
> at Full-Speed, replays the init, reads the gaze stream and draws the gaze
> point on a 128×64 OLED. The USB host is [TinyUSB](https://github.com/hathach/tinyusb);
> the app is [`tobii_gaze/`](tobii_gaze). Button-triggered calibration is not
> wired in yet (see [Milestones](#milestones)).

## Why a new firmware (and not "tobii-linux on the MCU")

`tobii-linux` is a Linux/macOS/Android daemon built on libusb and ONNX. None of
that runs on a 100 MHz Cortex-M4 with 128 KB of RAM. This firmware reimplements
only the **minimal gaze path**:

```
[Tobii] --USB (host)--> [STM32F411] --(optional)--> OLED  (gaze dot)
```

- **USB host** (STM32 OTG_FS) enumerates the tracker and claims its vendor
  interface.
- The captured **init sequence** is replayed to bring the device up and start
  the gaze stream.
- The **gaze stream (0x500)** is read and the **combined gaze point** decoded.
- No image stream, no head pose, no ONNX — those need a PC.

The protocol is the same TTP-over-USB this repo already speaks; the decode logic
is ported from [`crates/tobii-proto`](../../crates/tobii-proto) (the source of
truth), see [`src/tobii_proto.c`](src/tobii_proto.c).

## Hardware

- **STM32F411** (e.g. WeAct "Black Pill" F411CEU6): Cortex-M4F @ 100 MHz,
  128 KB SRAM, USB **OTG_FS** with **host** mode.
- **USB OTG**: the tracker connects to the OTG_FS pins (PA11 D−, PA12 D+) via a
  USB-OTG adapter. A **Y-OTG adapter that injects external 5 V** is recommended
  — the tracker draws more than a bus-powered board should source.
- **OLED (optional)**: SSD1306/SSD1309, 128×64 by default (tested on a 2.42"
  panel at I²C address `0x3C`), on I²C1 — PB6 SCL, PB7 SDA (see
  [`include/config.h`](include/config.h)).

### ⚠️ Full-Speed caveat (the main risk)

The F411's USB host is **Full-Speed only (12 Mbps)**; the Tobii trackers are
high-speed devices. **Verified on hardware: the Eye Tracker 5 falls back to
Full-Speed and streams gaze correctly.** The image stream would not fit FS,
which is fine: only gaze is read.

One thing that matters at FS: EP `0x83` must be read **continuously**, with a
read always pending (re-submitted from its own completion callback) and
started **before** the init replay, so the device's responses are drained
while the init is written to EP `0x05` — as the daemon's
`replay_init_packets` does. Writing the whole init first and reading after
leaves the stream stalled.

## Protocol (what the firmware does on the wire)

All multi-byte TTP fields are **big-endian**; the USB bulk framing words are
**little-endian**. Constants are in [`include/tobii_proto.h`](include/tobii_proto.h).

1. **Enumerate & configure**: find VID `0x2104`, a known PID; set configuration
   1; claim interface 0 (vendor class, bulk EP `0x05` OUT / `0x83` IN).
2. **Vendor control setup** (`bmRequestType` `0x41`/`0xC1`):
   - req **48**: SCP metadata (platform/firmware) — optional, nice for logging.
   - req **65**: open the TTP channel (this also starts the EP `0x83` stream).
   - req **66**: close it (Windows-style stop) on shutdown.
3. **Replay the init** on EP `0x05`: the captured command sequence
   (`crates/tobii-usb/init_packets_ep.txt`, now the tracker's built-in default
   calibration — ~42 packets). Each piece is `u32 LE channel`, `u32 LE length`,
   then the TTP message. See [`tools/gen_init_packets.py`](tools/gen_init_packets.py).
4. **Read EP `0x83`**: frames are `u32 LE channel mask`, `u32 LE total length`,
   then one TTP message. Keep the ones whose `marker` (BE @ offset 8) is
   `0x53` (stream) and `id` (@ offset 20) is `0x500` (gaze).
5. **Decode the gaze point**: the 0x500 payload is `00 00` then keyed TLV
   entries; the **combined gaze point is key `0x1c`** (validity key `0x1b`), a
   2-D point in the display frame (normalised ~[0,1]). `tobii_proto.c`
   extracts it; map it to OLED pixels.

## Optional display — off by default

The OLED feedback is a **compile-time option, disabled by default** so the
firmware pushed to the repo builds without it (and without the display driver):

```c
// include/config.h
#define OPEN_TOBII_ENABLE_DISPLAY 0   // 1 to build the OLED gaze dot
```

With it off, the firmware still enumerates, streams and decodes gaze; the
board LED is the only feedback (on once the tracker is enumerated, blinking at
~2 Hz while valid gaze arrives). With it on (`make OLED=1`), the OLED shows the
bring-up stage as bars — 1 enumerated, 2 pipes + vendor control, 3 init
replayed, 4 first EP `0x83` frame — then the moving gaze dot.

## Calibration (button)

A press on the board's user button launches a calibration (`main.c` →
`tobii_calib_run`): the device runs the TTP calibration sequence — START,
CLEAR, COLLECT at each stimulus point while the user fixates it, COMPUTE, STOP
(ids ported from `crates/tobii-proto/src/calibration.rs`; the gaze point of a
collect is `x, y` as 32.32 fixed plus the eye mask). The default is a 5-point
set (centre + corners) with a per-point fixation delay.

Two things to settle on hardware:

- **Stimulus surface.** A real multi-point calibration needs the user to fixate
  points across their actual field of view; the 0.42" OLED is too small to be
  that surface. Options: show the points on it anyway (crude), drive an external
  display, or do a single centre point (a coarse re-centre). This is a design
  choice, left configurable (`cal_points` in `main.c`).
- **Apply step.** Whether the device applies on COMPUTE or needs the computed
  blob READ (1100) then WRITTEN back (1110) is marked TODO in `tobii_calib.c` —
  validate against `crates/tobiid/src/calibration.rs`.

## Build & flash

Needs `arm-none-eabi-gcc`, `make`, `git` and `python3`. TinyUSB and the STM32F4
CMSIS/HAL are not vendored: a script fetches them, at the commits verified on
hardware, into `usb/` (ignored by git).

```sh
cd firmware/stm32f411
python3 tools/gen_init_packets.py ../../crates/tobii-usb/init_packets_ep.txt > include/tobii_init.h
tools/setup_tinyusb.sh                # once
cd tobii_gaze
make BOARD=stm32f411blackpill         # LED only
make BOARD=stm32f411blackpill OLED=1  # with the OLED gaze dot
```

The image is `_build/stm32f411blackpill/tobii_gaze.bin` (~23 KB flash, ~11 KB
RAM). Flash it over DFU (hold BOOT0, reset, connect the board to the computer),
e.g. with `dfu-util -a 0 -s 0x08000000:leave -D tobii_gaze.bin` or
STM32CubeProgrammer (`STM32_Programmer_CLI -c port=usb1 -w tobii_gaze.bin 0x08000000 -v -g 0x08000000`).
The board has a single USB port: unplug the computer, then connect the tracker
through the OTG adapter with external power.

[`bringup/`](bringup) is a bare-metal OLED demo (no USB) that checks the wiring
and the flash cycle. The portable layer (`tobii_proto.c`) has no hardware
dependency and can be built and tested on the host.

The older [`src/main.c`](src/main.c) / [`include/tobii_host.h`](include/tobii_host.h)
are the HAL-based scaffold the calibration flow was sketched in; they are not
part of the build.

## Milestones

1. **[done]** Portable protocol layer + design.
2. **[done]** USB host (TinyUSB, OTG_FS), I²C1 OLED driver.
3. **[done]** Enumerate the tracker, vendor control setup — the FS host link works.
4. **[done]** Replay the init and read EP `0x83`.
5. **[done]** Decode `0x500` — **gaze works at Full-Speed.**
6. **[done]** The OLED gaze dot (behind `OPEN_TOBII_ENABLE_DISPLAY`).
7. Button → `tobii_calib_run` in the TinyUSB app: validate the calibration
   sequence and the apply step on hardware; settle the stimulus surface.
8. Port `tlv.rs::keyed_fields` exactly into `tobii_decode_gaze` (the current
   walk is a heuristic that works on the ET5's frames) and add a host test.
