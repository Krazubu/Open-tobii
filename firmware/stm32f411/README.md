# open-tobii on STM32F411 — autonomous gaze reader

A standalone firmware that lets an **STM32F411** read the **gaze point** from a
Tobii tracker (Eye Tracker 5 `2104:0313`, Pro Spark `2104:031C`) over USB,
**with no PC** — the tracker plugs straight into the MCU — and optionally shows
the point on a small OLED.

> Status: **scaffold / work in progress.** This directory holds the design, the
> portable protocol layer (`src/tobii_proto.c`, no hardware dependency) and the
> glue skeleton. The USB-Host middleware integration and the on-hardware
> bring-up are the next steps (see [Milestones](#milestones)).

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
- **OLED (optional)**: SSD1306 72×40 (the 0.42" panel) or any SSD1306, on I²C1
  (PB6 SCL, PB7 SDA by default — see [`include/config.h`](include/config.h)).

### ⚠️ Full-Speed caveat (the main risk)

The F411's USB host is **Full-Speed only (12 Mbps)**. The Tobii trackers are
**high-speed** devices; on a FS host they must fall back to Full-Speed, but
whether their firmware *streams gaze correctly at FS* is **unverified** — this is
the experiment. Gaze is low-bandwidth, so it is plausible. If it fails, the path
is an **STM32H7 + ULPI PHY** (true high-speed). The image stream never fits FS,
which is fine: we only want gaze.

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

With it off, the firmware still enumerates, streams and decodes gaze (reported
over the debug UART / USB-CDC console); with it on, it also draws the dot.

## Build

Not wired to a build system yet. Intended: an STM32CubeIDE / CubeMX project (or
a Makefile + `arm-none-eabi-gcc` + STM32CubeF4 HAL and USB_HOST middleware) that
compiles `src/*.c` with the CubeMX-generated HAL. The portable layer
(`tobii_proto.c`) has **no HAL dependency** and can be unit-tested on the host:

```sh
cc -I include -o /tmp/t src/tobii_proto.c tools/test_decode.c && /tmp/t   # TODO
```

## Milestones

1. **[done]** Portable protocol layer + design (this scaffold).
2. CubeMX project: OTG_FS **host**, I²C1, UART; USB_HOST middleware.
3. Enumerate the tracker and read its SCP metadata (req 48) — proves the FS host
   link works at all.
4. Replay the init (embedded from `init_packets_ep.txt`) and read EP `0x83`.
5. Decode `0x500`, print gaze over the console — **proves FS gaze works.**
6. SSD1306 driver + the gaze dot (behind `OPEN_TOBII_ENABLE_DISPLAY`).

Milestone 3/5 is the go/no-go for the Full-Speed bet.
