<#
.SYNOPSIS
  Fills or refreshes assets/cisco/ with the official Webex app's files (list: scripts/cisco-assets.txt).

.DESCRIPTION
  These files are in the repository (docs/cisco-assets.md); use this to refresh them. Three sources, use any combination:
    -FromCommit <rev>   a commit of a private checkout that still had them at their old paths under assets/
    -FromFolder <dir>   another assets/cisco folder (e.g. from a different checkout or a backup)
    -FromInstall        the sounds from the installed official app (Windows, %LOCALAPPDATA%\CiscoSparkLauncher)
  Existing files are kept unless -Force. Afterwards `cargo build` embeds them (build.rs watches assets/).

.EXAMPLE
  pwsh scripts/restore-cisco-assets.ps1 -FromCommit d12f9b0
  pwsh scripts/restore-cisco-assets.ps1 -FromInstall
#>
param([string]$FromCommit, [string]$FromFolder, [switch]$FromInstall, [switch]$Force)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dest = Join-Path $repo 'assets/cisco'
$list = Get-Content (Join-Path $PSScriptRoot 'cisco-assets.txt') | Where-Object { $_.Trim() }

# where each file lived before it moved to assets/cisco/
function Old-Path([string]$rel) {
    switch -regex ($rel) {
        '^images/giphy/(.+)$' { return "assets/giphy/$($Matches[1])" }
        '^images/(.+)$' { return "assets/$($Matches[1])" }
        default { return "assets/$rel" }
    }
}

# -FromCommit: one `git archive` of the old asset paths into a temp folder (a single git call, seconds), then plain copies.
$fromTree = $null
if ($FromCommit) {
    $fromTree = Join-Path ([IO.Path]::GetTempPath()) ("webox-cisco-{0}" -f [guid]::NewGuid())
    [void][IO.Directory]::CreateDirectory($fromTree)
    $tar = "$fromTree.tar"
    $paths = @($list | ForEach-Object { (Old-Path $_) -replace '/[^/]+$', '' } | Sort-Object -Unique) +
             @($list | Where-Object { $_ -notmatch '/' -or $_ -like 'images/*' } | ForEach-Object { Old-Path $_ })
    $paths = @($paths | Where-Object { $_ -ne 'assets' } | Sort-Object -Unique)
    git -C $repo archive --format=tar -o $tar $FromCommit -- @paths
    if ($LASTEXITCODE) { throw "git archive of $FromCommit failed" }
    tar -xf $tar -C $fromTree
    Remove-Item $tar
}

$done = 0; $missing = @()
foreach ($rel in $list) {
    $out = Join-Path $dest $rel
    if ((Test-Path $out) -and -not $Force) { continue }
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($out))
    $ok = $false
    if ($FromFolder -and (Test-Path (Join-Path $FromFolder $rel))) { Copy-Item (Join-Path $FromFolder $rel) $out -Force; $ok = $true }
    if (-not $ok -and $fromTree) {
        $src = Join-Path $fromTree (Old-Path $rel)
        if (Test-Path $src) { Copy-Item $src $out -Force; $ok = $true }
    }
    if (-not $ok -and $FromInstall -and $rel -like 'sounds/*' -and $env:LOCALAPPDATA) {
        $name = Split-Path $rel -Leaf
        $base = Join-Path $env:LOCALAPPDATA 'CiscoSparkLauncher\dynamic_dependencies'
        foreach ($g in 'ringtones', 'momentum_ringtones', 'sounds') {
            $src = Join-Path $base "$g\$name"
            if (Test-Path $src) { Copy-Item $src $out -Force; $ok = $true; break }
        }
    }
    if ($ok) { $done++ } else { $missing += $rel }
}
if ($fromTree) { Remove-Item -Recurse -Force $fromTree }
Write-Host "restored $done file(s) into assets/cisco"
if ($missing.Count) { Write-Host "$($missing.Count) not found (the app falls back for these); first: $($missing[0])" }
