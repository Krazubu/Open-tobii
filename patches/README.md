# i386 macOS build patches

Patches applied to vendored/registry crates so the 32-bit macOS (`i686-apple-darwin`) build compiles. Not needed on x86_64/arm64.

## winit-0.30.13-i386-cgfloat.patch

`winit` 0.30.13 assumes `CGFloat` is `f64`, but on 32-bit macOS `CGFloat` is `f32` (via objc2-foundation). This breaks `tobii-calibrate` (the only winit user). The patch fixes 8 type errors in winit's macOS backend; the key change makes `handle_scale_factor_changed` take `f64` instead of `CGFloat`. All edits are no-ops on x86_64.

Apply to the cargo registry copy before building for i386:

    W=~/.cargo/registry/src/*/winit-0.30.13
    patch -d $W -p0 < patches/winit-0.30.13-i386-cgfloat.patch

Then build, e.g.:

    SDKROOT=~/sdk/MacOSX10.13.sdk cargo +nightly build --release \\
      -Zbuild-std=std,panic_abort --target i686-apple-darwin -p tobii-calibrate
