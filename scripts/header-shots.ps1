# Renders the sample header at several window widths, at 100 % and 125 % zoom, into shots/header/ (git-ignored).
# Run after `cargo build --release`. 125 % widths are window points; the content is width / 1.25 pt wide.
Set-Location (Join-Path $PSScriptRoot '..')
$exe = Join-Path ($env:CARGO_TARGET_DIR ?? 'target') 'release\webox.exe'
New-Item -ItemType Directory -Force shots\header | Out-Null
foreach ($zoom in '1', '1.25') {
  foreach ($w in 1600, 1280, 1100, 1000, 760) {
    $env:WEBOX_SHOT_SIZE = "${w}x720"
    $env:WEBOX_SHOT_ZOOM = $zoom
    & $exe --sample --screenshot "shots\header\w${w}_z$($zoom.Replace('.', '')).png" 2>&1 | Out-Null
  }
}
Remove-Item env:WEBOX_SHOT_SIZE, env:WEBOX_SHOT_ZOOM
