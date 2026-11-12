<#
.SYNOPSIS
Makes Tobii Pro Eye Tracker Manager for Windows list the Eye Tracker 5
(serial prefix IS5FF) that ttpbridge serves.

.DESCRIPTION
PETM's interface only lists trackers whose serial prefix is in a built-in
list (plus one downloaded at start-up), and IS5FF is in neither. For PETM
2.6.1 the obsolete entry "TX300" becomes "IS5FF": five bytes of the app's
own resources\app.asar, same length, nothing else moves. The original is kept
as app.asar.orig and -Restore puts it back. Any other app.asar is refused
rather than guessed at.

PETM 2.7.2 ships Tobii Pro SDK 2.0, which no longer talks TTP; use 2.6.1 on
Windows.

.PARAMETER AppDir
PETM's install folder. Defaults to the per-user install location.

.PARAMETER Restore
Put the original app.asar back.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File patch-petm.ps1
powershell -ExecutionPolicy Bypass -File patch-petm.ps1 -Restore
#>
param(
    [string]$AppDir = "$env:LOCALAPPDATA\Programs\TobiiProEyeTrackerManager",
    [switch]$Restore
)
$ErrorActionPreference = "Stop"

# SHA-256 of the unmodified app.asar of PETM 2.6.1 for Windows, and the
# offset of the opening quote of its "TX300" entry.
$ExpectedSha256 = "9945f8eade70d108efe26ab735888038a30e798011b25580aa9b461f97118d17"
$EntryOffset = 5521729
$Old = '"TX300"'
$New = '"IS5FF"'

$asar = Join-Path $AppDir "resources\app.asar"
$backup = "$asar.orig"
if (-not (Test-Path $asar)) { throw "$asar not found" }
$fullDir = (Resolve-Path $AppDir).Path
$running = Get-Process TobiiProEyeTrackerManager -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($fullDir, [System.StringComparison]::OrdinalIgnoreCase) }
if ($running) { throw "Quit Eye Tracker Manager first" }

if ($Restore) {
    if (-not (Test-Path $backup)) { throw "no backup at $backup" }
    Move-Item -Force $backup $asar
    Write-Output "restored the original app.asar"
    exit 0
}

function Read-At([string]$Path, [long]$Offset, [int]$Count) {
    $fs = [System.IO.File]::Open($Path, "Open", "Read", "ReadWrite")
    try {
        $buf = New-Object byte[] $Count
        $fs.Seek($Offset, "Begin") | Out-Null
        $n = $fs.Read($buf, 0, $Count)
        [System.Text.Encoding]::ASCII.GetString($buf, 0, $n)
    } finally { $fs.Close() }
}

if ((Read-At $asar $EntryOffset $New.Length) -eq $New) {
    Write-Output "already patched"
    exit 0
}
$sha = (Get-FileHash $asar -Algorithm SHA256).Hash.ToLower()
if ($sha -ne $ExpectedSha256) {
    throw "this app.asar is not the one of PETM 2.6.1 for Windows (sha256 $sha); not patching"
}
if ((Read-At $asar $EntryOffset $Old.Length) -ne $Old) {
    throw "unexpected bytes at offset $EntryOffset"
}

Copy-Item $asar $backup -Force
$fs = [System.IO.File]::Open($asar, "Open", "ReadWrite")
try {
    $fs.Seek($EntryOffset + 1, "Begin") | Out-Null
    $bytes = [System.Text.Encoding]::ASCII.GetBytes($New.Trim('"'))
    $fs.Write($bytes, 0, $bytes.Length)
} finally { $fs.Close() }

if ((Read-At $asar $EntryOffset $New.Length) -ne $New) {
    Move-Item -Force $backup $asar
    throw "patch did not apply; original restored"
}
Write-Output "patched: IS5FF added to PETM's supported trackers (backup: $backup)"
