# Pixel-parity harness (docs/theming-engine.md, phase 0). Renders a fixed list of scenes with `--sample --screenshot` and compares
# them pixel by pixel against a baseline, so a refactor can prove "no visual change".
#
#   Render:   pwsh scripts/parity.ps1 -Exe <webox.exe> -Out <dir> [-Only <regex>] [-Jobs 4] [-All]
#   Compare:  pwsh scripts/parity.ps1 -Baseline <dir> -Current <dir> [-Diff <dir>]
#   Both:     pwsh scripts/parity.ps1 -BaseExe <main webox.exe> -Exe <branch webox.exe> -Out <dir> [-Diff <dir>]
#
# Every scene runs with its own empty config folder (APPDATA / LOCALAPPDATA / XDG_* point into a scratch folder, never the real
# account), English UI text (WEBOX_LANG=en) and no update check. Scenes that need the Pride Theme copy
# collabhost_pride_theme.<dll|so|dylib> from the exe's folder into that scratch config. The exit code is the number of scenes that
# differ (0 = identical). A differing scene gets <name>.diff.png in -Diff: changed pixels red over a dimmed copy of the baseline
# (Windows; elsewhere both pictures are copied there).
param(
    [string]$Exe, [string]$Out, [string]$BaseExe,
    [string]$Baseline, [string]$Current, [string]$Diff,
    [string]$Only = '.', [int]$Jobs = 4, [switch]$List, [switch]$All
)
$ErrorActionPreference = 'Stop'

# name -> environment. Keep names stable: they are the baseline file names.
$Scenes = [ordered]@{
    'start'                 = @{}
    'dark'                  = @{ WEBOX_SHOT_THEME = 'dark' }
    'light'                 = @{ WEBOX_SHOT_THEME = 'light' }
    'jade_light'            = @{ WEBOX_SHOT_THEME = 'MomentumJadeLight' }
    'compact'               = @{ WEBOX_SHOT_DENSITY = 'compact' }
    'compact_view'          = @{ WEBOX_SHOT_COMPACT_VIEW = '1' }
    'compact_light'         = @{ WEBOX_SHOT_DENSITY = 'compact'; WEBOX_SHOT_THEME = 'light' }
    'zoom125'               = @{ WEBOX_SHOT_ZOOM = '1.25' }
    'wide_rail'             = @{ WEBOX_SHOT_WIDE = '1' }
    'narrow'                = @{ WEBOX_SHOT_SIZE = '900x640' }
    'direct'                = @{ WEBOX_SHOT_OPEN = 's2' }
    'draft'                 = @{ WEBOX_SHOT_DRAFT = 's3' }
    'menu_gear'             = @{ WEBOX_SHOT_MENU = 'gear' }
    'menu_status'           = @{ WEBOX_SHOT_MENU = 'status' }
    'menu_filters'          = @{ WEBOX_SHOT_MENU = 'filters' }
    'menu_more'             = @{ WEBOX_SHOT_MENU = 'more' }
    'menu_row'              = @{ WEBOX_SHOT_MENU = 'row' }
    'menu_status_light'     = @{ WEBOX_SHOT_MENU = 'status'; WEBOX_SHOT_THEME = 'light' }
    'call_ringing'          = @{ WEBOX_SHOT_CALL = 'ringing' }
    'call_connected'        = @{ WEBOX_SHOT_CALL = 'connected' }
    'call_incoming'         = @{ WEBOX_SHOT_CALL = 'incoming' }
    'call_menu'             = @{ WEBOX_SHOT_CALL = 'menu' }
    'create_space'          = @{ WEBOX_SHOT_CREATE = '1' }
    'emoji'                 = @{ WEBOX_SHOT_EMOJI = '1' }
    'shortcuts_search'      = @{ WEBOX_SHOT_SHORTCUTS = 'search' }
    'profile'               = @{ WEBOX_SHOT_OPEN = 's2'; WEBOX_SHOT_TABS = 'profile' }
    'people'                = @{ WEBOX_SHOT_TABS = 'people' }
    'content'               = @{ WEBOX_SHOT_TABS = 'content' }
    'links'                 = @{ WEBOX_SHOT_TABS = 'links' }
    'meetings_tab'          = @{ WEBOX_SHOT_TABS = 'meetings' }
    'apps'                  = @{ WEBOX_SHOT_TABS = 'apps' }
    'find'                  = @{ WEBOX_SHOT_TABS = 'find' }
    'pins'                  = @{ WEBOX_SHOT_TABS = 'pins' }
    'search_empty'          = @{ WEBOX_SHOT_SEARCH = '1' }
    'search_q'              = @{ WEBOX_SHOT_SEARCH = 'an' }
    'card'                  = @{ WEBOX_SHOT_CARD = '1' }
    'page_meetings'         = @{ WEBOX_SHOT_TAB = 'meetings' }
    'page_contacts'         = @{ WEBOX_SHOT_TAB = 'contacts' }
    'page_contacts_sel'     = @{ WEBOX_SHOT_TAB = 'contacts'; WEBOX_SHOT_SEL = '1' }
    'page_teams'            = @{ WEBOX_SHOT_TAB = 'teams' }
    'page_teams_sel'        = @{ WEBOX_SHOT_TAB = 'teams'; WEBOX_SHOT_SEL = '0' }
    'page_teams_new'        = @{ WEBOX_SHOT_TAB = 'teams'; WEBOX_SHOT_NEWTEAM = '1' }
    'page_whiteboards'      = @{ WEBOX_SHOT_TAB = 'whiteboards' }
    'set_general'           = @{ WEBOX_SHOT_SETTINGS = 'general' }
    'set_audio'             = @{ WEBOX_SHOT_SETTINGS = 'audio' }
    'set_notifications'     = @{ WEBOX_SHOT_SETTINGS = 'notifications' }
    'set_messaging'         = @{ WEBOX_SHOT_SETTINGS = 'messaging' }
    'set_appearance'        = @{ WEBOX_SHOT_SETTINGS = 'appearance' }
    'set_appearance_light'  = @{ WEBOX_SHOT_SETTINGS = 'appearance'; WEBOX_SHOT_THEME = 'light' }
    'set_accessibility'     = @{ WEBOX_SHOT_SETTINGS = 'accessibility' }
    'set_shortcuts'         = @{ WEBOX_SHOT_SETTINGS = 'shortcuts' }
    'set_calling'           = @{ WEBOX_SHOT_SETTINGS = 'calling' }
    'set_customize'         = @{ WEBOX_SHOT_SETTINGS = 'customize' }
    'set_search'            = @{ WEBOX_SHOT_SETTINGS = 'general'; WEBOX_SHOT_SETTINGS_SEARCH = 'theme' }
    'plugins_installed'     = @{ WEBOX_SHOT_SETTINGS = 'plugins' }
    'plugins_available'     = @{ WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_TAB = 'available' }
    'plugins_warning'       = @{ WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_WARN = '1' }
    'plugins_repos'         = @{ WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_TAB = 'repos' }
    'plugin_detail_gif'     = @{ WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_DETAIL = 'builtin.gif-extras' }
    'spaces_row_menu'       = @{ WEBOX_SHOT_SPACES_ROW = 's3' }
    # Pride Theme (palette, accents and decorations through the plugin ABI).
    'pride_rainbow_dark'    = @{ PRIDE = 'rainbow' }
    'pride_rainbow_light'   = @{ PRIDE = 'rainbow'; WEBOX_SHOT_THEME = 'light' }
    'pride_trans_dark'      = @{ PRIDE = 'trans' }
    'pride_bi_light'        = @{ PRIDE = 'bi'; WEBOX_SHOT_THEME = 'light' }
    'pride_menu_dark'       = @{ PRIDE = 'rainbow'; WEBOX_SHOT_MENU = 'status' }
    'pride_settings_dark'   = @{ PRIDE = 'progress'; WEBOX_SHOT_SETTINGS = 'plugins'; WEBOX_SHOT_PLUGIN_DETAIL = 'collabhost_pride_theme' }
    'pride_wide_dark'       = @{ PRIDE = 'rainbow'; WEBOX_SHOT_WIDE = '1' }
}

# Scenes that follow the wall clock (call history times "now - n minutes", the moving camera test pattern): rendered and
# compared only with -All, since two runs a minute apart differ.
$Clock = @('page_calls', 'page_calls_dial', 'page_voicemail', 'set_video')
$Scenes['page_calls'] = @{ WEBOX_SHOT_TAB = 'calls' }
$Scenes['page_calls_dial'] = @{ WEBOX_SHOT_TAB = 'calls'; WEBOX_SHOT_DIAL = '1' }
$Scenes['page_voicemail'] = @{ WEBOX_SHOT_TAB = 'voicemail' }
$Scenes['set_video'] = @{ WEBOX_SHOT_SETTINGS = 'video' }
function Selected { $Scenes.Keys | Where-Object { $_ -match $Only -and ($All -or $Clock -notcontains $_) } }

if ($List) { Selected; exit 0 }

function Render([string]$exe, [string]$out) {
    $exe = (Resolve-Path $exe).Path
    New-Item -ItemType Directory -Force $out | Out-Null
    $out = (Resolve-Path $out).Path
    $ext = if ($IsWindows) { 'dll' } elseif ($IsMacOS) { 'dylib' } else { 'so' }
    $pride = Join-Path (Split-Path $exe) $(if ($IsWindows) { "collabhost_pride_theme.$ext" } else { "libcollabhost_pride_theme.$ext" })
    $scratch = Join-Path ([IO.Path]::GetTempPath()) "webox-parity-$PID"
    $names = @(Selected)
    $todo = $names | ForEach-Object { [pscustomobject]@{ Name = $_; Env = $Scenes[$_] } }
    $todo | ForEach-Object -ThrottleLimit $Jobs -Parallel {
        $s = $_; $exe = $using:exe; $out = $using:out; $pride = $using:pride
        $cfg = Join-Path $using:scratch $s.Name
        Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue
        $app = Join-Path $cfg 'WebOx'
        New-Item -ItemType Directory -Force (Join-Path $app 'plugins'), (Join-Path $app 'plugin-settings') | Out-Null
        $envs = @{ APPDATA = $cfg; LOCALAPPDATA = $cfg; XDG_CONFIG_HOME = $cfg; XDG_DATA_HOME = $cfg; XDG_CACHE_HOME = $cfg; HOME = $cfg
                   WEBOX_LANG = 'en'; WEBOX_NO_UPDATE_CHECK = '1' }
        foreach ($k in $s.Env.Keys) { if ($k -ne 'PRIDE') { $envs[$k] = $s.Env[$k] } }
        if ($s.Env.PRIDE) {
            if (-not (Test-Path $pride)) { Write-Warning "$($s.Name): $pride missing, skipped"; return }
            # The config folder on Linux / macOS is <XDG_CONFIG_HOME or ~/.config>/webox; both spellings get the plugin.
            foreach ($d in @($app, (Join-Path $cfg 'webox'))) {
                New-Item -ItemType Directory -Force (Join-Path $d 'plugins'), (Join-Path $d 'plugin-settings') | Out-Null
                Copy-Item $pride (Join-Path $d 'plugins')
                '{"collabhost_pride_theme": true}' | Set-Content (Join-Path $d 'plugins.json')
                "{`"values`": {`"flag`": `"$($s.Env.PRIDE)`"}}" | Set-Content (Join-Path $d 'plugin-settings/collabhost_pride_theme.json')
            }
        }
        $psi = [Diagnostics.ProcessStartInfo]::new($exe)
        foreach ($a in '--sample', '--screenshot', (Join-Path $out "$($s.Name).png")) { $psi.ArgumentList.Add($a) }
        foreach ($k in $envs.Keys) { $psi.Environment[$k] = $envs[$k] }
        $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
        $p = [Diagnostics.Process]::Start($psi)
        $p.StandardOutput.ReadToEndAsync() | Out-Null; $p.StandardError.ReadToEndAsync() | Out-Null
        if (-not $p.WaitForExit(180000)) { $p.Kill(); Write-Warning "$($s.Name): timed out" }
        if (-not (Test-Path (Join-Path $out "$($s.Name).png"))) { Write-Warning "$($s.Name): no picture" }
        Remove-Item -Recurse -Force $cfg -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
    Write-Host "rendered $($names.Count) scenes into $out"
}

function Compare-Dirs([string]$base, [string]$cur, [string]$diff) {
    # System.Drawing decodes the pictures on Windows. Elsewhere the PNG bytes are compared: the same encoder writes both sides,
    # so equal pixels give equal files.
    if ($IsWindows) { Add-Type -AssemblyName System.Drawing }
    if ($diff) { New-Item -ItemType Directory -Force $diff | Out-Null }
    $bad = 0
    $names = @(Selected)
    foreach ($n in $names) {
        $a = Join-Path $base "$n.png"; $b = Join-Path $cur "$n.png"
        if (-not (Test-Path $a) -and -not (Test-Path $b)) { continue }
        if (-not (Test-Path $a) -or -not (Test-Path $b)) { Write-Host "MISSING  $n"; $bad++; continue }
        if ((Get-FileHash $a).Hash -eq (Get-FileHash $b).Hash) { continue }
        if (-not $IsWindows) { Write-Host "DIFFERS  $n"; if ($diff) { Copy-Item $a (Join-Path $diff "$n.base.png"); Copy-Item $b (Join-Path $diff "$n.head.png") }; $bad++; continue }
        $r = Compare-Png $a $b $(if ($diff) { Join-Path $diff "$n.diff.png" })
        if ($r -ne 0) { Write-Host ("DIFFERS  {0}: {1} pixels" -f $n, $r); $bad++ }
    }
    if ($bad -eq 0) { Write-Host "parity: $($names.Count) scenes identical" } else { Write-Host "parity: $bad of $($names.Count) scenes differ" }
    return $bad
}

# Number of differing pixels (-1 for a size change); writes the diff picture when asked.
function Compare-Png([string]$a, [string]$b, [string]$diffPath) {
    $ia = [Drawing.Bitmap]::new($a); $ib = [Drawing.Bitmap]::new($b)
    try {
        if ($ia.Width -ne $ib.Width -or $ia.Height -ne $ib.Height) { return -1 }
        $pa = Get-Pixels $ia; $pb = Get-Pixels $ib
        if ([Linq.Enumerable]::SequenceEqual($pa, $pb)) { return 0 }
        $count = 0
        $out = [byte[]]::new($pa.Length)
        for ($i = 0; $i -lt $pa.Length; $i += 4) {
            if ($pa[$i] -ne $pb[$i] -or $pa[$i + 1] -ne $pb[$i + 1] -or $pa[$i + 2] -ne $pb[$i + 2] -or $pa[$i + 3] -ne $pb[$i + 3]) {
                $count++; $out[$i] = 0; $out[$i + 1] = 0; $out[$i + 2] = 255
            } else {
                $out[$i] = [byte]($pa[$i] / 4); $out[$i + 1] = [byte]($pa[$i + 1] / 4); $out[$i + 2] = [byte]($pa[$i + 2] / 4)
            }
            $out[$i + 3] = 255
        }
        if ($diffPath) {
            $d = [Drawing.Bitmap]::new($ia.Width, $ia.Height, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $bd = $d.LockBits([Drawing.Rectangle]::new(0, 0, $d.Width, $d.Height), 'WriteOnly', $d.PixelFormat)
            [Runtime.InteropServices.Marshal]::Copy($out, 0, $bd.Scan0, $out.Length); $d.UnlockBits($bd)
            $d.Save($diffPath); $d.Dispose()
        }
        return $count
    } finally { $ia.Dispose(); $ib.Dispose() }
}

function Get-Pixels([Drawing.Bitmap]$bmp) {
    $bd = $bmp.LockBits([Drawing.Rectangle]::new(0, 0, $bmp.Width, $bmp.Height), 'ReadOnly', [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { $buf = [byte[]]::new($bd.Stride * $bmp.Height); [Runtime.InteropServices.Marshal]::Copy($bd.Scan0, $buf, 0, $buf.Length); return , $buf }
    finally { $bmp.UnlockBits($bd) }
}

if ($Baseline -and $Current) { exit (Compare-Dirs $Baseline $Current $Diff) }
if ($BaseExe -and $Exe -and $Out) {
    Render $BaseExe (Join-Path $Out 'base')
    Render $Exe (Join-Path $Out 'head')
    exit (Compare-Dirs (Join-Path $Out 'base') (Join-Path $Out 'head') $(if ($Diff) { $Diff } else { Join-Path $Out 'diff' }))
}
if ($Exe -and $Out) { Render $Exe $Out; exit 0 }
Write-Host 'usage: parity.ps1 -Exe <exe> -Out <dir> | -Baseline <dir> -Current <dir> [-Diff <dir>] | -BaseExe <exe> -Exe <exe> -Out <dir>'
exit 2
