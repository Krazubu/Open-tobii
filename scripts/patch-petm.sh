#!/bin/bash
# Adds the Eye Tracker 5's serial prefix (IS5FF) to the list of trackers that
# Tobii Pro Eye Tracker Manager 2.6.1 shows in its user interface.
#
# PETM's interface only lists trackers whose serial prefix is in a built-in
# list (plus one downloaded at start-up), and IS5FF is in neither. This
# replaces one obsolete entry of the built-in list, "TX300", with "IS5FF":
# five bytes of the app's own copy of app.asar, same length, nothing else
# moves. The original is kept next to it as app.asar.orig.
#
# Usage: patch-petm.sh "/path/to/Tobii Pro Eye Tracker Manager.app"
#        patch-petm.sh --restore "/path/to/Tobii Pro Eye Tracker Manager.app"
#
# Quit PETM first. If the app lives in /Applications you may need sudo.
set -euo pipefail

# SHA-256 of the unmodified app.asar of PETM 2.6.1 (macOS), and where the
# entry sits in it. Any other version is refused rather than guessed at.
EXPECTED_SHA256=041b1ebe0a754940c7d420279d7f565a3c69c1d5feb19eee85db0d04a4eaf2cb
ENTRY_OFFSET=5515007   # offset of the opening quote of "TX300"

restore=0
if [ "${1:-}" = "--restore" ]; then
    restore=1
    shift
fi
app="${1:?usage: patch-petm.sh [--restore] /path/to/PETM.app}"
asar="$app/Contents/Resources/app.asar"
backup="$asar.orig"

[ -f "$asar" ] || { echo "error: $asar not found" >&2; exit 1; }

if [ "$restore" = 1 ]; then
    [ -f "$backup" ] || { echo "error: no backup at $backup" >&2; exit 1; }
    mv -f "$backup" "$asar"
    echo "restored the original app.asar"
    exit 0
fi

entry=$(dd if="$asar" bs=1 skip="$ENTRY_OFFSET" count=7 2>/dev/null)
if [ "$entry" = '"IS5FF"' ]; then
    echo "already patched"
    exit 0
fi

sha=$(shasum -a 256 "$asar" | cut -d' ' -f1)
if [ "$sha" != "$EXPECTED_SHA256" ]; then
    echo "error: this app.asar is not the one of PETM 2.6.1 (sha256 $sha); not patching" >&2
    exit 1
fi
if [ "$entry" != '"TX300"' ]; then
    echo "error: unexpected bytes at offset $ENTRY_OFFSET: $entry" >&2
    exit 1
fi

cp -p "$asar" "$backup"
printf 'IS5FF' | dd of="$asar" bs=1 seek=$((ENTRY_OFFSET + 1)) conv=notrunc 2>/dev/null

after=$(dd if="$asar" bs=1 skip="$ENTRY_OFFSET" count=7 2>/dev/null)
[ "$after" = '"IS5FF"' ] || { echo "error: patch did not apply; restoring" >&2; mv -f "$backup" "$asar"; exit 1; }
echo "patched: IS5FF added to PETM's supported trackers (backup: $backup)"
