# Tobii Pro Spark support (model `TPE01` / platform `TPSP1`)

This documents what is known about the **Tobii Pro Spark** and how it is wired
into this codebase. The Spark speaks the **same TTP-over-USB protocol as the
Eye Tracker 5** and is driven by the **same embedded init**: replaying the ET5
init onto a Spark brings the device up and **arms its gaze stream** (verified
live, see [Live validation](#live-validation)). It is recognised, reported and
driven like the ET5; the model registry only tells the two apart by USB product
id.

## USB identity (confirmed against a live Spark)

| | Eye Tracker 5 | **Pro Spark** |
|---|---|---|
| USB vendor id | `0x2104` (Tobii AB) | `0x2104` (Tobii AB) |
| **USB product id** | `0x0313` | **`0x031C`** |
| `iProduct` string | `EyeChip` | `EyeChip` |
| Serial prefix | `IS5FF-` | `TPE01-` |
| `bcdDevice` | — | `0x0100` |
| USB speed | — | high-speed (USB 2.0) |
| Configurations | 1 | 1 |
| Interface 0 | vendor class `0xFF`, 3 EPs: `0x83` IN, `0x05` OUT | vendor class `0xFF`, 3 EPs: `0x83` IN, `0x05` OUT, **`0x04` OUT** |

Endpoints confirmed from the live device: interface 0 is vendor class `0xFF`
with three bulk endpoints (max packet 512). Both models share bulk **`0x83`
IN** and **`0x05` OUT**; the Spark adds an **extra bulk OUT `0x04`** that this
crate does not use (role unknown — a second command/data channel). Driving the
Spark over `0x05`/`0x83` alone is enough to bring it up and stream gaze.

The `EyeChip` product string and serial prefix are hints only. The authoritative
model identity (`IS5FF`, `TPE01`, …) is reported by the device over TTP and
surfaces through `DeviceFacts.model`; it is not hardcoded here.

## Platform identity

The Spark is served on macOS by
`platform_runtime_IS5LPROENTRY_MAC_x64_service`. Strings in that runtime place
the Spark on the platform codenamed **Castor** (`IS5PRO` / `IS5LPROENTRY`,
`IS5_DEVICE_CASTOR_EVAL`), with sources under
`platform_modules/castor/src/standard/platmod_castor_IS5.cpp` and a
TTP-over-USB transport in
`platform_modules/castor/src/common/tracker_lib_usb/src/tracker_usb_ttp.cpp`
and `.../ttp/ttp_parser.cpp`.

**Consequence:** the Spark speaks the **same TTP-over-USB protocol** as the ET5.
The expected framing is unchanged — USB bulk pieces `[u32 channel][u32 len]`,
TTP message `marker/seq/status/id/error/len` (big-endian u32) followed by a
`00 00` lead-in and TLVs — and the calibration command sequence validated on the
ET5 is expected to apply unchanged:

- start: `0x1010` → `0x1060` → `0x1110` (write), to prime the sample ring with
  the active calibration;
- collect: `0x1030` (point x,y in 32.32 fixed-point + eye mask, payload led by
  `00 00`);
- compute: `0x1070` → `0x1100` (read, returns the computed blob, chunked);
- stop: `0x1020` → `0x1110`.

TTP error codes are shared: `0x20000501` NOT_SUPPORTED, `502` OPERATION_FAILED,
`503` CALIBRATION_NOT_STARTED, `505` UNAUTHORIZED, `508` BAD_STATE,
`509` INVALID_PARAMETER.

Firmware family is **TPSP1** (the Eye Tracker Manager fetches
`downloadable-content/firmware/TPSP1/index.json` and
`downloadable-content/drivers/spark.json`), and `TPE01` appears in Eye Tracker
Manager's `supportedTrackerPrefixes`.

## Live validation

Verified on a live Spark (`TPE01-100205013153`, fw `615b6e3dfa`) on macOS:

1. The Spark answers the Tobii control protocol (SCP, vendor request 48):
   `/hw/platformtype=IS5LPROENTRY`, `/fw/fwversion=615b6e3dfa`.
2. Replaying tobii-usb's embedded ET5 init (`calibration::embedded_packets`,
   202 packets) onto the Spark after `vendor_control_init` **succeeds**: 41
   responses, and `wait_for_gaze_stream` returns **armed** — the Spark streams
   gaze under this crate's control.
3. Only two init commands come back non-zero, both `0x20000505` UNAUTHORIZED,
   on ids `0x777` and `0x848` — the ET5 licence/signature-carrying commands.
   They are **harmless**: gaze arms regardless. A Spark-native init could drop
   or replace them (see below).

Separately, Tobii's own SDK (PETM 2.7.2 with the Pro SDK 1.11 module, over
`tobii-et5-bridge`) was captured driving the Spark through a full session —
init, display-area write (id `0x5a0`, 188 bytes vs the ET5's 164), stream
subscribe, calibration, and live gaze/eye-image frames. That capture is the
ground truth for a Spark-native init if one is built later.

## How it is wired in this codebase

- `crates/tobii-usb/src/models.rs` — the model registry. Both
  `Model::EyeTracker5` and `Model::ProSpark` are `InitSource::Embedded` and
  drivable; they are told apart only by USB product id. `InitSource::Unavailable`
  remains for any future model recognised before its init is known.
- `crates/tobii-usb/src/device.rs` — discovery (`open_tobii`, `tobii_address`)
  matches the whole registry via `models::recognise(vid, pid)`, logs which model
  was found, and refuses a recognised-but-not-drivable model with an explicit
  error. The Spark takes the same path as the ET5: same endpoints, same embedded
  `init_packets_ep.txt`, same calibration/display-area splicing.

## Gaps / possible refinements

1. **Full daemon run.** Init + gaze arming is verified; a complete `tobiid`
   session on the Spark (stream catalogue columns, presence, head pose, a real
   calibration round-trip) has not been exercised end to end.
2. **Spark-native init.** The Spark runs on the shared ET5 init. A Spark-specific
   `init_packets` built from the captured session (`spark-full-capture.tsv`)
   could drop the two UNAUTHORIZED licence commands (ids `0x777`, `0x848`), carry
   the Spark's extra property reads (`0x4ba`, `0xbf4`, `0xc08`, `0x6a4`) and its
   188-byte display area, and embed a Spark calibration blob. This would mean a
   per-model init resource and parameterising the blob splice (`embedded_blob` /
   `substitute_calibration`, today keyed to the ET5's `packets[38..200]`).
3. **Extra bulk OUT `0x04`.** Present on the Spark, unused here; its role is
   unknown. Not needed for gaze.
4. **Stream catalogue specifics.** Output frequencies, image-stream size
   (`READ_BUF` is sized for the ET5's 78609-byte image message), display
   generation and capability differences are not yet characterised for the Spark.
