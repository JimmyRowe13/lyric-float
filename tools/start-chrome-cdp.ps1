# Launches Chrome with the DevTools protocol enabled and opens Apple Music web.
# Used by LyricFloat's CDP provider (milestone M3) to read precise playback
# position and to capture Apple's own word-level TTML lyrics.
#
# The profile is separate on purpose: Chrome refuses --remote-debugging-port when
# it would have to share a profile with an already running instance.
#
# ASCII only (Windows PowerShell 5.1 decodes BOM-less scripts as ANSI).

param(
    [int]$Port = 9222,
    [string]$ProfileDir = "",
    [string]$Url = "https://music.apple.com/",
    [switch]$Wait
)

$ErrorActionPreference = 'Stop'

$candidates = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
)
$browser = $null
foreach ($c in $candidates) { if (Test-Path $c) { $browser = $c; break } }
if (-not $browser) { throw 'no chrome/msedge found' }

if ([string]::IsNullOrWhiteSpace($ProfileDir)) {
    $base = if ($env:APPDATA) { $env:APPDATA } else { "." }
    $ProfileDir = Join-Path $base 'lyric-float\chrome-cdp-profile'
}
if (-not (Test-Path $ProfileDir)) { New-Item -ItemType Directory -Force -Path $ProfileDir | Out-Null }

$chromeArgs = @(
    "--remote-debugging-port=$Port",
    "--user-data-dir=$ProfileDir",
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-features=Translate',
    '--new-window',
    $Url
)

Write-Output ("browser: " + $browser)
Write-Output ("profile: " + $ProfileDir)
Write-Output ("cdp:     http://127.0.0.1:" + $Port + "/json")

$proc = Start-Process -FilePath $browser -ArgumentList $chromeArgs -PassThru

if ($Wait) {
    Write-Output 'waiting for the devtools endpoint...'
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        try {
            $targets = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 2
            $page = $targets | Where-Object { $_.type -eq 'page' } | Select-Object -First 1
            if ($page) {
                Write-Output ("target: " + $page.url)
                Write-Output ("ws:     " + $page.webSocketDebuggerUrl)
                break
            }
        } catch { }
    }
}

# Reminder: use the same profile next time, otherwise a second instance is started
# and the debug port moves to the newer process.
