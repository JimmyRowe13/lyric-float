# lyric-float M0 harness: run a controlled local web player and watch SMTC.
# Answers: while a Chromium page is actively playing and publishing
# setPositionState(), does the Windows SMTC timeline (Position / LastUpdatedTime)
# actually advance and refresh?
#
# Uses an isolated Chrome profile so the user's own browser is untouched.
# ASCII only (Windows PowerShell 5.1 ANSI decoding of BOM-less scripts).

param(
    [int]$Samples = 12,
    [int]$IntervalMs = 700,
    [int]$Seconds = 15
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$sandbox = Join-Path $root 'sandbox'
$wav = Join-Path $sandbox 'tone.wav'
$page = Join-Path $sandbox 'player.html'

if (-not (Test-Path $sandbox)) { New-Item -ItemType Directory -Path $sandbox | Out-Null }

if (-not (Test-Path $wav)) {
    Write-Output "GEN wav seconds=$Seconds"
    $sr = 8000
    $n = $sr * $Seconds
    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter($ms)
    $bw.Write([System.Text.Encoding]::ASCII.GetBytes('RIFF'))
    $bw.Write([int](36 + $n * 2))
    $bw.Write([System.Text.Encoding]::ASCII.GetBytes('WAVE'))
    $bw.Write([System.Text.Encoding]::ASCII.GetBytes('fmt '))
    $bw.Write([int]16)
    $bw.Write([int16]1)
    $bw.Write([int16]1)
    $bw.Write([int]$sr)
    $bw.Write([int]($sr * 2))
    $bw.Write([int16]2)
    $bw.Write([int16]16)
    $bw.Write([System.Text.Encoding]::ASCII.GetBytes('data'))
    $bw.Write([int]($n * 2))
    $twoPi = 2 * [math]::PI
    for ($i = 0; $i -lt $n; $i++) {
        $v = [int16](3000 * [math]::Sin($twoPi * 440 * $i / $sr))
        $bw.Write($v)
    }
    $bw.Flush()
    [System.IO.File]::WriteAllBytes($wav, $ms.ToArray())
    $ms.Dispose()
}

$profile = Join-Path $env:TEMP 'lyric-float-m0-profile'
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

$url = ([System.Uri]$page).AbsoluteUri
$chromeArgs = @(
    "--user-data-dir=$profile",
    '--autoplay-policy=no-user-gesture-required',
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-features=Translate',
    '--new-window',
    $url
)

Write-Output ("BROWSER " + $browser)
Write-Output ("PROFILE " + $profile)
Write-Output ("URL " + $url)

Start-Process -FilePath $browser -ArgumentList $chromeArgs | Out-Null

try {
    Write-Output 'WAIT warmup 6s'
    Start-Sleep -Seconds 6

    & (Join-Path $root 'probe.ps1') -Samples $Samples -IntervalMs $IntervalMs
}
finally {
    Start-Sleep -Milliseconds 300
    $killed = 0
    Get-CimInstance Win32_Process -Filter "Name='chrome.exe' OR Name='msedge.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -and $_.CommandLine.Contains($profile) } |
        ForEach-Object {
            Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
            $killed++
        }
    Write-Output ("CLEANUP killed=" + $killed)
}
