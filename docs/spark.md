# Tobii Pro Spark support (model `TPE01` / platform `TPSP1`)

This documents what is known about the **Tobii Pro Spark** and how it is wired
into this codebase. The Spark is **recognised on the USB bus and reported**, but
**not yet drivable**: the one missing piece is a captured init replay (see
[Gaps](#gaps)). Until that lands, `tobii-usb` finds a Spark and refuses it with a
clear message rather than replaying the Eye Tracker 5's init onto it.

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
| Interface 0 | vendor class `0xFF`, 3 endpoints | vendor class `0xFF`, 3 endpoints |

The Spark presents the **same transport topology** as the ET5: a single
vendor-specific interface 0 with three endpoints. This crate assumes the same
endpoint addresses as the ET5 — bulk **`0x05` OUT** / **`0x83` IN** plus a third
(interrupt) endpoint — because the two share the platform's TTP-over-USB
transport (see below). **This assumption is not yet verified against a capture
of the endpoint descriptors; confirm it before relying on streaming.**

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

## How it is wired in this codebase

- `crates/tobii-usb/src/models.rs` — the model registry. `Model::ProSpark`
  carries the confirmed USB facts above and an `InitSource::Unavailable` (no init
  replay yet). `Model::EyeTracker5` is `InitSource::Embedded`.
- `crates/tobii-usb/src/device.rs` — discovery (`open_tobii`, `tobii_address`)
  now matches the whole registry via `models::recognise(vid, pid)`, logs which
  model was found, and `open_tobii` refuses a recognised-but-not-drivable model
  with an explicit error instead of replaying the ET5 init.
- The ET5 path is unchanged: same endpoints, same embedded
  `init_packets_ep.txt`, same calibration splicing.

## Gaps (what is still needed to actually drive a Spark)

1. **Init replay capture.** The ET5 is driven by an embedded capture
   (`crates/tobii-usb/init_packets_ep.txt`, replayed by
   `calibration::embedded_packets`). The Spark needs its own equivalent —
   captured from the Castor runtime ↔ Spark handshake (USB sniffer / `usbmon` on
   Linux, or by instrumenting the macOS runtime), or reconstructed statically
   from `platmod_castor_IS5.cpp`. Drop-in plan: add a Spark init resource and
   point `Model::ProSpark.init` at it (mirroring the ET5's `InitSource::Embedded`
   path), then teach `calibration`/`device` to select the per-model init.
2. **Calibration-blob byte offsets.** On the ET5 the calibration blob is spliced
   out of `packets[38..200]` of the embedded init. The Spark's init will have its
   own offsets; `embedded_blob` / `substitute_calibration` must be
   parameterised per model once the Spark capture exists.
3. **Endpoint addresses.** Assumed `0x05`/`0x83` from the shared transport;
   confirm against the Spark's real endpoint descriptors.
4. **Stream catalogue specifics.** Output frequencies, image stream size
   (`READ_BUF` is sized for the ET5's 78609-byte image message), display
   generation and capability differences are unverified for the Spark.
5. **Live bring-up.** None of the above has been exercised against a streaming
   Spark; discovery/refusal is all that is currently validated.
