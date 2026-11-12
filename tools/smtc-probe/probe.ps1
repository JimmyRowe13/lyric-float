# lyric-float M0 probe
# Reads Windows System Media Transport Controls (SMTC) sessions and samples the
# timeline repeatedly, to answer one question: does a Chromium browser report a
# usable playback position, and does it advance over time?
#
# ASCII-only on purpose: Windows PowerShell 5.1 decodes BOM-less scripts as ANSI.
# Run with Windows PowerShell (powershell.exe), not pwsh 7 (no WinRT projection).

param(
    [int]$Samples = 5,
    [int]$IntervalMs = 2000,
    [switch]$Drive
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Runtime.WindowsRuntime | Out-Null

$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and
        $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
    })[0]

function Await($op, $resultType) {
    $task = $asTaskGeneric.MakeGenericMethod($resultType).Invoke($null, @($op))
    $task.Wait(-1) | Out-Null
    $task.Result
}

$sessionMgrType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType = WindowsRuntime]
$mediaPropsType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties, Windows.Media.Control, ContentType = WindowsRuntime]

$mgr = Await ($sessionMgrType::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
$sessions = $mgr.GetSessions()

Write-Output "PROBE version=1"
Write-Output ("SESSION_COUNT " + $sessions.Count)

function Format-Span($ts) {
    if ($null -eq $ts) { return 'null' }
    return ('{0:0.000}' -f $ts.TotalSeconds)
}

foreach ($s in $sessions) {
    $app = $s.SourceAppUserModelId
    $title = ''
    $artist = ''
    $album = ''
    try {
        $props = Await ($s.TryGetMediaPropertiesAsync()) ($mediaPropsType)
        $title = $props.Title
        $artist = $props.Artist
        $album = $props.AlbumTitle
    }
    catch {
        $title = ('<err:' + $_.Exception.Message + '>')
    }

    $info = $s.GetPlaybackInfo()
    $status = $info.PlaybackStatus
    $rate = $info.PlaybackRate

    Write-Output "---"
    Write-Output ("APP " + $app)
    Write-Output ("TITLE " + $title)
    Write-Output ("ARTIST " + $artist)
    Write-Output ("ALBUM " + $album)
    Write-Output ("STATUS " + $status)
    Write-Output ("RATE " + $rate)

    $weStarted = $false
    if ($Drive -and $status -ne 'Playing') {
        try {
            $ok = Await ($s.TryPlayAsync()) ([bool])
            Write-Output ("DRIVE_PLAY ok=" + $ok)
            $weStarted = $ok
            Start-Sleep -Milliseconds 1500
        }
        catch {
            Write-Output ("DRIVE_PLAY ERROR " + $_.Exception.Message)
        }
    }

    for ($i = 0; $i -lt $Samples; $i++) {
        $samples_ok = $false
        try {
            $tl = $s.GetTimelineProperties()
            $pos = $tl.Position
            $start = $tl.StartTime
            $end = $tl.EndTime
            $luRaw = $tl.LastUpdatedTime
            $luType = if ($null -eq $luRaw) { 'null' } else { $luRaw.GetType().FullName }
            $luText = if ($null -eq $luRaw) { 'null' } elseif ($luRaw -is [datetimeoffset]) { $luRaw.ToString('o') } elseif ($luRaw -is [datetime]) { $luRaw.ToString('o') } else { [string]$luRaw }
            $drift = 'n/a'
            if ($luRaw -is [datetimeoffset]) {
                $drift = ('{0:0.000}' -f ((Get-Date) - $luRaw.LocalDateTime).TotalSeconds)
            }
            elseif ($luRaw -is [datetime]) {
                $drift = ('{0:0.000}' -f ((Get-Date) - $luRaw).TotalSeconds)
            }
            $liveStatus = $s.GetPlaybackInfo().PlaybackStatus
            $now = (Get-Date).ToString('HH:mm:ss.fff')
            Write-Output ("SAMPLE i=$i wall=$now status=$liveStatus pos=" + (Format-Span $pos) + " start=" + (Format-Span $start) + " end=" + (Format-Span $end) + " lastUpdatedType=$luType lastUpdated=$luText secondsSinceUpdate=$drift")
            $samples_ok = $true
        }
        catch {
            Write-Output ("SAMPLE i=$i ERROR " + $_.Exception.Message)
        }
        if ($i -lt ($Samples - 1)) { Start-Sleep -Milliseconds $IntervalMs }
    }

    if ($weStarted) {
        try {
            $ok = Await ($s.TryPauseAsync()) ([bool])
            Write-Output ("DRIVE_PAUSE_RESTORE ok=" + $ok)
        }
        catch {
            Write-Output ("DRIVE_PAUSE_RESTORE ERROR " + $_.Exception.Message)
        }
    }
}

Write-Output "---"
Write-Output "DONE"
