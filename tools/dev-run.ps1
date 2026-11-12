# Build + launch + observe, for bisecting startup problems.
# All paths are derived from this file's location; nothing is machine-specific.
#
# Usage: powershell -File tools/dev-run.ps1 [-WaitSeconds 10]
#
# Expects the usual Rust toolchain on PATH (or CARGO_HOME set in the environment).

param(
    [int]$WaitSeconds = 10,
    [string]$Log = ""
)

$ErrorActionPreference = 'Continue'

$repoRoot = Split-Path -Parent $PSScriptRoot
$srcTauri = Join-Path $repoRoot 'src-tauri'
if ([string]::IsNullOrWhiteSpace($Log)) {
    $Log = Join-Path ([System.IO.Path]::GetTempPath()) 'lyric-float-run.log'
}

$cargo = (Get-Command cargo.exe -ErrorAction SilentlyContinue).Source
if (-not $cargo) { throw 'cargo not found on PATH' }

Push-Location $srcTauri
& $cargo build 2>&1 | Select-String -Pattern 'error|Finished|Compiling lyric' | Select-Object -Last 6
$buildExit = $LASTEXITCODE
Pop-Location
"BUILD_EXIT=$buildExit"
if ($buildExit -ne 0) { return }

$exe = Join-Path $srcTauri 'target\debug\lyric-float.exe'
if (-not (Test-Path $exe)) { throw "built binary not found: $exe" }

if (Test-Path $Log) { Remove-Item $Log -Force }
$p = Start-Process -FilePath $exe -PassThru -RedirectStandardError $Log
Start-Sleep -Seconds $WaitSeconds

if ($p.HasExited) {
    "EXITED code=$($p.ExitCode) after ${WaitSeconds}s"
    if (Test-Path $Log) {
        '-- stderr --'
        Get-Content $Log | Select-Object -First 8
    }
} else {
    $proc = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
    "ALIVE pid=$($p.Id) title='$($proc.MainWindowTitle)'"
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    'killed'
}
