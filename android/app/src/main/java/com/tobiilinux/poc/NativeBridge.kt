package com.tobiilinux.poc

/** Thin JNI surface over `tobii-android`'s native library. */
object NativeBridge {
    init {
        System.loadLibrary("tobii_android")
    }

    /**
     * Starts the gaze engine from [fd], already open and permitted by
     * `UsbManager` for the Tobii Eye Tracker 5. The caller must keep the
     * underlying `UsbDeviceConnection` open for as long as the engine runs.
     */
    external fun startEngine(fd: Int)

    /** Stops the engine and releases the device handle. */
    external fun stopEngine()

    /** `[x, y, validity (0.0/1.0), device timestamp (microseconds)]`. */
    external fun latestGaze(): DoubleArray

    /**
     * Whether this device's library makes head poses: only the arm64 one is
     * built with ONNX Runtime (the armv7 one is gaze-only).
     */
    external fun headPoseSupported(): Boolean

    /** Turns head-pose inference on or off (off by default: it costs CPU). */
    external fun setHeadPoseWanted(wanted: Boolean)

    /**
     * `[validity (0.0/1.0), x, y, z (mm), rotation x, y, z (radians),
     * device timestamp (microseconds)]`, in the display frame. All zero
     * before the first pose and while head poses are not wanted.
     */
    external fun latestHeadPose(): DoubleArray

    /**
     * Starts a calibration session: the captured start/clear/write, seeded
     * from the calibration saved under [storageDir] (the app's own files
     * directory), or the one built into the init replay if none is saved
     * yet. `false` if the device refused it, or there is no engine running.
     */
    external fun calibrationStart(storageDir: String): Boolean

    /**
     * Collects one point, normalised display coordinates (0..=1 each way).
     * Blocks the calling thread for under a second while the device
     * samples the user's gaze at it: call off the main thread.
     */
    external fun calibrationCollect(x: Double, y: Double): Boolean

    /**
     * Computes the calibration from the points collected so far. Blocks for
     * a couple of seconds. The new calibration's id, or -1 on failure
     * (nothing collected, the device refused, or what it computed does not
     * validate).
     */
    external fun calibrationCompute(): Int

    /**
     * Ends the session: `keep` writes back and saves the last computed
     * calibration; otherwise the one the session started from goes back.
     * Returns whether a calibration was saved.
     */
    external fun calibrationStop(keep: Boolean): Boolean

    /** Why the last calibration call that failed did; empty when none has. */
    external fun lastCalibrationError(): String
}
