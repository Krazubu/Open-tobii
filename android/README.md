# Android

A minimal proof of concept: the Tobii Eye Tracker 5's gaze stream in an
Android app, no root, over the file descriptor `UsbManager` hands out (see
`crates/tobii-android`). One engine runs inside the app's own process — not
the daemon/client split the Linux and macOS builds use — and a calibration
screen runs the same captured command sequence as the daemon's own
(`tobiid::calibration`) directly against the engine's `Commands` handle.

## Building

1. Build the native library for your device's ABI (`arm64-v8a` for a current
   phone or tablet) with NDK 27 (`ANDROID_NDK_HOME` pointing at
   `ndk/27.2.12479018`), straight into the directory Gradle picks it up
   from:

   ```sh
   cargo ndk -t arm64-v8a --link-libcxx-shared -o app/src/main/jniLibs \
     build -p tobii-android --release
   ```

   The arm64 library includes head-pose inference (ONNX Runtime, from
   `ort`'s prebuilt `aarch64-linux-android` binary, linked statically).
   It needs the NDK's `libc++_shared.so` next to it in the APK, which
   `--link-libcxx-shared` copies in. `ort` has no prebuilt binary for
   `armeabi-v7a`, so that library runs the same models on tract (pure
   Rust, `tobii-usb`'s `head-pose-tract`, which needs Rust 1.91): slower,
   and 16 MB instead of 0.5 MB. The app shows a **Head pose** button only
   when the library has head-pose inference; inference is off until you
   press it, since it costs CPU (a whole core while it runs).

2. (Nothing to copy: `-o` above already wrote
   `app/src/main/jniLibs/arm64-v8a/libtobii_android.so` and
   `libc++_shared.so`.)

3. Build the APK (needs a JDK 17+ on `JAVA_HOME`):

   ```sh
   gradle assembleDebug
   ```

## Running

The tracker exposes a USB Video Class interface alongside the vendor one
this app talks to; Android gates USB permission for the whole device on
that account, so the app also requests the `CAMERA` runtime permission
(never opened) before `UsbManager` will show its own dialog — see
`MainActivity.requestCameraThenDevice`.
