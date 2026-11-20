<#
.SYNOPSIS
  Exports the committed tree into an existing public clone as one new commit on a release branch, ready for a pull request.

.DESCRIPTION
  The public repository gets one commit per update, never the development history. This script:
    1. refuses to run with uncommitted changes here or in the clone, or when the clone has untracked files;
    2. optionally runs scripts/check-company-data.ps1 on HEAD (-FullScan; every commit already passed the pre-commit hook);
    3. fetches origin in the clone (read only) and creates -Branch from origin/main;
    4. replaces every tracked file of the clone with HEAD of this repository (`git archive`; tracked files that .gitignore
       would ignore are left out; the clone's .git and its ignored files stay), stages with LF line endings and the
       executable bits of HEAD;
    5. checks the staged diff for attribution (and the private-data guard with -FullScan), commits it as
       PianoNic <79938743+PianoNic@users.noreply.github.com> with hooks disabled, and verifies it is exactly one commit
       on top of origin/main with a clean message and identity;
    6. writes the pull-request body (-Summary, else the conventional commit titles since the last export) to
       <state>/pr-body.md, checks it like a commit message, records the exported commit in <state>/last-export.txt and
       prints the push and `gh pr create` commands.
  It never pushes and never opens the pull request. If a step after 3 fails, the clone goes back to where it was.

.PARAMETER Clone       The public clone (has a remote `origin` and a `main` branch).
.PARAMETER Message     Title of the commit and the pull request, a conventional commit ("feat: ...", "fix: ...").
.PARAMETER Branch      Branch in the clone (default release/<yyyy-MM-dd>).
.PARAMETER Summary     Pull-request body. Default: the feat / fix / perf / refactor / docs titles of the first-parent
                       history since the commit in last-export.txt.
.PARAMETER DenyList    Deny-lists for the guard, as in publish-fresh.ps1. Needed for -FullScan and the message checks.
.PARAMETER StateDir    Where last-export.txt and pr-body.md live (default: publish-private next to the main checkout).

.EXAMPLE
  pwsh scripts/publish-update.ps1 -Clone C:\coding\Webox -Message 'feat: added the app updater and the Windows installer' -DenyList ..\publish-private
#>
param(
    [Parameter(Mandatory)][string]$Clone,
    [Parameter(Mandatory)][string]$Message,
    [Parameter(Mandatory)][string[]]$DenyList,
    [string]$Branch = ('release/' + (Get-Date -Format 'yyyy-MM-dd')),
    [string]$Summary,
    [string]$StateDir,
    # The two full-tree scans; off by default because every commit already passed the guard in the pre-commit hook.
    [switch]$FullScan,
    # Skip `git fetch origin` (offline tests); origin/main must then be current already.
    [switch]$NoFetch
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$lists = @($DenyList | ForEach-Object { $_ -split ';' } | Where-Object { $_ } | ForEach-Object { (Resolve-Path $_).Path }) -join ';'
$guard = Join-Path $repo 'scripts/check-company-data.ps1'
function Invoke-Guard([string[]]$a) { & pwsh -NoProfile -NonInteractive -File $guard -DenyList $lists @a | Out-Host; return $LASTEXITCODE }
function Git-Clone { git -C $clonePath @args; if ($LASTEXITCODE) { throw "git $($args -join ' ') failed in the clone." } }

# Attribution patterns, assembled from pieces so this file does not match itself.
$ai = 'Cl' + 'aude'; $an = 'Anth' + 'ropic'
$attribution = @(('Co-Authored' + '-By'), "Generated with .{0,20}$ai", "noreply@$an", "\b$ai\b", "\b$an\b")
function Test-Attribution([string]$text) {
    foreach ($rx in $attribution) { if ([regex]::IsMatch($text, $rx, 'IgnoreCase')) { return $true } }
    return $false
}
if (Test-Attribution $Message) { throw 'The commit message contains attribution.' }
if ($Message -notmatch '^(feat|fix|perf|refactor|docs|chore|ci|build|test)(\(.+\))?!?: .+') { throw 'Use a conventional commit title, e.g. "feat: added ...".' }
if ($Branch -notmatch '^[A-Za-z0-9._/-]+$') { throw "Not a usable branch name: $Branch" }

# State folder: publish-private beside the main checkout (also from a worktree).
if (-not $StateDir) {
    $common = (git -C $repo rev-parse --path-format=absolute --git-common-dir).Trim()
    $StateDir = Join-Path (Split-Path (Split-Path $common -Parent) -Parent) 'publish-private'
}
New-Item -ItemType Directory -Force $StateDir | Out-Null
$lastFile = Join-Path $StateDir 'last-export.txt'

$clonePath = (Resolve-Path $Clone).Path
if ($clonePath.StartsWith($repo, [StringComparison]::OrdinalIgnoreCase)) { throw 'The clone must be outside this repository.' }
if (-not (Test-Path (Join-Path $clonePath '.git'))) { throw "$clonePath is not a git clone." }

Push-Location $repo
try {
    if (git status --porcelain) { throw 'This repository has uncommitted changes. Commit or stash them first.' }
    $head = (git rev-parse HEAD).Trim()
    if ($FullScan -and (Invoke-Guard @('-Root', $repo, '-Mode', 'rev', '-Rev', 'HEAD'))) { throw 'The private-data check failed on HEAD.' }

    # The pull-request body: given, or the conventional titles since the last export.
    if (-not $Summary) {
        $since = if (Test-Path $lastFile) { (Get-Content $lastFile -TotalCount 1).Trim() } else { '' }
        if ($since -and -not (git cat-file -t $since 2>$null)) { $since = '' }
        $range = if ($since) { "$since..HEAD" } else { 'HEAD' }
        $titles = @(git log --first-parent --format=%s $range | Where-Object { $_ -match '^(feat|fix|perf|refactor|docs)(\(.+\))?!?: ' } | Select-Object -Unique)
        $Summary = if ($titles.Count) { ($titles | ForEach-Object { "- $_" }) -join "`n" } else { '- Maintenance and small fixes.' }
    }
    if (Test-Attribution $Summary) { throw 'The summary contains attribution.' }
    $ignored = @(git ls-files -ci --exclude-standard)
    $exec = @(git ls-tree -r HEAD | Where-Object { $_ -match '^100755 ' } | ForEach-Object { ($_ -split "`t", 2)[1] })
    $tar = Join-Path ([IO.Path]::GetTempPath()) ("webox-export-{0}.tar" -f [guid]::NewGuid())
    if ($ignored.Count) { Write-Host 'left out (tracked but ignored by .gitignore):'; $ignored | ForEach-Object { Write-Host "  $_" } }
    $spec = @('.') + ($ignored | ForEach-Object { ":(exclude,literal)$_" })
    git -c core.autocrlf=false archive --format=tar -o $tar HEAD -- @spec
    if ($LASTEXITCODE) { throw 'git archive failed.' }
} finally { Pop-Location }

$bodyFile = Join-Path $StateDir 'pr-body.md'
[IO.File]::WriteAllText($bodyFile, $Summary.Trim() + "`n", [Text.UTF8Encoding]::new($false))
$msgFile = Join-Path ([IO.Path]::GetTempPath()) ("webox-pr-{0}.txt" -f [guid]::NewGuid())
[IO.File]::WriteAllText($msgFile, "$Message`n`n$Summary`n", [Text.UTF8Encoding]::new($false))
function Set-PublicIdentity {
    $env:GIT_AUTHOR_NAME = 'PianoNic'; $env:GIT_AUTHOR_EMAIL = '79938743+PianoNic@users.noreply.github.com'
    $env:GIT_COMMITTER_NAME = $env:GIT_AUTHOR_NAME; $env:GIT_COMMITTER_EMAIL = $env:GIT_AUTHOR_EMAIL
}
function Clear-PublicIdentity { Remove-Item Env:GIT_AUTHOR_NAME, Env:GIT_AUTHOR_EMAIL, Env:GIT_COMMITTER_NAME, Env:GIT_COMMITTER_EMAIL -ErrorAction SilentlyContinue }
# The guard checks the message with the identity git would commit with, so the public one is set for it.
Set-PublicIdentity
try { if (Invoke-Guard @('-Root', $repo, '-Mode', 'none', '-MessageFile', $msgFile)) { Remove-Item $tar; throw 'The private-data check failed on the commit title or the summary.' } }
finally { Remove-Item $msgFile -ErrorAction SilentlyContinue; Clear-PublicIdentity }

# The clone: clean, on main, branch made from a fresh origin/main.
if (git -C $clonePath status --porcelain) { Remove-Item $tar; throw 'The clone has uncommitted or untracked files.' }
$startRef = (git -C $clonePath rev-parse --abbrev-ref HEAD).Trim()
if (-not $NoFetch) { Git-Clone fetch -q origin }
$base = (git -C $clonePath rev-parse origin/main).Trim()
if (git -C $clonePath rev-parse -q --verify "refs/heads/$Branch") { Remove-Item $tar; throw "Branch $Branch exists in the clone already." }

$ok = $false
try {
    Git-Clone config core.autocrlf false
    Git-Clone config core.filemode false
    Git-Clone switch -q -c $Branch $base
    # Replace the tracked content: delete every tracked file, unpack the export, stage everything.
    foreach ($f in (git -C $clonePath ls-files -z) -split "`0") { if ($f) { Remove-Item -LiteralPath (Join-Path $clonePath $f) -Force -ErrorAction SilentlyContinue } }
    tar -xf $tar -C $clonePath
    if ($LASTEXITCODE) { throw 'Unpacking the export failed.' }
    Git-Clone add -A
    Git-Clone add --renormalize .                    # every text file LF (.gitattributes)
    foreach ($f in $exec) { if (Test-Path -LiteralPath (Join-Path $clonePath $f)) { Git-Clone update-index --chmod=+x -- $f } }
    if (-not (git -C $clonePath diff --cached --name-only)) { throw 'Nothing changed since origin/main.' }
    $added = (git -C $clonePath diff --cached -U0 --no-color | Where-Object { $_ -match '^\+' -and $_ -notmatch '^\+\+\+ ' }) -join "`n"
    if (Test-Attribution $added) { throw 'The change adds attribution.' }
    if ($FullScan -and (Invoke-Guard @('-Root', $clonePath, '-Mode', 'staged'))) { throw 'The private-data check failed on the staged tree.' }

    Set-PublicIdentity
    $noHooks = Join-Path ([IO.Path]::GetTempPath()) 'webox-no-hooks'
    New-Item -ItemType Directory -Force $noHooks | Out-Null
    git -C $clonePath -c core.hooksPath="$noHooks" -c commit.template= commit -q --no-verify -m $Message
    if ($LASTEXITCODE) { throw 'git commit failed.' }

    $meta = git -C $clonePath log -1 --format='%an <%ae>%n%cn <%ce>%n%B'
    if (Test-Attribution ($meta -join "`n")) { throw 'The new commit carries attribution.' }
    if ((git -C $clonePath rev-list --count "$base..HEAD").Trim() -ne '1' -or (git -C $clonePath rev-parse HEAD~1).Trim() -ne $base) { throw 'The result is not exactly one commit on top of origin/main.' }
    if (Invoke-Guard @('-Root', $clonePath, '-Mode', 'none', '-Commits', '-Range', "$base..HEAD")) { throw 'The private-data check failed on the new commit''s message or identity.' }
    $ok = $true
} finally {
    Clear-PublicIdentity
    Remove-Item $tar -ErrorAction SilentlyContinue
    if (-not $ok) {
        # Back to the state before: the branch's files go, the start branch comes back, the new branch is deleted.
        git -C $clonePath reset -q --hard | Out-Null
        git -C $clonePath clean -q -fd | Out-Null
        git -C $clonePath switch -q -f $startRef | Out-Null
        git -C $clonePath branch -q -D $Branch 2>$null | Out-Null
        Write-Host "the clone is back on $startRef"
    }
}

[IO.File]::WriteAllText($lastFile, "$head`n", [Text.UTF8Encoding]::new($false))
Write-Host "done: one commit on $Branch in $clonePath"
git -C $clonePath log -1 --format='  %h %an <%ae>  %s'
git -C $clonePath diff --stat "$base..HEAD" | Select-Object -Last 1
Write-Host ''
Write-Host 'Nothing was pushed. Review it, then:'
Write-Host "  git -C `"$clonePath`" push -u origin $Branch"
Write-Host "  gh pr create --repo PianoNic/Webox --base main --head $Branch --title `"$($Message.Replace('"', '\"'))`" --body-file `"$bodyFile`""
