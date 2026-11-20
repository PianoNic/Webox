# Renders the Pride Theme (in-tree plugins/pride-theme) with three flags on dark and light into shots/ (git-ignored). Run after
# `cargo build`; uses a scratch config folder, never the real one.
Set-Location (Join-Path $PSScriptRoot '..')
New-Item -ItemType Directory -Force shots | Out-Null
$exe = Join-Path ($env:CARGO_TARGET_DIR ?? 'target') 'debug\webox.exe'
function Shot($name, $flag, $theme, $envs) {
    $s = Join-Path ([IO.Path]::GetTempPath()) 'webox-pride-shot'
    Remove-Item -Recurse -Force $s -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force "$s\WebOx\plugin-settings" | Out-Null
    '{"collabhost_pride_theme": true}' | Set-Content "$s\WebOx\plugins.json"
    "{`"values`": {`"flag`": `"$flag`"}}" | Set-Content "$s\WebOx\plugin-settings\collabhost_pride_theme.json"
    $env:APPDATA = $s
    if ($theme -eq 'light') { $env:WEBOX_SHOT_THEME = 'light' } else { Remove-Item env:WEBOX_SHOT_THEME -ErrorAction SilentlyContinue }
    foreach ($k in $envs.Keys) { Set-Item "env:$k" $envs[$k] }
    & $exe --sample --screenshot "shots\$name.png" 2>&1 | Out-Null
    foreach ($k in $envs.Keys) { Remove-Item "env:$k" }
}
Shot 'pride_rainbow_dark' 'rainbow' 'dark' @{}
Shot 'pride_rainbow_light' 'rainbow' 'light' @{}
Shot 'pride_trans_dark' 'trans' 'dark' @{}
Shot 'pride_bi_light' 'bi' 'light' @{}
Shot 'pride_rainbow_menu_dark' 'rainbow' 'dark' @{ WEBOX_SHOT_MENU = 'status' }
Shot 'pride_progress_settings_dark' 'progress' 'dark' @{ WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_DETAIL = 'collabhost_pride_theme' }
Shot 'pride_rainbow_settings_light' 'rainbow' 'light' @{ WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_DETAIL = 'collabhost_pride_theme' }
Shot 'pride_sparkle_dark' 'rainbow' 'dark' @{ WEBOX_SHOT_PRIDE_SPARKLE = '1' }
Get-ChildItem shots\pride_* | ForEach-Object { "$($_.Name) $($_.Length)" }
