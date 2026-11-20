# Renders offline screenshots of the sample data into shots/ (git-ignored). Run after `cargo build --release`.
Set-Location (Join-Path $PSScriptRoot '..')
$exe = Join-Path ($env:CARGO_TARGET_DIR ?? 'target') 'release\webox.exe'
New-Item -ItemType Directory -Force shots | Out-Null
function Shot($name, $envs) {
  foreach ($k in $envs.Keys) { Set-Item "env:$k" $envs[$k] }
  & $exe --sample --screenshot "shots\$name.png" 2>&1 | Out-Null
  foreach ($k in $envs.Keys) { Remove-Item "env:$k" }
}
Shot 'profile' @{ WEBOX_SHOT_OPEN='s2'; WEBOX_SHOT_TABS='profile' }
Shot 'content' @{ WEBOX_SHOT_TABS='content' }
Shot 'links' @{ WEBOX_SHOT_TABS='links' }
Shot 'meetings' @{ WEBOX_SHOT_TABS='meetings' }
Shot 'apps' @{ WEBOX_SHOT_TABS='apps' }
Shot 'find' @{ WEBOX_SHOT_TABS='find' }
Shot 'pins' @{ WEBOX_SHOT_TABS='pins' }
Shot 'search_empty' @{ WEBOX_SHOT_SEARCH='1' }
Shot 'search_q' @{ WEBOX_SHOT_SEARCH='an' }
Shot 'card' @{ WEBOX_SHOT_CARD='1' }
Shot 'plugins_installed' @{ WEBOX_SHOT_SETTINGS='plugins' }
Shot 'plugins_available' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_PLUGIN_TAB='available' }
Shot 'plugins_warning' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_PLUGIN_WARN='1' }
Shot 'plugin_detail_slash' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_PLUGIN_DETAIL='collabhost_slash_commands' }
Shot 'plugin_detail_gif' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_PLUGIN_DETAIL='builtin.gif-extras' }
Shot 'plugin_search' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_SETTINGS_SEARCH='shrug' }
# Pride Theme on (writes the switch into the shot's own config; run with APPDATA pointing at a scratch folder)
Shot 'pride_settings' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_PLUGIN_DETAIL='collabhost_pride_theme' }
Shot 'plugins_repos' @{ WEBOX_SHOT_SETTINGS='plugins'; WEBOX_SHOT_PLUGIN_TAB='repos' }
# Pride Theme decorations: scripts/pride-shots.ps1 renders rainbow / trans / bi on dark and light
