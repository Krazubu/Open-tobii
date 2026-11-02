# What stands behind the 199 entry points

`libtobii.so` exports **all 199 entry points** of Tobii's
`tobii_stream_engine.dll` 4.25.0.3, and no others, with its signatures, error
numbering and struct layouts but one (`tobii_hardware_configuration_t` keeps
4.1.0.3's two entries and 2472 bytes, where 4.25.0.3's holds four and is
inferred to be 3960) — so any Stream Engine client links and runs.
4.1.0.3, which this library followed until it had the whole of 4.25's
surface, exports 151 of them, and two that 4.25 renamed. The ABI was
recovered from the two DLLs themselves (`tools/abi/`) and the archived
4.1.0 reference, which the binaries overrule where they disagree with it.

This page says what each entry point answers for a Tobii Eye Tracker 5 and,
where that is not what the Windows DLL answers, why: *What stands behind the
entry points* for those 4.1 already had and for what all of them share,
*What 4.25 added* for the 48 names 4.25 added. `tools/abi/README.md` carries
4.1.0.3's derivation, with the DLL addresses every claim was read at; what
4.25.0.3 adds or changes is derived in the doc comments of
`crates/tobii-ffi/src/` and the headers in `crates/tobii-ffi/include/tobii/`,
with the DLL addresses where those give them.
[CHANGELOG.md](../CHANGELOG.md) says what changed when. For the shape of the
driver rather than its API, see the README.

## What stands behind the entry points

| | Entry points |
|---|---|
| Implemented (daemon-backed) | device lifetime and callbacks; gaze point, gaze origin, eye position, user position guide, presence, head pose, gaze data, raw gaze (the Stream Engine's own record of each gaze frame, `tobii_gaze_raw_subscribe`), the foveated rendering gaze point (made from raw gaze as the Stream Engine makes it for a tracker it drives itself, `tobii_foveated_rendering_gaze_point_subscribe`) and IR image streams; the custom stream (the tracker's stream 0x1770 as custom stream 0, subscribed by the Stream Engine's rules, `tobii_custom_stream_subscribe`; its frames carry no data on the ET5); image collection's subscribe (the tracker's stream 0x508, started as the Stream Engine starts it, `tobii_image_collection_subscribe`; the ET5 refuses it, so it answers `TOBII_ERROR_INTERNAL`, as the Stream Engine does, and no frame is ever delivered); notifications (display area, calibration state and id, pause, power save, faults and warnings); device info (the public and the internal one, `tobii_get_device_info_internal`), track box, display area (get/set), mounting, states; 2-D calibration, including discarding a point; the device/host clock pair (`tobii_timesync`); the tracker's stream catalogue and its streams' columns (`tobii_enumerate_stream_type_columns`); internal-stream support (the IR image, and the custom stream and image collection while that catalogue lists them, as the Stream Engine answers for an ET5, `tobii_internal_stream_supported`; absolute eye openness aside, see *What 4.25 added*); device pause and resume; power save, entered and left (`tobii_power_save_activate`, `_deactivate`), with its state and remote wake's; the tracker's extension lists (`tobii_enumerate_extensions`, `_enabled_extensions`), which on an ET5 are empty; the device name (kept by the host); the hardware configuration (provisional, see below); whether the tracker reports gaze over HID (`tobii_get_gaze_hid_enabled`, from its 1330 property 5, `false` on an ET5) and the track box that gaze is reported in (`tobii_get_combined_gaze_hid_track_box`, the tracker's measured one — see below); an application's own command to the tracker (`tobii_send_custom_command`, the tracker's command 5000) |
| Answered locally | API version, error texts (the DLL's), system clock, output frequency (33 Hz), enabled eye, capabilities, stream support (the streams libtobii delivers, where the DLL answers from what the device reports and never reports the user position guide for a tracker it drives itself, `tobii_stream_supported`), feature group, license validation, display-area calculation, calibration parsing, internal-capability support (eyeball centres only), lens-configuration writability (never), usage statistics (`TOBII_ERROR_NOT_AVAILABLE`, as the DLL answers without Tobii's Stream Engine Services); setting the illumination mode and enabling an extension (`TOBII_ERROR_INSUFFICIENT_LICENSE`, as the DLL answers a consumer ET5 without asking the tracker); clean IR, dumping the diagnostic images, reading diagnostics data and opening a realm (the same `TOBII_ERROR_INSUFFICIENT_LICENSE`, their module functions testing the licence before anything else, so no command goes out and no realm key is ever read); reading the display info (`TOBII_ERROR_NOT_AVAILABLE`, as the DLL answers without Tobii's Stream Engine Services, like the usage statistics) |
| `TOBII_ERROR_NOT_SUPPORTED` | what the ET5 refuses: remote wake and reading the illumination mode, whose commands it answers with an error the Stream Engine turns into this very code (the state commands, below); the configuration key, which it refuses until a realm it names (10000) is open (the Stream Engine answers otherwise, see the licences below); the diagnostics image and the display id, a stream and a property its catalogue and property lists do not hold; retrieving its diagnostic images or log files, whose Stream Engine commands its tracker module never offers; and what it was never observed doing: wearable, face id, firmware, licence storage on the device, 3-D and per-eye calibration, calibration stimulus points, the internal low-frequency head rotation and position, multiple faces position, wearable limited image and secondary camera image streams (for the wearable entry points, the digital sync port, face id, 3-D and per-eye calibration, the stimulus points and those five streams, the DLL's own answer for a tracker it drives itself through its in-process legacy TTP module, once the arguments check out; behind Tobii's service, never captured; per-eye calibration only inside the handle's own session, `TOBII_ERROR_CALIBRATION_NOT_STARTED` outside it, as there; for the face types, which the DLL asks the tracker for, and licence storage, which it offers a tracker with persistent storage, inferred); and setting the display info, which both DLLs refuse without reading it (*The display id and info*) |

Where the answers come from, and where they differ from Windows:

- **No licences.** Every key validates and the feature group is consumer, but
  nothing checks it: gaze data, timesync, calibration, display-area and name
  writes, the IR image, raw gaze, the internal device info, the stream
  catalogue, its streams' columns and pause all work, though the Stream
  Engine reserves them for higher feature groups (raw gaze, the internal
  device info, the stream catalogue, its columns and, in its own service,
  pause for its internal one)
  or, for the IR image, an additional-features licence. Where the Stream
  Engine refuses a consumer with `TOBII_ERROR_INSUFFICIENT_LICENSE` what the
  ET5 cannot do anyway (per-eye calibration inside a session, setting the
  face type), `libtobii.so` answers what it would past that check,
  `TOBII_ERROR_NOT_SUPPORTED`. The state commands split the difference, by
  choice: the extension lists answer past the check (they ask the tracker,
  where the Stream Engine refuses a consumer ET5), while setting the
  illumination mode and enabling an extension keep its refusal,
  `TOBII_ERROR_INSUFFICIENT_LICENSE`. Nothing could switch an extension off
  again, and the ET5 refuses to report an illumination mode and has no
  extension to enable. The configuration key is neither: it stays
  `TOBII_ERROR_NOT_SUPPORTED`, where the Stream Engine refuses a consumer
  `TOBII_ERROR_INSUFFICIENT_LICENSE` and, past that check, would report the
  ET5's refusal (it wants realm 10000 open) as
  `TOBII_ERROR_CONNECTION_FAILED`.
- **Facts from the last init.** Device info (both), track box, display area,
  mounting, the stream catalogue and its streams' columns, the hardware
  configuration and the fault and warning lists (`tobii_get_state_string`)
  are what the tracker reported at its last init, so they are answered even
  while it is unplugged
  (`TOBII_ERROR_TIMED_OUT` only if the daemon has not seen a tracker yet). A
  fault or warning list the tracker announces later (3200/3210, a layout
  taken from the Stream Engine and never yet seen from an ET5) replaces the
  one its init reported, and reaches notification subscribers either way, as
  in the Stream Engine. The first call that needs the tracker starts it, and
  it then stays on (IR illuminator lit) for as long as that connection is
  open, as for an open device in the Stream Engine. While it is unplugged,
  such a call (a clock pair, a pause, starting, retrieving or applying a
  calibration, a display-area write) is `TOBII_ERROR_CONNECTION_FAILED` at
  once from when the daemon has given the tracker up, a few seconds after
  the unplug (until then the call waits on the daemon's re-opens), and the
  daemon starts the tracker once it is plugged back in, for a connection
  still open. The subscribe of custom stream 0 or of image collection
  answers so too while the daemon holds the stream for no connection; one
  that waits on the re-opens gets what the Stream Engine answers for a
  tracker lost under the stream's start,
  `TOBII_ERROR_CONNECTION_FAILED_DRIVER` for the custom stream and
  `TOBII_ERROR_NO_ERROR` for image collection. A subscribe of a stream the
  daemon holds already (another connection subscribed it, say) joins it
  instead, `TOBII_ERROR_NO_ERROR` at once with or without the tracker, and
  the custom stream's frames come once the tracker is back.
  A calibration session its owner is not already stopping ends, saving
  nothing, when the daemon loses the tracker or the tracker re-initialises:
  its owner's later calls in it, `tobii_calibration_stop` included, are
  `TOBII_ERROR_CALIBRATION_NOT_STARTED`. A stop under way finishes, and a
  calibration it has saved stays saved even if the stop then fails
  (`TOBII_ERROR_CONNECTION_FAILED` when the tracker went away); the tracker
  loads it at its next init. Such a stop logs an ERROR line saying the
  calibration was saved but may not have been applied, since its status
  alone reads as a stop that kept nothing, and `tobii-calibrate` says so and
  exits 2.
- **Waiting for the daemon.** A call that asks the daemon (any but a
  subscription change or a reconnect, which wait 2 s and ~500 ms for their
  acknowledgement; the subscribes and unsubscribes of the custom stream and
  image collection ask it) waits for the answer longer than the daemon's own
  deadlines let it take, so `TOBII_ERROR_TIMED_OUT` is the daemon's answer
  (or a daemon held up past them), not libtobii giving up on a call the
  daemon then carries out (a calibration started, a display area set).
  Within the daemon's deadlines that happens only behind a stop of the
  custom stream or image collection that libtobii sends without waiting for
  it, when the daemon refused an unsubscribe's stop (another client's start
  or stop of either stream kept it waiting 20 s) or libtobii gave up on a
  subscribe's start: the daemon runs that stop before the device's next
  call, for up to ~53 s, and a call whose own wait runs out meanwhile (a
  state read waits ~3 s) is `TOBII_ERROR_TIMED_OUT`, yet carried out. A
  call answered from the facts waits at most ~12 s, for the tracker's first
  init. One that needs the tracker live waits for it to stream (~12 s from
  cold) and answer, and at worst for the daemon's own deadlines: 30 s for
  the tracker to take each command, plus the command's, which comes to 35 s
  for `tobii_calibration_retrieve`, about a minute for a subscribe or
  unsubscribe of the custom stream or image collection (which may also wait
  20 s for another start or stop of either stream) and about 3 min for a
  `tobii_calibration_start` that fails and puts the calibration back
  (`crates/tobii-ipc/src/timeout.rs` has every request's timeout, which
  `tobii-calibrate` and `ipc-probe` wait too, and
  `crates/tobii-ffi/src/timeouts.rs` which call waits which).
- **Lost daemon.** When the connection to `tobiid` is lost (the daemon
  stopped, crashed or was restarted), `tobii_device_process_callbacks`
  delivers what had arrived and then returns `TOBII_ERROR_CONNECTION_FAILED`
  on every call; the DLL returns it without delivering its queue on that
  call. `tobii_wait_for_callbacks` wakes once for the loss and never returns
  `TOBII_ERROR_CONNECTION_FAILED`. Nothing reconnects by itself: as the
  Stream Engine documentation says, the application calls
  `tobii_device_reconnect`. That only connects to a running daemon (it never
  spawns one, unlike `tobii_device_create`), gives up with
  `TOBII_ERROR_CONNECTION_FAILED` within ~500 ms when it cannot, and
  restores the subscriptions and the holds of the custom stream and image
  collection (the daemon starts the streams again itself; a daemon that
  refuses a hold fails the reconnect, a tracker that refuses a start does
  not, where it fails the Stream Engine's), not a calibration session or a
  pause the lost connection held.
  Reconnecting a live connection drops the samples the old one brought that
  were not delivered yet, which may lose those tobiid sent it alone just
  before (from the new one's first tick on it sends both the same, and each
  is delivered once), but keeps its notifications, which tobiid sends once
  and does not repeat to a new connection: they come ahead of the new
  connection's, and one sent to both may be delivered again, after later
  ones, so a state may be seen to step back before it settles (the last
  delivered is current). A tracker unplug is not a lost connection and never
  shows in `tobii_device_process_callbacks`: the daemon keeps every
  connection, and the samples resume on it once the tracker is back. Requests
  that need the tracker get `TOBII_ERROR_CONNECTION_FAILED` meanwhile
  (above), but that is the daemon's answer over a live connection, and a
  reconnect then succeeds without bringing the tracker back. OpenTrack's
  `tracker-tobii` plugin needs a reconnect call to recover by itself; as it
  stands, stopping and starting tracking recovers it (INSTALL.md §8).
- **Pause.** One state for the tracker, shared by every client, as in the
  Stream Engine: the last call wins and any client may resume.
  `TOBII_STATE_DEVICE_PAUSED` and a `DEVICE_PAUSED_STATE_CHANGED`
  notification change as soon as the tracker accepts the call. A pause ends
  when the client that paused last disconnects, and when the tracker
  re-initialises (after a USB failure or an unplug). A resume the tracker
  does not answer still succeeds: the daemon re-opens the silent tracker, and
  its init resumes it. Pausing during a calibration session is
  `TOBII_ERROR_CALIBRATION_BUSY`; starting one, or asking for
  `tobii_timesync`, while paused is `TOBII_ERROR_NOT_AVAILABLE`.
- **Power save.** `tobii_power_save_activate` and `_deactivate` send the
  tracker the Stream Engine's own commands, which the ET5 carries out; power
  save does not stop the sensor, and gaze keeps coming. It is one state for
  the tracker, as the pause is: the last call wins, any client may end it,
  and the daemon ends it when the client whose call is in effect (the last
  to enter it) disconnects, which the Stream Engine, living in its client's
  process, never has to do. The ET5 keeps power save through a re-open, the
  device's teardown and a new init included; the daemon sends nothing at a
  re-initialisation, unlike for a pause, and never re-applies power save. A
  daemon stopped or restarted while a client holds power save ends it first,
  as that client's departure would, where the Stream Engine leaves it on
  when its process ends; only a daemon killed outright leaves the tracker in
  power save with nobody holding it. Ending it for a client that went away,
  it records power save off from the tracker's answer, since an engine
  stopping for want of clients may never read the notification that follows.
  `TOBII_STATE_POWER_SAVE_ACTIVE` follows the tracker's own notifications
  rather than the call, as in the Stream Engine, and the ET5 sends one for
  each change. Remote wake the ET5 does not implement:
  `TOBII_STATE_REMOTE_WAKE_ACTIVE` reads false from the tracker's status,
  and arming or disarming it is `TOBII_ERROR_NOT_SUPPORTED`, which is how
  the Stream Engine reports the tracker's refusal.
- **Gaze over HID.** Both reads answer real data, where the Stream Engine's
  own licence gate might suggest otherwise: `tobii_get_gaze_hid_enabled`
  reports the tracker's 1330 property 5, which an ET5 sends as `false`, so
  the answer is no error and zero (its module's gate is "below licence level
  0", which every tracker passes). It is `TOBII_ERROR_NOT_SUPPORTED` only
  when the tracker reported no such property, as the Stream Engine answers
  without it. `tobii_get_combined_gaze_hid_track_box` **differs on purpose**:
  the Stream Engine answers a track box held in its own code — a 300x200 mm
  front face at 450 mm and a 550x400 mm back face at 900 mm — while this
  library answers the box the tracker itself measured (command 1400:
  250x200 mm at 450 mm and 500x400 mm at 900 mm). The two agree in height and
  depth exactly and differ by 25 mm on each side in width, so an application
  sees the same box from this call and from `tobii_get_track_box`, where
  Windows gives two slightly different ones. When the daemon has no box to
  give — an older daemon, or a tracker that reported none — it falls back to
  the Stream Engine's own values, so this call never answers
  `TOBII_ERROR_NOT_SUPPORTED`, which the Stream Engine's cannot either.
- **An application's own command.** `tobii_send_custom_command` sends the
  tracker the Stream Engine's command 5000 and hands its answer to the
  receiver. The `command` number never reaches the tracker: the Stream Engine
  hard-codes the command id and uses that number only as a gate, refusing
  anything but 0 with `TOBII_ERROR_NOT_SUPPORTED` and sending nothing, which
  this library does too. The ET5 knows the command — asked with a
  four-zero-byte blob it answered "invalid parameter", not "no such command" —
  but what a valid blob holds is the application's business: nothing in the
  Stream Engine or any capture says. Unlike Windows, both blobs are bounded
  at 64 KiB: the Stream Engine's own reply path copies a length the device
  sent into a block it allocated without one.
- **Diagnostics, logs and clean IR.** All eight answer as the Stream Engine
  does for an ET5, without asking the tracker, but by two routes that reach
  two different answers. The diagnostics image stream and the two retrieves
  are `TOBII_ERROR_NOT_SUPPORTED`: the ET5's catalogue does not list the
  stream, and its tracker module never offers the two Stream Engine commands
  the retrieves look for. Clean IR, dumping the images and reading
  diagnostics data are `TOBII_ERROR_INSUFFICIENT_LICENSE`, their module
  functions testing the licence level before anything else — so clean IR
  never reaches the check for the stream the ET5 also lacks, and the
  tracker's commands 1500 and 1510 never go out at all.
- **The display id and info.** The four display-id calls are
  `TOBII_ERROR_NOT_SUPPORTED`: the id is a Stream Engine property the
  tracker's module publishes in none of its lists, so the Stream Engine
  refuses them before any request too. The display id is a **string**, not a
  number, which the header said until 2026-10-22. `tobii_get_display_info`
  is `TOBII_ERROR_NOT_AVAILABLE`: past the Stream Engine's own checks it is
  not a tracker command at all but a request to Tobii's Stream Engine
  Services, which Linux has not — the same reason the usage statistics
  answer that code. `tobii_set_display_info` is `TOBII_ERROR_NOT_SUPPORTED`
  once its two pointers check out, as both DLLs answer without reading the
  record, and from inside a callback too, as 4.1's does, where 4.25's
  answers `TOBII_ERROR_CALLBACK_IN_PROGRESS`
  (`crates/tobii-ffi/src/internal.rs`).
- **Realms.** `tobii_open_realm` is `TOBII_ERROR_INSUFFICIENT_LICENSE`, which
  is what the Stream Engine answers a consumer ET5 before it contacts the
  tracker: its module tests the licence level as its first instruction. So no
  realm can be opened here, exactly as on Windows, and the key is never read.
  The ET5 does refuse two things for want of one — the configuration key and
  image collection, both naming realm 10000 — and that stands.
- **Notifications.** Seven of the thirteen types are delivered:
  `CALIBRATION_STATE_CHANGED` and `DEVICE_PAUSED_STATE_CHANGED` from the
  daemon itself, when a calibration session starts or ends and when the
  tracker accepts a pause or resume (the Stream Engine waits for the
  tracker's own messages, of which the ET5 was seen sending only the
  pause's); `DISPLAY_AREA_CHANGED`, `CALIBRATION_ID_CHANGED`,
  `FAULTS_CHANGED` and `WARNINGS_CHANGED` from the tracker's, one for each
  (as the Stream Engine does for faults and warnings, and as far as traced
  for the other two); `POWER_SAVE_STATE_CHANGED` from the tracker's too,
  every one, as the Stream Engine passes them on, and none on subscribe,
  where the Stream Engine may hand out an initial "off" (not traced).
  The others never come: the output frequency is
  fixed at 33 Hz, both eyes are always used, the ET5 was never seen changing
  its track box or reporting a face type, and the
  Stream Engine has no source for `CALIBRATION_ENABLED_EYE_CHANGED`.
  Exclusive mode is not reported either: the ET5 sends what the Stream
  Engine reads as exclusive mode on and off only on Linux, whenever an open
  starts the sensor (a cold engine start, or a re-open after the stream
  was lost): on with that open, off with the re-open that primes the
  stream about 10 s later. It never does so on Windows, so it says nothing
  about another application; `TOBII_STATE_EXCLUSIVE_MODE` stays false. The
  Stream Engine most likely also delivers one
  `COMBINED_GAZE_EYE_SELECTION_CHANGED` (both eyes) after
  `tobii_device_create` or `tobii_device_reconnect`, from the tracker's
  answer to its init; libtobii does not. `tools/abi/README.md` has the
  Stream Engine's side.
- **Device name.** A name set with `tobii_set_device_name` is kept by the
  daemon in `~/.config/tobii/device-name`, for every client and later
  sessions; nothing is written to the tracker. Until one is set,
  `tobii_get_device_name` gives the model.
- **Calibration parsing.** `tobii_calibration_parse` checks the whole blob
  before it hands out a point: data that is not a valid calibration is
  `TOBII_ERROR_OPERATION_FAILED`, as the Stream Engine documentation says.
  That is a blob shorter than its 44-byte header or over 4 MiB, a point list
  that lies outside the blob, holds more than 256 points or does not end
  exactly at `data_size`, a status word other than -1, 0, 1 or 2 (its low
  32 bits read as a signed int, as the Stream Engine reads them, with the
  upper 32 zero, or all ones with -1), and a value that is not finite or
  lies more than half a display off it, but for the mapping of an eye
  marked failed (-1). The Stream Engine returns that only for a negative
  point count; it reads everything else as given, past `data_size` if the
  blob says so, and returns `TOBII_ERROR_NO_ERROR` (8 zero bytes are an
  empty calibration there). Both report an eye whose status word is -1 as
  `FAILED_OR_INVALID` and pass its mapping on as the blob holds it.
- **Head pose.** The Stream Engine's comes from Tobii's runtime, which computes
  it on the host from each IR image; libtobii's from the daemon, which runs a
  face-landmark tracker on the same images and a model fitted to the poses the
  DLL delivered in three captured Windows sessions. It has the Stream Engine's
  form: absolute, in the display frame of the display area in effect, as gaze
  origins are (the position a point between the eyes, in mm from the area's
  centre; the rotation as the Stream Engine's yxz Euler angles, zero for a face
  square to the display), unclamped; one pose for every IR image, ~33 Hz, with
  the image's time; and invalid, all four validities at
  once, when there is no face or it is at the edge of the image, where the
  Stream Engine's turns invalid too. An invalid pose holds the last valid
  values (zeros before the first), where the DLL's holds leftovers of its other
  callbacks. Replayed through the daemon's pipeline without its rotation gains
  (the model fitted to the Stream Engine's pose, which `tobii5-init-replay
  compare-dll --head` replays), the three sessions' poses are valid on 96.8 to
  100 % of the images on which the DLL's are, and differ from the DLL's by a
  median 2.2 to 4.0° in rotation, 4 to 7 mm in z and under 4 mm across;
  constants fitted on two of the sessions do about as well on the third
  (`tools/headpose/README.md`). Those are medians over every image, most of
  them near straight ahead, not bounds, and they hide a scale: TX, TZ and roll
  move as far as the DLL's to within 3 %, TY 0.84 to 0.96 times as far; yaw
  and pitch, by design, do not. Measured with a laser pointer fixed to the head
  and aimed at rings on the screen, the Stream Engine under-reads a turn, and
  by a factor that changes from one session to the next (yaw 0.72 to 0.79 of
  the true angle, pitch 0.76 to 0.95, in three recorded sessions), where the
  model fitted to it read yaw a steady 0.80 and pitch 1.07 to 1.11. libtobii
  scales its yaw and pitch to the true angles (`HeadParams::rotation_gain`,
  1.249 and 0.913; with gains fitted without a session, 0.99 to 1.01 of the
  truth in yaw and 0.96 to 1.02 in pitch on it), so its yaw turns 1.27 to 1.44
  times as far as the Stream Engine's and its pitch 0.72 to 0.90 times as far
  in the Windows sessions, and its rotation differs from the DLL's by a median
  2.9 to 6.1° (8 to 11° where the DLL's yaw is 20° or more). Small yaw turns
  read short, as the Stream Engine's do more: about 0.8 of a 5° turn and 0.93
  of a 10° one, whole at 20 to 25° (INSTALL.md §8 has every axis;
  `tools/headpose/README.md`, "Laser truth", the measurement, of one person at
  one screen). What it does not reproduce: the head size the runtime ranges by,
  which differed by up to 1.5 % between the sessions (6 to 9 mm in z at a
  desk), where the model has one constant; the range the runtime reports 7.5 to
  11 % long for some 17 images after it finds a lost face again; its filters,
  which an EMA on the position and one-euro filters of one shared setting (1.0
  Hz, 0.2) on the three angles approximate (lags that differ from the DLL's by
  up to 13 ms on the angles and 21 ms on the position, noise at rest 0.6 to 1.4
  times the DLL's without the gains, which scale it too: yaw's up to 1.8 times
  the DLL's); and a pose for every image: the daemon's pose worker takes
  the newest image, and one it has not taken by the time the next comes gets
  none, which at ~6 ms an image on a desktop CPU, against 30 ms between images,
  takes a busy machine (`TOBII_IMAGE83_DEBUG` counts them, INSTALL.md §7).
  As with the Stream Engine, an application centres the pose itself, as
  OpenTrack does.
- **Timestamps.** Every callback timestamp but the tracker times below is on
  `tobii_system_clock`'s clock, as in the Stream Engine; the tracker's clock
  is left only in gaze data's and raw gaze's `timestamp_tracker_us` (raw
  gaze has no other time) and `tobii_timesync`'s `tracker_us`. The foveated
  rendering gaze point, which the Stream Engine stamps with raw gaze's
  tracker time plus its offset, takes the host time `tobiid` sends beside
  each raw gaze frame (a `tobiid` from before that sends none, and it then
  carries the tracker's). A head
  pose carries the time of the IR image it was made from, as there. The
  Stream Engine adds one offset per connection, from round trips to its
  service, and refreshes it only in `tobii_update_timesync` and
  `tobii_timesync`. Here the daemon's USB engine estimates the offset from
  the arrivals: the smallest receipt-minus-device time of the gaze frames and
  IR images of the last 120 s (the gaze frames alone when the image stream is
  off), started afresh at every tracker init, which may restart the
  tracker's clock. So it follows the drift between the clocks (5 to 13 ppm
  in the captures, some 50 ms an hour for a fixed offset), and every client
  gets the same stamps. The interval between two stamps can then differ
  from the tracker's by a couple of ms when the estimate moves (the Stream
  Engine's are exact within a connection), though never below 1 µs within a
  stream while the tracker stays open; across an init the stamps run on with
  the host clock instead of jumping back, and a presence reported again to a
  new subscriber keeps the stamp it was last reported with. A stamp is no
  later than the daemon's read of its sample (1 µs past it when two samples
  of a stream are read together), but for a sample read during the
  tracker's init, which gets the time it is delivered, a few ms late.
- **Clock pair.** `tobii_timesync` pairs the device timestamp of the next gaze
  frame with the host clock (`CLOCK_MONOTONIC`, as `tobii_system_clock`) in a
  fixed 30 ms bracket; the Stream Engine times a round trip instead, on
  `QueryPerformanceCounter`. With no tracker plugged in it is
  `TOBII_ERROR_CONNECTION_FAILED` rather than a wait, and a tracker unplugged
  shortly before or during the call makes it so about a second after the
  daemon gives the tracker up.
- **Hardware configuration.** Its layout is 4.1.0.3's, not 4.25.0.3's (see
  the top of this page): growing it would overrun the 2472 bytes a caller
  built against 4.1 provides. What the fields hold is inferred from one
  Windows capture, and on Linux the ET5 has answered its command (2120) with
  no data, so `tobii_hardware_configuration_get` is
  `TOBII_ERROR_NOT_SUPPORTED` until it reports one.
- **Threads.** As the Stream Engine promises, the functions may be called
  from several threads at once, on one device too: a device's state is
  locked by concern, as the DLL's is. Its requests, subscription changes and
  reconnects run one at a time, in the order they are called, each for its
  whole round trip to tobiid (a calibration start may take ~3 min at worst,
  a pause a minute: *Waiting for the daemon*), so one thread's calls made
  back to back hold another thread's up for one of them at most. They do not
  hold up its callbacks, processing or waiting, but for a reconnect's round
  trip (~500 ms at most) and its close of the old connection; a reconnect's
  ~500 ms counts from when the calls ahead of it, and then a process call
  another thread is making, have finished. Its callbacks run one at a time,
  on whichever thread processes it, and a subscribe, an unsubscribe, a clear
  or a reconnect waits for one running on another thread; once an
  unsubscribe returns, its callback is not running and never runs again.
  `tobii_device_destroy` and `tobii_api_destroy` take no lock, as in the
  Stream Engine (whose documentation says so for `tobii_device_destroy`): no
  other thread may be inside a call on the handle, or use it afterwards.
  Threads that create devices at once while no daemon runs spawn one
  `tobiid` between them: the others wait for that spawn, then connect to its
  daemon or fail as it did (two processes doing so can still spawn one
  each). `TOBII_ERROR_CALLBACK_IN_PROGRESS` guards only the thread a
  callback, the logger or `tobii_calibration_retrieve`'s receiver runs on. A
  callback, or the logger, must not block on another thread's call into any
  device (nor on a thread that waits for one), which can deadlock, as in the
  Stream Engine: only processing and waiting are sure to go on while a
  callback runs. The retrieve receiver holds no lock, so it may wait for
  other threads' calls. Unlike the DLL, a device's requests, subscription
  changes and reconnects run in the order they are called, where the DLL's
  critical section promises no order among its waiters (Windows semantics,
  not read from the DLL), so there one thread's calls made back to back may
  keep another thread's out for long; `tobii_wait_for_callbacks` waits on a
  device another thread is processing as on any other, where the DLL skips
  such a device, returning at once when it was the only one, so a
  wait-and-process loop on that device alone spins; a subscribe lets the
  device's other callbacks run during its round trip, where the DLL's, but
  for its custom stream subscribe, holds them back, and it sets its own
  first, where the DLL stores it once the round trip is done, so a
  subscribe that fails may have had its callback called before it
  returned; a `tobii_device_process_callbacks` that finds another
  thread processing returns at once, as there, but with
  `TOBII_ERROR_CONNECTION_FAILED` once the loss has been reported, and
  delivers nothing (the DLL first delivers the device's queued
  notifications); a clear waits for another thread's processing, never for a
  request (both wait for a reconnect's round trip and its close of the old
  connection);
  `tobii_calibration_retrieve` calls its receiver with no lock held, where
  the DLL holds the device's API mutex; and the logger is never called under
  a lock of the call that logs.
- **Logging and allocation.** The `tobii_custom_log_t` logger gets
  libtobii's own few lines (a refused `field_of_use`, a failed connect or
  reconnect, a lost daemon connection once per loss, a daemon reply that
  does not decode, a calibration stop that failed after the daemon saved the
  calibration, a head pose the daemon has not sent in the 20 s since it was
  subscribed and a custom stream or image collection it refuses as not
  supported though the tracker lists it, each once per device, each connect
  and reconnect), not the line per failing call the Stream Engine writes:
  the returned status says that. It is called on the thread inside the call
  that logs, from several threads at once if they log at once (their lines
  may interleave), and a device keeps logging through it after
  `tobii_api_destroy`. A `tobii_custom_alloc_t` is checked as in the Stream
  Engine and never called: libtobii allocates with Rust's allocator, where
  the DLL allocates the API handle, each device and long log lines through
  it (INSTALL.md §8).

The headers are in `crates/tobii-ffi/include/tobii/` (installed to
`/usr/include/tobii/` by the packages, `/usr/local/include/tobii/` by `make
install` and the tarball); OpenTrack's `tracker-tobii` plugin builds against
them from the `tracker-tobii-linux` branch of the
[OpenTrack fork](https://github.com/Skyggedans/opentrack/tree/tracker-tobii-linux),
which brings it to Stream Engine 4 and lets it configure on Linux
(INSTALL.md §8).

## What 4.25 added

Stream Engine 4.25 adds 46 entry points to 4.1's and renames two:
`tobii_multiple_faces_position_subscribe` and `_unsubscribe` are now
`tobii_multiple_faces_subscribe` and `_unsubscribe`, the same internal stream
5, refused as before. The old names are gone from the DLL and from
`libtobii.so` alike, so a program built against 4.1 that calls them has to
move to the new ones, as it would against Tobii's own 4.25 DLL. Of the 46, ten
are served by the daemon, seven are answered locally and twenty-nine are
refused; every subscribe has its unsubscribe. Their derivations are in the doc
comments of the files named below, in `crates/tobii-ffi/src/` unless a path
says otherwise (the `tobii/` headers are in `crates/tobii-ffi/include/`), with
the DLL addresses where those comments give them, rather than in
`tools/abi/README.md`, which covers 4.1; what was inferred rather than read is
marked so here.

| | Entry points |
|---|---|
| Implemented (daemon-backed) | the tracker's fault and warning lists, read (`tobii_get_faults`, `tobii_get_warnings`) and as streams (`tobii_faults_subscribe`, `tobii_warnings_subscribe`); 4.25's presence stream (`tobii_presence_subscribe`); the responsive gaze point (`tobii_responsive_gaze_point_subscribe`, `tobii_stream_t` 12, which `tobii_stream_supported` reports supported), which by mistake carries the filtered point (below) |
| Answered locally | the output-frequency stream (`tobii_output_frequency_subscribe`: 33 Hz, once); the four signing calls (`tobii_signing_sign`, `tobii_signing_sign_with_private_key`, `tobii_signing_verify`, `tobii_signing_verify_with_public_key`), which need no device; whether the extended lens configuration can be written (`tobii_lens_configuration_extended_writable`: never, as for the basic one, and, inferred, as the DLL answers) |
| `TOBII_ERROR_NOT_SUPPORTED` | the internal streams 4.25 added: proximity data collection (`tobii_proximity_data_collection_subscribe`), the wearable entrance pupil position (`tobii_wearable_entrance_pupil_position_subscribe`), the primary camera image (`tobii_primary_camera_image_subscribe`, a stream of its own; the IR image is `tobii_image_subscribe`'s), the wearable diagnostics image and the wearable image (`tobii_wearable_diagnostics_image_subscribe`, `tobii_wearable_image_subscribe`), the device log (`tobii_device_log_subscribe`; not the log `tobii_logs_retrieve` fetches), absolute eye openness (`tobii_absolute_eye_openness_subscribe`) and the 3-D facial landmarks (`tobii_facial_landmarks_3d_subscribe`); multiple faces, under its new name (`tobii_multiple_faces_subscribe`); the ambient light level (`tobii_ambient_light_level_subscribe`), the facial-landmark groups (`tobii_facial_landmarks_groups_enumerate`) and the algorithm-debug stream (`tobii_algodbg_stream_subscribe`; the ET5's own stream named "algodbg", 0x1770, is custom stream 0); the remote cameras (`tobii_remote_cameras_enumerate`, `tobii_remote_camera_set`, `tobii_remote_camera_listener_subscribe`); reading and writing the extended lens configuration (`tobii_get_lens_configuration_extended`, `tobii_set_lens_configuration_extended`); and the two PRP commands `tobii_force_frame_sequence` and `tobii_set_algo_box_mode` (the DLL's own answer for a tracker it drives itself, once the arguments check out: read for the internal streams' subscribes and the ambient light level's, inferred for the rest; absolute eye openness is below) |

Where the answers come from, and where they differ from Windows:

- **Arguments.** The 44 of these 48 names that take a device (all but the four
  signing calls) check their arguments in this library's usual order
  (`lib.rs`): a call from inside a callback is
  `TOBII_ERROR_CALLBACK_IN_PROGRESS` before anything else, then a null device,
  or a null callback, receiver, buffer, path or other pointer the call
  requires, is `TOBII_ERROR_INVALID_PARAMETER`. The DLL checks its nulls
  before its callback flag, so the two differ for a null argument passed from
  inside a callback.
- **Refusals.** Past those checks the refusals answer
  `TOBII_ERROR_NOT_SUPPORTED`, with nothing sent to the daemon, registered or
  written (`stub.rs`, `internal.rs`, `wearable.rs`). An unsubscribe is
  `TOBII_ERROR_NOT_SUPPORTED` too, never `TOBII_ERROR_NOT_SUBSCRIBED`, and so
  is the DLL's, which checks support before it looks for a subscription (read
  for the internal streams, inferred for the algorithm-debug stream and the
  remote-camera listener), but for two that may differ: absolute eye
  openness's (below), and the ambient light level's, which the DLL answers
  `TOBII_ERROR_NOT_SUBSCRIBED` for a device that lists its property, 0x18
  (whether an ET5 does was not traced). `tobii_remote_camera_set` **differs on
  purpose**: a null path is `TOBII_ERROR_INVALID_PARAMETER`, where the DLL
  copies the path before any check and so crashes on a null one.
- **Absolute eye openness.** `tobii_internal_stream_supported` reports its
  internal stream, 0x10, unsupported, and its subscribe and unsubscribe are
  `TOBII_ERROR_NOT_SUPPORTED`, every time. In 4.25 it is the one stream of
  those added that the DLL's support check has a case for, on the path it
  takes for a tracker it drives itself, and the case is a licence gate. Below
  licence level 2 (a consumer ET5 is at 0, inferred for 4.25 from 4.1) the
  gate refuses on the licence and returns without writing the answer the check
  then reads, so the DLL answers from whatever its stack held: the stream is
  unsupported, and its subscribe `TOBII_ERROR_NOT_SUPPORTED`, unless that word
  happens to be 1. Its unsubscribe asks the same check before it looks for a
  subscription, so it is `TOBII_ERROR_NOT_SUPPORTED` too, or, when the word is
  1 and nothing is subscribed, `TOBII_ERROR_NOT_SUBSCRIBED`. At level 2 or
  more the gate answers from a word of the DLL's tracker module whose meaning
  is not known. libtobii gives the one deterministic answer, which is also
  what the gate's licence check means, and there is nothing to hand out: the
  ET5's own "Absolute Eye Openness" columns are always -1.0 and invalid
  (`internal.rs`, `crates/tobii-proto/src/gaze83.rs`).
- **Fault and warning lists.** `tobii_get_faults` and `tobii_get_warnings` are
  `tobii_get_state_string` for `TOBII_STATE_FAULT` and `TOBII_STATE_WARNING`
  (4.25 has it the other way round, the state call a wrapper over the two
  getters, for the same text), so they answer as it does (*Facts from the last
  init*): "ok" on an ET5; `TOBII_ERROR_NOT_SUPPORTED` when the tracker's init
  reported no list, as the DLL answers (read in 4.1, inferred for 4.25); and,
  the daemon being asked, `TOBII_ERROR_TIMED_OUT` or
  `TOBII_ERROR_CONNECTION_FAILED` where the DLL reads a cache of its own
  (`faults.rs`, `api.rs`). `tobii_faults_subscribe` and
  `tobii_warnings_subscribe` hand their callback the text of every list the
  tracker announces (3200/3210), changed or not, as the Stream Engine passes
  those on as notifications, each right after its `FAULTS_CHANGED` or
  `WARNINGS_CHANGED` notification (`crates/tobiid/src/frames.rs`, and this
  crate's `device.rs`). Subscribing starts the daemon's notification stream
  and asks for nothing else, so nothing comes at subscribe; the getters answer
  the list standing then. The DLL most likely hands a new subscriber that list
  at once, or an empty string while it holds none: its tracker module calls
  the subscriber before the subscribe returns, and that this reaches the
  application's callback is inferred (`faults.rs`). An ET5 has never been seen
  announcing either list, so neither callback has been seen to run on one.
- **Presence, as 4.25 serves it.** `tobii_presence_subscribe` delivers what
  `tobii_user_presence_subscribe` does, the status and its timestamp, in a
  `tobii_presence_t`, from the one presence the daemon sends; 4.25 reads it
  from its property 0x14 instead, and what it answers there for an ET5 was not
  traced (the current Tobii reference marks the pair deprecated;
  `streams.rs`). The two see the same changes, but a subscribe to either while
  the other already holds presence on the device is not told the presence
  standing then, and hears of it at its next change: it changes nothing the
  device asks of the daemon, which reports the standing presence only to a
  connection newly asking for it. Nothing chose that; in the DLL the two are
  separate subscriptions.
- **The responsive gaze point is the filtered one.** 4.25's stream 12 is for
  the tracker's responsive combined gaze point, which an ET5 sends, by its own
  column names, as key 0x20 of each gaze frame, unfiltered: the mean of the
  eyes' 2-D points while both are tracked, the tracked eye's while one is
  (`crates/tobii-proto/src/gaze83.rs`).
  `tobii_responsive_gaze_point_subscribe` hands out the combined point of the
  daemon's raw-gaze record instead, which `tobiid` fills, as the Stream Engine
  fills its own, from the filtered point (key 0x1c): the point and host time
  `tobii_gaze_point_subscribe` delivers, valid whenever the tracker's validity
  word is not 0, where the gaze point is valid only when it is 1, and nothing
  at all from a `tobiid` too old to stamp that record with the host time. It
  **differs by mistake** from what the stream is for, as its header says;
  handing out key 0x20 needs `tobiid` to send it, which no daemon message
  does. The DLL itself most likely refuses the subscribe for a tracker it
  drives, an ET5 included: its stream check answers for 12 as for the gaze
  point, but its legacy-TTP module answers the subscribe of PRP stream 0x1d,
  which 12 maps to, with a not-supported stub, as for the device log
  (`streams.rs`, `tobii/tobii_streams.h`). Subscribing to it alone starts the
  daemon's raw-gaze stream, as `tobii_gaze_raw_subscribe` does.
- **Output frequency.** `tobii_output_frequency_subscribe` calls its receiver
  once, with 33 Hz, on the calling thread before it returns, and never again:
  the ET5 runs at that one frequency (*Notifications*). The DLL's source for
  it fires once when the subscription is taken and again on each change; on
  which thread, and when, is not traced. The subscription is kept, so a second
  subscribe is `TOBII_ERROR_ALREADY_SUBSCRIBED` and the unsubscribe has
  something to clear, as there; neither asks the daemon (`config.rs`).
- **Signing.** The four signing calls touch no device: SHA-256 with RSA PKCS#1
  v1.5 over a PEM key, the signature in standard base64, as the DLL computes
  it in OpenSSL; run under Wine, the 4.25 DLL gave the same signature byte for
  byte (`signing.rs`). They return a `tobii_signing_result_t`, **not** a
  `tobii_error_t`: each of its values 0..16 is also a `tobii_error_t` value,
  every one but 0 with another meaning (2 is
  `TOBII_ERROR_INSUFFICIENT_LICENSE`), so a result is checked against its own
  names (`tobii/tobii_signing.h`).
  They **differ on purpose** where the DLL's behaviour is a memory error. A
  null argument is `TOBII_SIGNING_NULL_ARGUMENT`, `signature_size` (which the
  DLL writes through unchecked) and a verify's key path (which it passes to
  `fopen`) included. Nothing is written on failure, where the DLL's
  `tobii_signing_sign` clears the caller's 640 bytes first (its
  `_with_private_key` form clears nothing). The signature is always
  NUL-terminated, where the DLL's is only while the raw signature is 477 bytes
  or less (636 encoded, a key of at most 3816 bits), and never with the key in
  memory. A signature that would not fit the 640 bytes with its terminator
  (any key over 3816 bits, RSA-4096 included) is
  `TOBII_SIGNING_BASE64_FAILED`, where the DLL fills the buffer without one
  or, past 3840 bits, runs beyond it; so is a signature to verify that decodes
  to more than 640 bytes, where the DLL's `tobii_signing_verify` runs off its
  640-byte frame.
  They also differ where nothing was chosen, `libtobii.so` signing with Rust
  crates rather than the DLL's OpenSSL (`signing.rs`). It reads only an
  unencrypted RSA key, PKCS#8 or PKCS#1 to sign and SPKI to verify, as the one
  PEM block of its text, with nothing after its END line but one line ending:
  trailing text, a blank line or a NUL counted in `key_data_size` is
  `TOBII_SIGNING_SIGN_PEM_FAILED` or `TOBII_SIGNING_VERIFY_PEM_FAILED`, and so
  is a public exponent of 2^33 or more in either kind of key, or a public key
  of more than 4096 bits, where OpenSSL reads more keys (inferred). Key bytes
  in memory that are not UTF-8 are `TOBII_SIGNING_KEY_BUFFER_FAILED`, where
  the DLL hands them to its PEM reader as they are, which answers
  `TOBII_SIGNING_SIGN_PEM_FAILED` or `TOBII_SIGNING_VERIFY_PEM_FAILED` when it
  finds no key in them. And, **by mistake**, signing with an RSA key of 488
  bits or fewer, too short for the padding, aborts the calling process, where
  the DLL fails with one of its OpenSSL codes (inferred); verifying under such
  a key answers `TOBII_SIGNING_NOT_VERIFIED`.
