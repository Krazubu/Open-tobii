# Contributing

Thanks for looking. This is a reverse-engineered driver for one piece of
hardware, which shapes what help is useful and how a change gets judged.

## What this project is bound by

`libtobii.so` presents **exactly the 199 entry points of Tobii's Stream Engine
4.25.0.3**, so that a program built for the Windows SDK links and runs against
it. That contract is not a style preference: `make verify-abi` fails if a
symbol appears, disappears or stops being declared in the headers, and CI runs
it on every pull request.

Where an answer cannot be the Stream Engine's, it diverges **deliberately and
in writing**. [docs/stream-engine-api.md](docs/stream-engine-api.md) says
what the entry points answer, by name or by family, and where that differs
from Windows. `tools/abi/README.md` carries 4.1.0.3's derivation, with the DLL
addresses it came from, and the doc comments in `crates/tobii-ffi/` (`src/`
and `include/tobii/`) carry that of what 4.25.0.3 adds or changes, with the
DLL addresses where they give them. A change that adds a divergence is
expected to add its entry. "It seemed more sensible" is a reason; it just has
to be written down.

## Before you open a pull request

Run the same gates CI runs:

```sh
make check        # rustfmt, clippy -D warnings, the whole test suite, the docs,
                  # and the checks of the udev rules and the .desktop file
make verify-abi   # the 199 exports, and a C program compiled against the headers
```

CI runs them with Rust 1.99.0, the toolchain the release container pins, and
a second job checks that the workspace still builds with 1.88, the minimum it
declares (`cargo +1.88 check --workspace --all-targets --locked`).

Neither needs a tracker. Ten tests whose inputs are captures of a face read
them from the paths in `TOBII_IMAGE83_FIXTURE` and `TOBII_POSE_PGM_FIXTURE`,
and skip without them.

A few things the reviewer will look for:

- **A test that fails without the change.** For a bug fix, the test that
  reproduces it; for a behaviour change, the test that pins the new answer.
  The suite is large (1145 tests) because that is how this codebase is kept
  honest about a device nobody can mock.
- **No `unwrap()` or `expect()` on a path that can fail at runtime.** `expect`
  is for invariants whose violation is a bug; everything else returns an
  error with context.
- **`// SAFETY:` on every `unsafe` block**, saying what makes it sound — most
  of the FFI surface is `unsafe` by necessity and the comments are what keep
  it reviewable.
- **A claim about the Stream Engine carries its evidence.** If you write that
  the DLL answers something, cite the address you read it at (image base
  `0x180000000`) and say whether you read it or inferred it. The existing docs
  do this throughout; it is the only reason they can be checked years later.
- **The daemon's socket protocol has readers this workspace does not build.**
  The gaze switcher extension (`gaze-switcher/`) speaks it in GJS; its own
  tests run with `make -C gaze-switcher test`, which needs gjs, and CI does
  not run them. tobii-hub, a Flutter plugin in its own repository, takes
  `tobii-ipc` by git at a release tag.
  `crates/tobii-ipc/tests/out_of_tree_consumers.rs` imports what tobii-hub
  imports and decodes frames as its dispatch does, so renaming, retyping or
  removing one of those items, or adding a field to `HeadPose`, fails there.
  `tobii-ipc`'s crate docs say how older and newer peers keep working
  together.

Commit messages are prose, not a change-type prefix: say what changed and
*why*, and name what you decided against. The log is the project's design
record.

## Changes that need hardware

Anything touching `tobii-usb`, the daemon's engine or a tracker command can
only be judged on a real Eye Tracker 5. If you have one, say in the pull
request what you ran and what the tracker answered, and attach the relevant
journal lines. If you do not, say that too — a well-reasoned patch with "not
tested on hardware" is welcome and will simply wait for someone who can run
it.

**Never add a probe that writes to the tracker without a dry run first.**
`tobii5-init-replay state-probe` exists for this: a closed whitelist of
commands, where a write that changes what the tracker reports is refused
unless its revert is in the same run (the custom command, which changes
nothing it reports, has none). By default it only prints the bytes;
`--run` sends them and needs a `--log` to record every message, and a write
also needs `--allow-writes`. `stream-probe` starts and stops only a stream
from the tracker's own catalogue (commands 1220 and 1230) and logs everything
around it. Extend them rather than writing something ad hoc.

## What is especially welcome

- **Another tracker.** Only the Eye Tracker 5 (`2104:0313`) is supported and
  verified. A 5L, 4C or anything else would need its stream catalogue
  (command 1200), its generation string (1420) and its property list (1330)
  captured — that alone, even without code, is a real contribution. See
  `tools/abi/README.md` for how those are read.
- **Distribution packaging.** `make package` builds a `.deb`, `.rpm` and
  tarball. `packaging/aur/` holds a recipe for an AUR `tobii-linux-bin`
  package, checked against v0.2.0 and not published, and a COPR build is
  blocked because the build downloads ONNX Runtime; `packaging/aur/README.md`
  says what each needs. Packages that track releases would help more people
  than most code changes.
- **Anything in the README's "Limitations".** Those are known gaps, not
  oversights, and each says what is missing.

## Research material

`fixtures/` is gitignored and holds the captures, logs and analysis the
driver was built from. It is not in the repository (some of it carries the
tracker's serial number, and all of it is large). The write-ups reference
paths inside it; if you need one to follow an argument, ask.

## Reporting a problem

See `.github/ISSUE_TEMPLATE/`. The short version: a driver bug without the
daemon's journal, your tracker's firmware and the version you are running is
usually not reproducible, and the template asks for exactly those.

## Licence

MIT, as the repository. By contributing you agree your work is published under
it.
