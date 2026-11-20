<#
.SYNOPSIS
  Packs assets/icon/webox-<size>.png into one .ico (PNG entries) for the installer and its shortcuts.
.EXAMPLE
  pwsh packaging/windows/make-ico.ps1 -Out dist/webox.ico
#>
param([Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$pngs = foreach ($s in $sizes) { , [IO.File]::ReadAllBytes((Join-Path $root "assets/icon/webox-$s.png")) }
$ms = [IO.MemoryStream]::new()
$w = [IO.BinaryWriter]::new($ms)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)  # ICONDIR: reserved, type 1 = icon, count
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256))                # width, height (0 = 256)
    $w.Write([byte]0); $w.Write([byte]0)                                    # palette, reserved
    $w.Write([uint16]1); $w.Write([uint16]32)                               # planes, bits per pixel
    $w.Write([uint32]$pngs[$i].Length); $w.Write([uint32]$offset)
    $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $w.Write($p) }
$w.Flush()
$dir = Split-Path -Parent $Out
if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
[IO.File]::WriteAllBytes((Join-Path (Resolve-Path ($dir ? $dir : '.')).Path (Split-Path -Leaf $Out)), $ms.ToArray())
Write-Host "$Out ($($sizes.Count) sizes)"
