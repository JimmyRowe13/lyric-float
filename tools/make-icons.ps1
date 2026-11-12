# Generates assets/logo.png (512x512) so `pnpm tauri icon assets/logo.png` has
# something to chew on. Shaped like two lyric bars: a long one and a short one.
# ASCII-only (Windows PowerShell 5.1 decodes BOM-less scripts as ANSI).

param(
    [int]$Size = 512,
    [string]$Out = ""
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$projRoot = Split-Path -Parent $root
$assets = Join-Path $projRoot 'assets'
if (-not (Test-Path $assets)) { New-Item -ItemType Directory -Path $assets | Out-Null }
if ([string]::IsNullOrWhiteSpace($Out)) { $Out = Join-Path $assets 'logo.png' }

$bmp = New-Object System.Drawing.Bitmap $Size, $Size
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias

# transparent background with a dark rounded square
$g.Clear([System.Drawing.Color]::Transparent)
$bg = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 18, 20, 28))
$radius = [int]($Size * 0.22)
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$d = $radius * 2
$path.AddArc(0, 0, $d, $d, 180, 90)
$path.AddArc($Size - $d, 0, $d, $d, 270, 90)
$path.AddArc($Size - $d, $Size - $d, $d, $d, 0, 90)
$path.AddArc(0, $Size - $d, $d, $d, 90, 90)
$path.CloseFigure()
$g.FillPath($bg, $path)

# two lyric bars
$barColor = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 124, 227, 255))
$muted = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(140, 255, 255, 255))
$barH = [int]($Size * 0.085)
$barR = $barH / 2
$x = [int]($Size * 0.18)
$w1 = [int]($Size * 0.64)
$w2 = [int]($Size * 0.40)
$y1 = [int]($Size * 0.42)
$y2 = $y1 + $barH * 2

function New-Bar($bx, $by, $bw, $bh, $brush) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $r = $bh / 2
    $p.AddArc($bx, $by, $bh, $bh, 90, 180)
    $p.AddArc($bx + $bw - $bh, $by, $bh, $bh, 270, 180)
    $p.CloseFigure()
    return $p
}

$p1 = New-Bar $x $y1 $w1 $barH $barColor
$g.FillPath($barColor, $p1)
$p2 = New-Bar $x $y2 $w2 $barH $muted
$g.FillPath($muted, $p2)

$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Output ("wrote " + $Out + " (" + $Size + "x" + $Size + ")")
Write-Output "next: pnpm tauri icon assets/logo.png"
