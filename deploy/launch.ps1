# LyricFloat launcher.
#
# Starts the debug browser first (its own profile, so the Apple Music login persists),
# then the overlay. Both steps are idempotent: if something is already listening on the
# port, Chrome just forwards the new window into the running instance.
#
# Point a desktop shortcut at:
#   powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File launch.ps1

param(
    [int]$Port = 9222,
    [string]$ProfileDir = "",
    [string]$Url = "https://music.apple.com/",
    [switch]$ForceBrowser
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path

if ([string]::IsNullOrWhiteSpace($ProfileDir)) {
    $ProfileDir = Join-Path $env:APPDATA 'lyric-float\chrome-cdp-profile'
}

$candidates = @(
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe",
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
)
$browser = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1

$listening = $false
try {
    $listening = (Test-NetConnection -ComputerName 127.0.0.1 -Port $Port -InformationLevel Quiet -WarningAction SilentlyContinue)
} catch { }

if ($ForceBrowser -or -not $listening) {
    if (-not $browser) {
        Write-Output "[launcher] no chrome/edge found"
    } else {
        if (-not (Test-Path $ProfileDir)) { New-Item -ItemType Directory -Force -Path $ProfileDir | Out-Null }
        Start-Process -FilePath $browser -ArgumentList @(
            "--remote-debugging-port=$Port",
            "--user-data-dir=$ProfileDir",
            '--no-first-run',
            '--no-default-browser-check',
            '--new-window',
            $Url
        )
        # give the endpoint a moment; the overlay retries anyway
        Start-Sleep -Seconds 2
    }
}

$exe = Join-Path $root 'lyric-float.exe'
if (Test-Path $exe) {
    Start-Process -FilePath $exe
} else {
    Write-Output "[launcher] lyric-float.exe not found next to launch.ps1"
}
