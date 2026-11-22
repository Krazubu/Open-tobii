#!/bin/bash
# Makes Tobii Pro Eye Tracker Manager list the Eye Tracker 5 (serial prefix
# IS5FF) that ttpbridge serves.
#
# PETM 2.6.1: its interface only lists trackers whose serial prefix is in a
#   built-in list (plus one downloaded at start-up), and IS5FF is in neither.
#   The obsolete entry "TX300" becomes "IS5FF": five bytes, same length.
#
# PETM 2.7.2: the list has no obsolete entry left, so the filter function
#   itself is rewritten, at the same length, to accept IS5FF-* in addition
#   to the list. This version also ships Tobii Pro SDK 2.0, which no longer
#   talks TTP and so cannot see ttpbridge; the SDK module of PETM 2.6.1
#   (stage/SdkJsBinding.app, Pro SDK 1.11) is put in its place, copied from
#   your own PETM 2.6.1 install given with --sdk-from.
#
# Every change keeps a backup (app.asar.orig, SdkJsBinding.app.orig) and
# --restore puts them back.
#
# Usage: patch-petm.sh /path/to/PETM-2.6.1.app
#        patch-petm.sh /path/to/PETM-2.7.2.app --sdk-from /path/to/PETM-2.6.1.app
#        patch-petm.sh --restore /path/to/PETM.app
#
# Quit PETM first. If the app lives in /Applications you may need sudo.
set -euo pipefail

# SHA-256 of the unmodified app.asar of each supported version. Anything
# else is refused rather than guessed at.
SHA_261=041b1ebe0a754940c7d420279d7f565a3c69c1d5feb19eee85db0d04a4eaf2cb
SHA_272=e48c412f707d454e62c513904e92e36a9e236ce7089b36b6fc3d318a49814cbd

# 2.6.1: the "TX300" entry of supportedTrackerPrefixes (offset of its quote).
OFF_261=5515007
OLD_261='"TX300"'
NEW_261='"IS5FF"'

# 2.7.2: the trackerIsSupported function of the interface bundle.
OFF_272=5474272
OLD_272='this.trackerIsSupported=function(R){var l=R.substring(0,R.indexOf("-"));return B.supportedTrackerPrefixes.includes(l)}'
NEW_272='this.trackerIsSupported=function(R){return/^IS5FF-/.test(R)||B.supportedTrackerPrefixes.includes(R.split("-")[0])    }'

# SHA-256 of PETM 2.6.1's SDK module executable, to check --sdk-from.
SHA_SDK_261=95756006c48b256ea4331a97afe879a7ed9167bfb13a2bd5cd95c991ae3a21d8

usage() {
    echo "usage: patch-petm.sh [--restore] /path/to/PETM.app [--sdk-from /path/to/PETM-2.6.1.app]" >&2
    exit 2
}

restore=0
app=""
sdk_from=""
while [ $# -gt 0 ]; do
    case "$1" in
        --restore) restore=1 ;;
        --sdk-from) shift; sdk_from="${1:-}"; [ -n "$sdk_from" ] || usage ;;
        -*) usage ;;
        *) [ -z "$app" ] || usage; app="$1" ;;
    esac
    shift
done
[ -n "$app" ] || usage

asar="$app/Contents/Resources/app.asar"
sdk="$app/Contents/Resources/stage/SdkJsBinding.app"
[ -f "$asar" ] || { echo "error: $asar not found" >&2; exit 1; }

if [ "$restore" = 1 ]; then
    done_any=0
    if [ -f "$asar.orig" ]; then
        mv -f "$asar.orig" "$asar"
        echo "restored the original app.asar"
        done_any=1
    fi
    if [ -d "$sdk.orig" ]; then
        rm -rf "$sdk"
        mv "$sdk.orig" "$sdk"
        echo "restored the original SDK module"
        done_any=1
    fi
    [ "$done_any" = 1 ] || { echo "error: no backup found" >&2; exit 1; }
    exit 0
fi

# read_at OFFSET LENGTH: the bytes of app.asar there, as text.
read_at() {
    dd if="$asar" bs=1 skip="$1" count="$2" 2>/dev/null
}

# write_at OFFSET TEXT: overwrite app.asar in place.
write_at() {
    printf '%s' "$2" | dd of="$asar" bs=1 seek="$1" conv=notrunc 2>/dev/null
}

# patch_asar OFFSET OLD NEW EXPECTED_SHA: same-length replacement with a backup.
patch_asar() {
    local off=$1 old=$2 new=$3 sha_expected=$4
    if [ "$(read_at "$off" ${#new})" = "$new" ]; then
        echo "interface: already patched"
        return 0
    fi
    local sha
    sha=$(shasum -a 256 "$asar" | cut -d' ' -f1)
    [ "$sha" = "$sha_expected" ] || { echo "error: app.asar changed (sha256 $sha); not patching" >&2; exit 1; }
    [ "$(read_at "$off" ${#old})" = "$old" ] || { echo "error: unexpected bytes at offset $off" >&2; exit 1; }
    cp -p "$asar" "$asar.orig"
    write_at "$off" "$new"
    if [ "$(read_at "$off" ${#new})" != "$new" ]; then
        echo "error: patch did not apply; restoring" >&2
        mv -f "$asar.orig" "$asar"
        exit 1
    fi
    echo "interface: IS5FF added to the supported trackers (backup: $asar.orig)"
}

# Which version is it? Recognise patched copies by their patched bytes.
version=""
if [ "$(read_at $OFF_261 ${#NEW_261})" = "$NEW_261" ] || [ "$(read_at $OFF_261 ${#OLD_261})" = "$OLD_261" ]; then
    version=2.6.1
elif [ "$(read_at $OFF_272 ${#NEW_272})" = "$NEW_272" ] || [ "$(read_at $OFF_272 ${#OLD_272})" = "$OLD_272" ]; then
    version=2.7.2
fi

case "$version" in
    2.6.1)
        echo "PETM 2.6.1"
        patch_asar $OFF_261 "$OLD_261" "$NEW_261" $SHA_261
        ;;
    2.7.2)
        echo "PETM 2.7.2"
        [ -n "$sdk_from" ] || { echo "error: PETM 2.7.2 needs --sdk-from /path/to/PETM-2.6.1.app (see the comments at the top)" >&2; exit 1; }
        src="$sdk_from/Contents/Resources/stage/SdkJsBinding.app"
        [ -f "$src/Contents/MacOS/sdk-js-binding" ] || { echo "error: no SDK module in $sdk_from" >&2; exit 1; }
        src_sha=$(shasum -a 256 "$src/Contents/MacOS/sdk-js-binding" | cut -d' ' -f1)
        [ "$src_sha" = "$SHA_SDK_261" ] || { echo "error: $sdk_from is not PETM 2.6.1 (its SDK module differs)" >&2; exit 1; }
        patch_asar $OFF_272 "$OLD_272" "$NEW_272" $SHA_272
        cur_sha=$(shasum -a 256 "$sdk/Contents/MacOS/sdk-js-binding" | cut -d' ' -f1)
        if [ "$cur_sha" = "$SHA_SDK_261" ]; then
            echo "SDK module: already the 2.6.1 one"
        else
            [ -e "$sdk.orig" ] || mv "$sdk" "$sdk.orig"
            rm -rf "$sdk"
            cp -Rp "$src" "$sdk"
            echo "SDK module: replaced by PETM 2.6.1's (backup: $sdk.orig)"
        fi
        ;;
    *)
        echo "error: unsupported PETM version (supported: 2.6.1, 2.7.2)" >&2
        exit 1
        ;;
esac
