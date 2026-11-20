<#
.SYNOPSIS
  Fails if files, commit messages or commit identities hold private data (real people, companies, ids,
  hosts, paths) or tool attribution. Prints category + location only, never the matched value.

.DESCRIPTION
  Two kinds of rules:
  1. Private deny-lists that live OUTSIDE the repository (so the list itself never leaks): lines
     `category<TAB>regex`, case-insensitive; a category ending in `!` is case-sensitive; `#` comments.
     Every literal a/o/u umlaut and sharp s in a rule also matches its ASCII spellings (ae/a, oe/o, ue/u, ss).
     Lists come from -DenyList, else from `git config --get-all webox.denylist` (a file, or a folder whose
     denylist*.txt files are all read), else from $env:WEBOX_DENYLIST (paths separated by ';').
  2. Generic rules that need no list (see docs/privacy-guard.md): e-mail addresses (only reserved example
     domains, reserved TLDs and noreply addresses pass), phone numbers, UUIDs (only obviously synthetic ones
     pass), base64 Webex ids, tenant host names, user-profile paths, default machine names, tool attribution,
     and screenshots: every image outside assets/icons, assets/icon, assets/emoji needs a reviewed entry.

  Binary files are scanned too: printable ASCII / UTF-16 runs, PNG text chunks (tEXt/zTXt/iTXt), JPEG
  EXIF/XMP (as strings) and PDF streams (FlateDecode is inflated). File names are checked as well.

  False positives of generic rules go into the in-repo allowlist scripts/privacy-allowlist.txt; a deny-list
  hit can only be allowed there by the SHA-256 of the matched text, so no private value is ever written down.

.PARAMETER Mode
  tree   : tracked + untracked-not-ignored files of the working tree (default)
  staged : the files staged for the next commit, as staged (pre-commit hook)
  rev    : every file of the commit -Rev (e.g. a branch tip)

.EXAMPLE
  pwsh scripts/check-company-data.ps1                                  # working tree, lists from git config
  pwsh scripts/check-company-data.ps1 -Mode rev -Rev main -Commits     # a branch tip and all its commit messages
  pwsh scripts/check-company-data.ps1 -Mode staged                     # what the pre-commit hook runs
  pwsh scripts/check-company-data.ps1 -MessageFile .git/COMMIT_EDITMSG # what the commit-msg hook runs
#>
param(
    [string[]]$DenyList,
    [string]$Root = (Join-Path $PSScriptRoot '..'),
    [ValidateSet('tree', 'staged', 'rev', 'none')][string]$Mode = 'tree',
    [string]$Rev = 'HEAD',
    [switch]$Commits,            # also commit messages + author/committer of -Range
    [string]$Range,              # rev-list arguments for -Commits (default: -Rev)
    [string]$MessageFile,        # check this commit message and the identity git would commit with
    [string]$Allowlist,          # default: <root>/scripts/privacy-allowlist.txt
    [int]$MaxShow = 400,         # locations printed per category
    [switch]$Context,            # print each finding's line with every match masked as «category»
    [switch]$Quiet,
    [switch]$SelfTest            # prove in a temp repo that hits, regex features and broken rules fail the run
)
$ErrorActionPreference = 'Stop'
$startDir = (Get-Location).Path   # relative -DenyList / -Allowlist / -MessageFile paths are relative to where the guard was started
# fail closed on any error; the exception text is not printed (a regex error would quote the private pattern)
trap { Write-Host "privacy guard error ($($_.Exception.GetType().Name), line $($_.InvocationInfo.ScriptLineNumber)) - nothing was checked (exit 2)."; exit 2 }

if ($SelfTest) {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('privacy-guard-selftest-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory $tmp | Out-Null
    $fails = 0
    try {
        git -C $tmp init -q
        git -C $tmp config user.name 'Self Test'; git -C $tmp config user.email 'selftest@example.com'
        [IO.File]::WriteAllText("$tmp/a.txt", "hello Zorblax Quentaro, see pullrequest/42`n")
        git -C $tmp -c core.autocrlf=false add a.txt
        $d = "$tmp/../$(Split-Path $tmp -Leaf)-deny.txt"
        $msg = "$tmp/../$(Split-Path $tmp -Leaf)-msg.txt"
        function T([string]$name, [string]$rules, [int]$expect, [string[]]$extra) {
            [IO.File]::WriteAllText($d, "$rules`n")
            & pwsh -NoProfile -NonInteractive -File $PSCommandPath -Root $tmp -DenyList $d -Quiet @extra *> $null
            $ok = $LASTEXITCODE -eq $expect
            if (-not $ok) { $script:fails++ }
            Write-Host ("  {0,-4} {1} (exit {2}, expected {3})" -f $(if ($ok) { 'ok' } else { 'FAIL' }), $name, $LASTEXITCODE, $expect)
        }
        T 'no hit is clean' "person`tnothing-matches-this" 0
        T 'literal deny-list hit' "person`tZorblax Quentaro" 1
        T 'lookahead rule' "person`tZorblax(?= Quentaro)" 1
        T '\d rule' "project`tpullrequest/\d+" 1
        T 'umlaut rule matches ASCII spelling' "person`tZörblax" 1
        T 'case-sensitive rule (!) misses other case' "person!`tzorblax" 0
        T 'staged mode sees the staged file' "person`tQuentaro" 1 @('-Mode', 'staged')
        T 'invalid rule fails the run' "person`tZorblax(" 2
        [IO.File]::WriteAllText($msg, "fix: thing`n")
        T 'clean commit message' "person`tnothing-matches-this" 0 @('-MessageFile', $msg)
        [IO.File]::WriteAllText($msg, "fix: ask jane.roe@$('corp-mail' + '.ch')`n")
        T 'e-mail in commit message' "person`tnothing-matches-this" 1 @('-MessageFile', $msg)
        [IO.File]::WriteAllText($msg, "fix: thing`n`n$('Co-Authored' + '-By'): x <x@example.com>`n")
        T 'attribution trailer' "person`tnothing-matches-this" 1 @('-MessageFile', $msg)
    } finally {
        Remove-Item -Recurse -Force $tmp, $d, $msg -EA SilentlyContinue
    }
    if ($fails) { Write-Host "self-test FAILED ($fails)"; exit 1 }
    Write-Host 'self-test passed'; exit 0
}
Set-Location $Root
$Root = (git rev-parse --show-toplevel).Trim()
Set-Location $Root
if ($MessageFile) { if (-not [IO.Path]::IsPathRooted($MessageFile)) { $MessageFile = Join-Path $startDir $MessageFile }; $MessageFile = (Resolve-Path $MessageFile).Path; if (-not $PSBoundParameters.ContainsKey('Mode')) { $Mode = 'none' } }

# ---------------------------------------------------------------------------------------------- rules
$OptIC = [Text.RegularExpressions.RegexOptions]'IgnoreCase, CultureInvariant, Compiled'
$OptCS = [Text.RegularExpressions.RegexOptions]'CultureInvariant, Compiled'

function Expand-Umlauts([string]$rx) {
    $rx = $rx -creplace 'ä|Ä', '(?:ä|ae|a)' -creplace 'ö|Ö', '(?:ö|oe|o)' -creplace 'ü|Ü', '(?:ü|ue|u)' -creplace 'ß', '(?:ß|ss)'
    $rx -creplace 'é|è|É', '(?:é|è|e)'
}

$denyFiles = [System.Collections.Generic.List[string]]::new()
$src = if ($DenyList) { $DenyList } else { @(git config --get-all webox.denylist 2>$null) + @($env:WEBOX_DENYLIST) }
foreach ($d in ($src | ForEach-Object { $_ -split ';' })) {   # several lists: repeat -DenyList or separate with ';'
    if (-not $d) { continue }
    if (-not [IO.Path]::IsPathRooted($d)) { $d = Join-Path $startDir $d }
    if (Test-Path $d -PathType Container) { Get-ChildItem $d -File | Where-Object { $_.Name -like 'denylist*.txt' -or $_.Name -like 'anonymise-map*.tsv' } | ForEach-Object { $denyFiles.Add($_.FullName) } }
    elseif (Test-Path $d -PathType Leaf) { $denyFiles.Add((Resolve-Path $d).Path) }
    else { Write-Host "deny-list not found: (configured path #$($denyFiles.Count + 1))"; exit 2 }
}
# deny rules, grouped per category + case into one alternation each (fast); rules with back-references stay alone
$groups = @{}
$nDeny = 0
$bad = 0
foreach ($f in $denyFiles) {
    $ln = 0
    foreach ($line in [IO.File]::ReadAllLines($f, [Text.Encoding]::UTF8)) {
        $ln++
        if (-not $line.Trim() -or $line.TrimStart().StartsWith('#')) { continue }
        if ($f -like '*.tsv') {
            # an anonymisation map (from<TAB>to): the "from" column is a literal, case-sensitive term
            $from = ($line -split "`t", 2)[0]
            if ($from.Trim().Length -lt 4) { continue }
            $cat, $rx = 'anonymise-map!', [regex]::Escape($from)
        } else {
            $cat, $rx = $line -split "`t", 2
            if (-not $rx) { continue }
        }
        $cs = $cat.EndsWith('!'); $cat = $cat.TrimEnd('!')
        $rx = Expand-Umlauts $rx
        # fail closed: a rule the engine cannot compile is an error, never a silent "clean"
        try { [void][regex]::new($rx) } catch { Write-Host "invalid deny-list rule: $(Split-Path $f -Leaf):$ln (category $cat)"; $bad++; continue }
        $key = if ($rx -match '\\\d|\\k<') { "$cat|$cs|$nDeny" } else { "$cat|$cs" }
        if (-not $groups[$key]) { $groups[$key] = [System.Collections.Generic.List[string]]::new() }
        $groups[$key].Add($rx); $nDeny++
    }
}
if ($bad) { Write-Host "privacy guard: $bad invalid deny-list rule(s) - nothing was checked (exit 2)."; exit 2 }
$denyRules = foreach ($k in $groups.Keys) {
    $cat, $cs = $k -split '\|'
    $alt = ($groups[$k] | ForEach-Object { "(?:$_)" }) -join '|'
    [pscustomobject]@{ Category = $cat; Rx = [regex]::new($alt, $(if ($cs -eq 'True') { $OptCS } else { $OptIC })); Deny = $true }
}
if (-not $denyFiles.Count -and -not $Quiet) { Write-Host 'warning: no private deny-list configured (git config webox.denylist); generic rules only' }

# generic rules (Check = scriptblock deciding whether a match is allowed; $true = allowed)
$ai = 'Cl' + 'aude'; $an = 'Anth' + 'ropic'; $co = 'Co-Authored' + '-By'   # assembled so this file does not match itself
$reservedTld = '(?:example|invalid|test|localhost|local|internal|lan|home\.arpa)'
$fileExt = '(?:png|jpe?g|gif|svg|webp|ico|bmp|rs|toml|json|md|txt|js|ts|py|sh|ps1|lock|yml|yaml|html?|css|wav|mp3|ogg|flac|ttf|otf|qm|exe|dll|so|dylib|zip|gz|tar|nix|desktop)'
$genericHost = '^(?:[a-z]|www|web|signup|api|help|status|binaries|settings|idbroker(?:-[a-z]+)?|identity(?:-[a-z]+)?|u2c|signin|login|developer|developers|meetings?|teams|go|admin|cdn|download|downloads|community|blog|apps|appstore|hub|people|user|whiteboard|bitmoji|analyzer|tsa|nebulabg|reachable|insights-a|cifls-[a-z]+|ai-assistant-[a-z]+|wxcc-[a-z0-9-]+|wap[a-z0-9-]*|mhs-[a-z0-9-]+|mrs-[a-z0-9-]+|aibridge-[a-z0-9-]+|partnerhub[a-z0-9-]*|userhub[a-z0-9-]*|reportservice-[a-z]+|xm|ds|da|ciscospark|webexapis|example|yoursite|yourcompany|your-site|company|acme|contoso|site|demo|sample|test|mycompany|placeholder)$'
$genericZone = '\.(?:wbx2\.com|prod\.infra\.webex\.com|meetapi\.webex\.com|svc\.webex\.com|intelligence\.webex\.com|bcld\.webex\.com|wxdap-[a-z0-9]+\.webex\.com|gov\.ciscospark\.com|prod-[a-z0-9-]+\.ciscospark\.com|ucmgmt\.cisco\.com)$'
$okPathUser = '^(?:Public|Default|All Users|Default User|you|your?[ _-]?name|alice|bob|jdoe|deinname|yourname|test|[a-z](?: [a-z])?|your[-_ ]?user(?:name)?|user|username|name|me|runner|runneradmin|someone|example|<[^>]*>|%[A-Z_]+%|\$env:[A-Z_]+|\$\{?[A-Z_]+\}?|\.\.\.|…|\*|ME)$'
$okPersonalRoom = '^(?:[a-z]|pmrlink|example|someone|user|you|name|jdoe|john\.?doe|alice|bob|max\.?muster(?:mann)?|demo|test|<[^>]*>)$'

function Test-SyntheticUuid([string]$u) {
    $u = $u.ToLowerInvariant()
    $groupsU = $u -split '-'
    if (@($groupsU | Where-Object { $_ -notmatch '^(.)\1*$' }).Count -eq 0) { return $true }   # 11111111-2222-3333-4444-555555555555, all zeros
    $hex = $u -replace '-', ''
    if (($hex -replace '0', '').Length -le 4) { return $true }                                   # 00000000-0000-0000-0000-000000000042
    if ('0123456789abcdef0123456789abcdef0123456789abcdef'.Contains($hex)) { return $true }       # 01234567-89ab-cdef-0123-456789abcdef
    if ($u -match '^12345678-1234-[0-9a-f]234-[0-9a-f]234-123456789abc$') { return $true }        # 12345678-1234-1234-1234-123456789abc
    return $false
}

$generic = @(
    [pscustomobject]@{ Category = 'email'; Rx = [regex]::new("(?<![\w.%+-])[a-z0-9._%+-]+@[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)*\.[a-z]{2,}\b", $OptIC); Check = {
        param($v)
        $local, $dom = $v.ToLowerInvariant() -split '@', 2
        if ($dom -match "(?:^|\.)example\.(?:com|org|net)$" -or $dom -match "(?:^|\.)$reservedTld$") { return $true }
        if ($dom -match "\.$fileExt$" -and $dom -notmatch '\.(?:com|org|net|ch|de|io)$') { return $true }  # icon@2x.png, crate@1.2.rs
        if ($dom -eq 'users.noreply.github.com' -or $local -match '^(?:no-?reply|do-?not-?reply)(?:[+.-].*)?$') { return $true }
        return $false } },
    [pscustomobject]@{ Category = 'phone'; Rx = [regex]::new('(?<![\w+#./=-])(?:\+|00)[1-9]\d{0,2}[ .\-/]?(?:\(0\)[ .\-]?)?\(?\d{1,4}\)?(?:[ .\-/]?\d{2,4}){2,5}(?![\w.]|-\d)|(?<![\w.+-])0\d{2}[ /.\-]\d{3}[ .\-]\d{2}[ .\-]\d{2}(?![\w.])|tel:\+?[\d\-. ()]{6,}', $OptIC); Check = {
        param($v)
        $digits = $v -replace '\D', ''; $d = $digits.Length
        if ($v -match '^00' -and $v -notmatch '[ .\-/]') { return $true }   # 0012345678: a number, not a phone
        # numbers reserved for fiction: NANP 555-0100..0199 (with any area code), UK Ofcom 07700 900000..900999
        if ($digits -match '0{5,}$' -or $digits -match '1234567|7654321') { return $true }   # +41 44 000 00 00, +41 79 123 45 67
        if ($digits -match '^(?:1?\d{3})?55501\d\d$' -or $digits -match '^(?:0044|44|0)7700900\d{3}$') { return $true }
        return ($d -lt 9 -or $d -gt 15) } },
    [pscustomobject]@{ Category = 'uuid'; Rx = [regex]::new('(?<![0-9a-f-])[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(?![0-9a-f-])', $OptIC); Check = { param($v) Test-SyntheticUuid $v } },
    [pscustomobject]@{ Category = 'webex-id'; Rx = [regex]::new('Y2lzY29zcGFyazovL[A-Za-z0-9+/_=-]{8,}', $OptCS); Check = {
        param($v)
        $b = $v.Replace('-', '+').Replace('_', '/'); $b = $b.PadRight($b.Length + (4 - $b.Length % 4) % 4, '=')
        try { $t = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($b)) } catch { return $false }
        $m = [regex]::Match($t, '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}')
        return ($m.Success -and (Test-SyntheticUuid $m.Value)) } },
    [pscustomobject]@{ Category = 'tenant-host'; Rx = [regex]::new('(?<![\w.-])(?:[a-z0-9-]+\.)+(?:webex\.com|ciscospark\.com|webexapis\.com|wbx2\.com|webex\.[a-z]{2}|webex\.co\.[a-z]{2})(?![\w-])', $OptIC); Check = {
        param($v)
        $h = $v.ToLowerInvariant()
        if ($h -match $genericZone) { return $true }
        return (($h -split '\.')[0] -match $genericHost) } },
    [pscustomobject]@{ Category = 'tenant-host'; Rx = [regex]::new('webex\.com/(?:meet|join)/([A-Za-z0-9._<>-]+)', $OptIC); Check = { param($v) ($v -split '/')[-1] -match $okPersonalRoom } },
    [pscustomobject]@{ Category = 'user-path'; Rx = [regex]::new('(?:\b[a-z]:|%SystemDrive%|\$env:SystemDrive)[\\/]+Users[\\/]+([^\\/"''<>|:*?\r\n\t `),;]+)|(?<![\w.~])/(?:home|Users)/([^/"''<>|:*?\r\n\t `),;]+)', $OptIC); Check = {
        param($v)
        $n = [uri]::UnescapeDataString(($v -split '[\\/]+')[2])
        return ($n -match $okPathUser) } },
    [pscustomobject]@{ Category = 'machine'; Rx = [regex]::new('\b(?:DESKTOP|LAPTOP|WIN|PC)-[A-Z0-9]{7,8}\b', $OptCS); Check = { param($v) $v -match '^(?:DESKTOP|LAPTOP|WIN|PC)-(?:X+|0+|EXAMPLE|ABC1234|ABCDEFG)$' } },
    [pscustomobject]@{ Category = 'attribution'; Rx = [regex]::new("$co|Generated (?:with|by) .{0,30}$ai|$ai Code|noreply@$an|@$an\.com|$ai\.ai/|\bAI[- ]generated\b|\bgenerated by (?:an )?AI\b", $OptIC); Check = { param($v) $false } },
    [pscustomobject]@{ Category = 'attribution'; Rx = [regex]::new("\b$ai\b|\b$an\b", $OptCS); Check = { param($v) $false } }
)

# ---------------------------------------------------------------------------------------------- allowlist
$allowRel = 'scripts/privacy-allowlist.txt'
$allowLines = @()
if ($Allowlist) { if (-not [IO.Path]::IsPathRooted($Allowlist)) { $Allowlist = Join-Path $startDir $Allowlist }; $allowLines = [IO.File]::ReadAllLines((Resolve-Path $Allowlist).Path, [Text.Encoding]::UTF8) }
else {
    # the allowlist of what is being checked: the commit's (rev), the staged one (staged), else the working tree's
    $spec = switch ($Mode) { 'rev' { "${Rev}:$allowRel" } 'staged' { ":$allowRel" } default { $null } }
    $txt = if ($spec) { git show $spec 2>$null } else { $null }
    if ($LASTEXITCODE -or -not $spec) { $txt = $null }
    if ($null -ne $txt) { $allowLines = @($txt) }
    elseif ($Mode -ne 'rev' -and (Test-Path (Join-Path $Root $allowRel))) { $allowLines = [IO.File]::ReadAllLines((Join-Path $Root $allowRel), [Text.Encoding]::UTF8) }
}
$allow = [System.Collections.Generic.List[object]]::new()
$allowImages = @{}
if ($allowLines) {
    foreach ($line in $allowLines) {
        $l = ($line -replace '\s+#.*$', '').TrimEnd()
        if (-not $l.Trim() -or $l.TrimStart().StartsWith('#')) { continue }
        $p = $l -split "`t"
        if ($p.Count -lt 2) { continue }
        if ($p[0] -eq 'image') { $allowImages[$p[1]] = $(if ($p.Count -ge 3) { $p[2].ToLowerInvariant() } else { '' }); continue }
        $glob = if ($p.Count -ge 3 -and $p[2]) { $p[2] } else { '*' }
        $allow.Add([pscustomobject]@{ Category = $p[0]; Value = $p[1]; Glob = $glob })
    }
}
$sha = [Security.Cryptography.SHA256]::Create()
function Get-Sha([string]$v) { -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($v.ToLowerInvariant())) | ForEach-Object { $_.ToString('x2') }) }
function Test-Allowed([string]$cat, [string]$value, [string]$path, [bool]$deny) {
    if (-not $allow.Count) { return $false }
    $h = $null
    foreach ($a in $allow) {
        if ($a.Category -ne $cat -and $a.Category -ne '*') { continue }
        if ($path -and -not ($path -like $a.Glob)) { continue }
        if ($a.Value -eq '*') { if ($path -and $a.Glob -ne '*') { return $true }; continue }   # whole category in these files only (vendored data)
        if ($a.Value.StartsWith('sha256:')) { if (-not $h) { $h = Get-Sha $value }; if ($a.Value.Substring(7) -eq $h) { return $true } }
        elseif (-not $deny -and $a.Value -ieq $value) { return $true }
    }
    return $false
}

# ---------------------------------------------------------------------------------------------- scanning
$findings = [System.Collections.Generic.List[object]]::new()
function Add-Finding($cat, $where) { $findings.Add([pscustomobject]@{ Category = $cat; Where = $where; Line = $script:ctxLine }) }

function Get-LineNo([string]$t, [int]$idx) { $n = 1; $i = -1; while (($i = $t.IndexOf("`n", $i + 1)) -ge 0 -and $i -lt $idx) { $n++ }; $n }

# -Context: the finding's line with EVERY match of every rule replaced by «category» (values never shown)
function Get-MaskedLine([string]$t, [int]$idx) {
    if (-not $Context) { return $null }
    $s = $t.LastIndexOf("`n", [Math]::Max(0, $idx - 1)) + 1; if ($idx -eq 0) { $s = 0 }
    $e = $t.IndexOf("`n", $idx); if ($e -lt 0) { $e = $t.Length }
    $line = $t.Substring($s, $e - $s).Trim()
    if ($line.Length -gt 400) { $line = $line.Substring([Math]::Max(0, $idx - $s - 150), [Math]::Min(300, $line.Length - [Math]::Max(0, $idx - $s - 150))) }
    foreach ($r in @($denyRules) + @($generic)) { $c = $r.Category; $line = $r.Rx.Replace($line, { param($m) if ($r.Check -and (& $r.Check $m.Value)) { $m.Value } else { "«$c»" } }) }
    $line
}

# Scan one text; $label = 'path' or 'commit abc123'; $lineBase = $true to report :line
function Scan-Text([string]$t, [string]$label, [string]$path, [bool]$withLine, [bool]$genericOn) {
    if (-not $t) { return }
    foreach ($r in $denyRules) {
        foreach ($m in $r.Rx.Matches($t)) {
            if (Test-Allowed $r.Category $m.Value $path $true) { continue }
            $script:ctxLine = Get-MaskedLine $t $m.Index
            Add-Finding $r.Category $(if ($withLine) { "${label}:$(Get-LineNo $t $m.Index)" } else { $label })
        }
    }
    if (-not $genericOn) { return }
    foreach ($r in $generic) {
        foreach ($m in $r.Rx.Matches($t)) {
            if (& $r.Check $m.Value) { continue }
            if (Test-Allowed $r.Category $m.Value $path $false) { continue }
            $script:ctxLine = Get-MaskedLine $t $m.Index
            Add-Finding $r.Category $(if ($withLine) { "${label}:$(Get-LineNo $t $m.Index)" } else { $label })
        }
    }
}

function Get-Strings([byte[]]$b) {
    # printable ASCII/Latin-1 runs (>= 4) and UTF-16LE runs (>= 4)
    $sb = [Text.StringBuilder]::new()
    $a = [Text.Encoding]::Latin1.GetString($b)
    foreach ($m in [regex]::Matches($a, '[\x20-\x7e\xa0-\xff\t]{4,}')) { [void]$sb.AppendLine($m.Value) }
    $u = [Text.Encoding]::Unicode.GetString($b, 0, $b.Length - ($b.Length % 2))
    foreach ($m in [regex]::Matches($u, '[\x20-\x7e -ɏ]{4,}')) { [void]$sb.AppendLine($m.Value) }
    if ($b.Length -gt 1) { $u2 = [Text.Encoding]::Unicode.GetString($b, 1, $b.Length - 1 - (($b.Length - 1) % 2)); foreach ($m in [regex]::Matches($u2, '[\x20-\x7e -ɏ]{4,}')) { [void]$sb.AppendLine($m.Value) } }
    $sb.ToString()
}
function Inflate([byte[]]$b, [int]$off, [int]$len) {
    try {
        $ms = [IO.MemoryStream]::new($b, $off, $len)
        $z = [IO.Compression.ZLibStream]::new($ms, [IO.Compression.CompressionMode]::Decompress)
        $o = [IO.MemoryStream]::new(); $z.CopyTo($o); $z.Dispose()
        return [Text.Encoding]::Latin1.GetString($o.ToArray())
    } catch { return '' }
}
function Get-EmbeddedText([byte[]]$b, [string]$path) {
    $sb = [Text.StringBuilder]::new()
    [void]$sb.AppendLine((Get-Strings $b))
    if ($b.Length -gt 8 -and $b[0] -eq 0x89 -and $b[1] -eq 0x50 -and $b[2] -eq 0x4E -and $b[3] -eq 0x47) {
        $i = 8
        while ($i + 8 -le $b.Length) {
            $len = ([int]$b[$i] -shl 24) -bor ([int]$b[$i + 1] -shl 16) -bor ([int]$b[$i + 2] -shl 8) -bor [int]$b[$i + 3]
            $type = [Text.Encoding]::ASCII.GetString($b, $i + 4, 4)
            if ($len -lt 0 -or $i + 12 + $len -gt $b.Length) { break }
            if ($type -in 'zTXt', 'iTXt') {
                $d = $i + 8; $end = $d + $len
                $k = [Array]::IndexOf($b, [byte]0, $d, $len)
                if ($k -ge 0) {
                    if ($type -eq 'zTXt') { [void]$sb.AppendLine((Inflate $b ($k + 2) ($end - $k - 2))) }
                    elseif ($b[$k + 1] -eq 1) {
                        # iTXt compressed: keyword\0 flag method lang\0 transkey\0 text
                        $p = $k + 3; $p = [Array]::IndexOf($b, [byte]0, $p, $end - $p) + 1; $p = [Array]::IndexOf($b, [byte]0, $p, $end - $p) + 1
                        if ($p -gt 0) { [void]$sb.AppendLine((Inflate $b $p ($end - $p))) }
                    }
                }
            }
            $i += 12 + $len
        }
    }
    if ($b.Length -gt 5 -and [Text.Encoding]::ASCII.GetString($b, 0, 5) -eq '%PDF-') {
        $a = [Text.Encoding]::Latin1.GetString($b)
        foreach ($m in [regex]::Matches($a, 'stream\r?\n')) {
            $s = $m.Index + $m.Length; $e = $a.IndexOf('endstream', $s)
            if ($e -gt $s) { [void]$sb.AppendLine((Inflate $b $s ($e - $s))) }
        }
    }
    $sb.ToString()
}

$imageExt = '\.(?:png|jpe?g|gif|webp|bmp|tiff?|heic|avif)$'
$imageOkDir = '^(?:assets/icons/|assets/icon/|assets/emoji/)'

function Scan-File([string]$path, [byte[]]$bytes, [string]$blob) {
    $genericOn = ($path -ne $allowRel)
    # the file name
    Scan-Text $path "${path}:name" $path $false $genericOn
    $isBin = $false
    $n = [Math]::Min($bytes.Length, 8000)
    for ($i = 0; $i -lt $n; $i++) { if ($bytes[$i] -eq 0) { $isBin = $true; break } }
    if ($path -match $imageExt -or $path -match '\.pdf$') { $isBin = $true }
    if ($isBin) {
        Scan-Text (Get-EmbeddedText $bytes $path) "${path}:binary" $path $false $genericOn
        if ($path -match $imageExt -and $path -notmatch $imageOkDir) {
            $ok = $allowImages.ContainsKey($path) -and ($allowImages[$path] -eq '' -or $allowImages[$path] -eq $blob)
            if (-not $ok) { $script:ctxLine = $null; Add-Finding 'image-review' "${path}:image" }
        }
    } else {
        $t = if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) { [Text.Encoding]::Unicode.GetString($bytes) } else { [Text.Encoding]::UTF8.GetString($bytes) }
        Scan-Text $t $path $path $true $genericOn
    }
}

# git cat-file --batch reader
$script:cat = $null
function Get-Blob([string]$id) {
    if (-not $script:cat) {
        $psi = [Diagnostics.ProcessStartInfo]::new('git', 'cat-file --batch')
        $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.UseShellExecute = $false; $psi.WorkingDirectory = $Root
        $script:cat = [Diagnostics.Process]::Start($psi)
    }
    $script:cat.StandardInput.Write("$id`n"); $script:cat.StandardInput.Flush()
    $s = $script:cat.StandardOutput.BaseStream
    $hdr = [Text.StringBuilder]::new()
    while (($c = $s.ReadByte()) -ne 10) { if ($c -lt 0) { throw 'git cat-file ended' }; [void]$hdr.Append([char]$c) }
    $parts = $hdr.ToString() -split ' '
    if ($parts[1] -eq 'missing') { return , [byte[]]@() }
    $size = [int64]$parts[2]
    $buf = [byte[]]::new($size); $got = 0
    while ($got -lt $size) { $r = $s.Read($buf, $got, $size - $got); if ($r -le 0) { throw 'short read' }; $got += $r }
    [void]$s.ReadByte()
    , $buf
}

$nFiles = 0
switch ($Mode) {
    'tree' {
        $paths = (git -c core.quotepath=off ls-files -co --exclude-standard -z) -split "`0" | Where-Object { $_ }
        $hashes = @{}
        $imgs = @($paths | Where-Object { $_ -match $imageExt -and (Test-Path -LiteralPath $_ -PathType Leaf) })
        if ($imgs.Count) { $h = @($imgs | git hash-object --stdin-paths); for ($i = 0; $i -lt $imgs.Count; $i++) { $hashes[$imgs[$i]] = $h[$i] } }
        foreach ($p in $paths) {
            $full = Join-Path $Root $p
            if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { continue }
            Scan-File $p ([IO.File]::ReadAllBytes($full)) $hashes[$p]; $nFiles++
        }
    }
    'staged' {
        $names = (git -c core.quotepath=off diff --cached --name-only --diff-filter=ACMR -z) -split "`0" | Where-Object { $_ }
        foreach ($p in $names) {
            $e = git ls-files -s -- $p | Select-Object -First 1
            if (-not $e) { continue }
            $id = ($e -split '\s+')[1]
            Scan-File $p (Get-Blob $id) $id; $nFiles++
        }
    }
    'rev' {
        foreach ($e in ((git -c core.quotepath=off ls-tree -r -z $Rev) -split "`0" | Where-Object { $_ })) {
            $meta, $p = $e -split "`t", 2
            $m = $meta -split ' '
            if ($m[1] -ne 'blob') { continue }
            Scan-File $p (Get-Blob $m[2]) $m[2]; $nFiles++
        }
    }
}

# identities and messages
function Test-Identity([string]$name, [string]$email, [string]$label) {
    Scan-Text "$name <$email>" $label '' $false $false            # deny-list rules on both
    $em = $generic[0]
    if (-not (& $em.Check $email) -and -not (Test-Allowed 'email' $email '' $false)) { Add-Finding 'identity' $label }
}
$nCommits = 0
if ($Commits) {
    $args2 = @(if ($Range) { $Range -split ' ' } else { $Rev })
    $raw = git log --format='%H%x00%an%x00%ae%x00%cn%x00%ce%x00%B%x1e' @args2
    foreach ($c in (($raw -join "`n") -split "\x1e")) {
        $c = $c.TrimStart("`n")
        if (-not $c) { continue }
        $f = $c -split "`0", 6
        $id = "commit $($f[0].Substring(0, 10))"
        Test-Identity $f[1] $f[2] "$id author"
        Test-Identity $f[3] $f[4] "$id committer"
        Scan-Text $f[5] "$id message" '' $false $true
        $nCommits++
    }
}
if ($MessageFile) {
    $msg = ([IO.File]::ReadAllLines($MessageFile) | Where-Object { -not $_.StartsWith('#') }) -join "`n"
    if ($msg -match '(?m)^-{24} >8 -{24}') { $msg = $msg.Substring(0, $msg.IndexOf('------------------------ >8')) }
    Scan-Text $msg 'commit message' '' $false $true
    foreach ($v in 'GIT_AUTHOR_IDENT', 'GIT_COMMITTER_IDENT') {
        $ident = git var $v
        if ($ident -match '^(.*) <([^>]*)>') { Test-Identity $Matches[1] $Matches[2] "$($v.ToLower() -replace '^git_|_ident$','') identity" }
    }
}
if ($script:cat) { $script:cat.StandardInput.Close(); $script:cat.WaitForExit(2000) | Out-Null }

# ---------------------------------------------------------------------------------------------- report
$what = @(); if ($Mode -ne 'none') { $what += "$nFiles files ($Mode$(if ($Mode -eq 'rev') { " $Rev" }))" }; if ($Commits) { $what += "$nCommits commits" }; if ($MessageFile) { $what += 'commit message' }
$rulesN = "$nDeny private + $($generic.Count) generic rules"
if ($findings.Count -eq 0) { if (-not $Quiet) { Write-Host "clean: $rulesN, $($what -join ', ')" }; exit 0 }
Write-Host "PRIVATE DATA OR ATTRIBUTION FOUND ($rulesN, $($what -join ', ')). Values are never printed."
foreach ($g in ($findings | Group-Object Category | Sort-Object Name)) {
    Write-Host ("  {0,-12} {1}" -f $g.Name, $g.Count)
    $locs = $g.Group | Group-Object Where | Sort-Object Name
    foreach ($l in ($locs | Select-Object -First $MaxShow)) {
        Write-Host ("      {0}{1}" -f $l.Name, $(if ($l.Count -gt 1) { "  (x$($l.Count))" }))
        if ($Context -and $l.Group[0].Line) { Write-Host "          | $($l.Group[0].Line)" }
    }
    if ($locs.Count -gt $MaxShow) { Write-Host "      ... $($locs.Count - $MaxShow) more locations" }
}
Write-Host 'Replace real values with made-up ones (example.com addresses, synthetic ids, invented names).'
Write-Host 'Only a false positive of a rule may be allowed: see docs/privacy-guard.md (scripts/privacy-allowlist.txt).'
exit 1
