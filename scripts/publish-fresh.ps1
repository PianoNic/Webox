<#
.SYNOPSIS
  Exports the committed tree into a new folder and makes it a new repository with one commit.

.DESCRIPTION
  The development history holds data that must never be public, so the public repository starts
  from a single commit. This script:
    1. refuses to run with uncommitted changes, so what was checked is what gets exported;
    2. runs scripts/check-company-data.ps1 with your private deny-lists on HEAD's tree;
    3. exports HEAD with `git archive` (tracked files only; tracked files that .gitignore
       would ignore are left out and listed); line endings follow .gitattributes, not core.autocrlf;
    4. runs `git init` in the destination, stages with LF line endings and the executable bits of
       HEAD, checks the staged tree again, and commits it as
       PianoNic <79938743+PianoNic@users.noreply.github.com> with hooks disabled;
    5. checks the new commit's message and identity (the guard's commit rules) and that it is a
       single commit.
  If any check after step 3 fails, the whole destination is deleted again.
  It never adds a remote and never pushes.

.PARAMETER DenyList
  One or more deny-list files or folders (a folder contributes its denylist*.txt and anonymise-map*.tsv;
  an anonymise-map's "from" column counts as literal terms). Repeat or separate with ';'.

.EXAMPLE
  pwsh scripts/publish-fresh.ps1 -Destination ..\Webox-public -DenyList ..\private
  pwsh scripts/publish-fresh.ps1 -Destination ..\Webox-public -DenyList '..\private\denylist.txt;..\private\denylist-generated.txt;..\private\anonymise-map.tsv'
#>
param(
    [Parameter(Mandatory)][string]$Destination,
    [Parameter(Mandatory)][string[]]$DenyList,
    [string]$Message = 'Initial commit',
    # Skips the two full-tree scans (steps 2 and 4). Every commit already passes the guard through the pre-commit hook;
    # the commit message / identity checks still run.
    [switch]$SkipFullScan
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$lists = @($DenyList | ForEach-Object { $_ -split ';' } | Where-Object { $_ } | ForEach-Object { (Resolve-Path $_).Path }) -join ';'
$guard = Join-Path $repo 'scripts/check-company-data.ps1'
function Invoke-Guard([string[]]$a) { & pwsh -NoProfile -NonInteractive -File $guard -DenyList $lists @a | Out-Host; return $LASTEXITCODE }

# Attribution patterns, assembled from pieces so this file does not match itself.
$ai = 'Cl' + 'aude'; $an = 'Anth' + 'ropic'
$attribution = @(('Co-Authored' + '-By'), "Generated with .{0,20}$ai", "noreply@$an", "\b$ai\b", "\b$an\b")
function Test-Attribution([string]$text) {
    foreach ($rx in $attribution) { if ([regex]::IsMatch($text, $rx, 'IgnoreCase')) { return $true } }
    return $false
}
if (Test-Attribution $Message) { throw 'The commit message contains attribution.' }

$dest = $null
Push-Location $repo
try {
    if (git status --porcelain) { throw 'The working tree has uncommitted changes. Commit or stash them first.' }
    if (-not $SkipFullScan -and (Invoke-Guard @('-Root', $repo, '-Mode', 'rev', '-Rev', 'HEAD'))) { throw 'The private-data check failed on HEAD.' }

    if (Test-Path $Destination) {
        if (Get-ChildItem -Force $Destination | Select-Object -First 1) { throw "$Destination exists and is not empty." }
    } else {
        New-Item -ItemType Directory $Destination | Out-Null
    }
    $dest = (Resolve-Path $Destination).Path
    if ($dest.StartsWith($repo, [StringComparison]::OrdinalIgnoreCase)) { throw 'The destination must be outside this repository.' }

    $ignored = @(git ls-files -ci --exclude-standard)
    $exec = @(git ls-tree -r HEAD | Where-Object { $_ -match '^100755 ' } | ForEach-Object { ($_ -split "`t", 2)[1] })
    $tar = Join-Path ([IO.Path]::GetTempPath()) ("webox-export-{0}.tar" -f [guid]::NewGuid())
    if ($ignored.Count) { Write-Host 'left out (tracked but ignored by .gitignore):'; $ignored | ForEach-Object { Write-Host "  $_" } }
    $spec = @('.') + ($ignored | ForEach-Object { ":(exclude,literal)$_" })
    git -c core.autocrlf=false archive --format=tar -o $tar HEAD -- @spec
    if ($LASTEXITCODE) { throw 'git archive failed.' }
    tar -xf $tar -C $dest
    Remove-Item $tar
} catch {
    if ($dest) { Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue }
    throw
} finally { Pop-Location }

$ok = $false
Push-Location $dest
try {
    git init -q -b main
    git config core.autocrlf false
    git config core.filemode false
    git add -A
    git add --renormalize .                       # every text file LF in the new repository (.gitattributes)
    foreach ($f in $exec) { if (Test-Path -LiteralPath $f) { git update-index --chmod=+x -- $f } }
    if (-not $SkipFullScan -and (Invoke-Guard @('-Root', $dest, '-Mode', 'staged'))) { throw 'The private-data check failed on the exported tree.' }

    $env:GIT_AUTHOR_NAME = 'PianoNic'; $env:GIT_AUTHOR_EMAIL = '79938743+PianoNic@users.noreply.github.com'
    $env:GIT_COMMITTER_NAME = $env:GIT_AUTHOR_NAME; $env:GIT_COMMITTER_EMAIL = $env:GIT_AUTHOR_EMAIL
    $noHooks = Join-Path ([IO.Path]::GetTempPath()) 'webox-no-hooks'
    New-Item -ItemType Directory -Force $noHooks | Out-Null
    git -c core.hooksPath="$noHooks" -c commit.template= commit -q --no-verify -m $Message
    if ($LASTEXITCODE) { throw 'git commit failed.' }

    $meta = git log -1 --format='%an <%ae>%n%cn <%ce>%n%B'
    if ((Test-Attribution ($meta -join "`n")) -or ((git rev-list --count HEAD) -ne '1')) { throw 'The new commit carries attribution or is not a single commit.' }
    if (Invoke-Guard @('-Root', $dest, '-Mode', 'none', '-Commits', '-Rev', 'HEAD')) { throw 'The private-data check failed on the new commit''s message or identity.' }
    $ok = $true
    Write-Host "done: $dest"
    git log -1 --format='  %h %an <%ae>  %s'
    Write-Host 'No remote was added and nothing was pushed. See docs/publishing.md for the next steps.'
} finally {
    Remove-Item Env:GIT_AUTHOR_NAME, Env:GIT_AUTHOR_EMAIL, Env:GIT_COMMITTER_NAME, Env:GIT_COMMITTER_EMAIL -ErrorAction SilentlyContinue
    Pop-Location
    if (-not $ok) { Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue; Write-Host "removed the export again: $dest" }
}
