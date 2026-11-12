# Pushes the working tree to GitHub through the REST API.
#
# Why this exists: on networks where github.com:443 is blocked (git push dies with
# "Connection was reset" / SSL_ERROR_SYSCALL) but api.github.com is reachable, the
# normal remote cannot be used. This script builds one commit from the tracked files
# and moves `main` to it — no git transport involved.
#
# Requirements: gh (authenticated). The repository must already exist and be non-empty.
#
# Usage: powershell -File tools/publish-via-api.ps1 -Repo owner/name [-Branch main] [-Message "..."]

param(
    [Parameter(Mandatory = $true)][string]$Repo,
    [string]$Branch = 'main',
    [string]$Message = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$utf8 = New-Object System.Text.UTF8Encoding($false)
$work = Join-Path ([System.IO.Path]::GetTempPath()) 'lyric-float-publish'
New-Item -ItemType Directory -Force -Path $work | Out-Null

if ([string]::IsNullOrWhiteSpace($Message)) {
    $Message = "Update from working tree ($(Get-Date -Format 'yyyy-MM-dd HH:mm'))"
}

Push-Location $root
$files = git ls-files
Pop-Location

# Two steps on purpose. The trees API takes either a blob `sha` or inline `content`,
# and its inline `content` is stored verbatim — feeding it a base64 string (which is
# what the blobs API wants) put base64 text into every file in the repository. Blobs
# first, then a tree that references their shas, keeps the two encodings apart.
$entries = New-Object System.Collections.ArrayList
$count = 0
foreach ($f in $files) {
    $p = Join-Path $root $f
    if (-not (Test-Path $p)) { continue }
    $count++
    $b64 = [Convert]::ToBase64String([System.IO.File]::ReadAllBytes($p))
    $blobPath = Join-Path $work 'blob.json'
    [System.IO.File]::WriteAllText($blobPath, (@{ content = $b64; encoding = 'base64' } | ConvertTo-Json -Compress), $utf8)
    $sha = (gh api -X POST "repos/$Repo/git/blobs" --input $blobPath | ConvertFrom-Json).sha
    if (-not $sha) { throw "blob upload failed for $f" }
    [void]$entries.Add([ordered]@{ path = ($f -replace '\\', '/'); mode = '100644'; type = 'blob'; sha = $sha })
}
Write-Output "files: $count"

$treePath = Join-Path $work 'tree.json'
[System.IO.File]::WriteAllText($treePath, ([ordered]@{ tree = $entries } | ConvertTo-Json -Depth 6 -Compress), $utf8)

$treeSha = (gh api -X POST "repos/$Repo/git/trees" --input $treePath | ConvertFrom-Json).sha
Write-Output "tree: $treeSha"

$parentSha = gh api "repos/$Repo/git/ref/heads/$Branch" --jq '.object.sha'
Write-Output "parent: $parentSha"

$commitPath = Join-Path $work 'commit.json'
[System.IO.File]::WriteAllText($commitPath, ([ordered]@{
    message = $Message
    tree    = $treeSha
    parents = @($parentSha)
} | ConvertTo-Json -Compress), $utf8)
$commitSha = (gh api -X POST "repos/$Repo/git/commits" --input $commitPath | ConvertFrom-Json).sha
Write-Output "commit: $commitSha"

$refPath = Join-Path $work 'ref.json'
[System.IO.File]::WriteAllText($refPath, ([ordered]@{ sha = $commitSha; force = $true } | ConvertTo-Json -Compress), $utf8)
gh api -X PATCH "repos/$Repo/git/refs/heads/$Branch" --input $refPath | Out-Null

Write-Output "pushed: https://github.com/$Repo/commit/$commitSha"
