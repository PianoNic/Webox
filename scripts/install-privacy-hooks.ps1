<#
.SYNOPSIS
  Installs the privacy guard as git hooks (pre-commit, pre-merge-commit, commit-msg) for a repository.

.DESCRIPTION
  Hooks live in the repository's COMMON git folder, so one install covers every worktree of it.
  The guard script is copied next to the hooks (<git-common-dir>/privacy-guard/), so worktrees whose
  branch is older than the guard are checked by the same, current rules. Re-run after changing the guard.

  pre-commit / pre-merge-commit : the staged files only (content as staged, file names, embedded text of binaries)
  commit-msg                    : the message and the author / committer identity git is about to use

  -DenyList records the private deny-list (a file, or a folder of denylist*.txt) in the repository's LOCAL
  git config (`webox.denylist`), which is never committed.

.EXAMPLE
  pwsh scripts/install-privacy-hooks.ps1 -DenyList ..\private
  pwsh scripts/install-privacy-hooks.ps1 -Repo C:\code\other-repo -DenyList ..\private
#>
param(
    [string]$Repo = (Join-Path $PSScriptRoot '..'),
    [string]$DenyList
)
$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path $Repo).Path
$common = (git -C $Repo rev-parse --path-format=absolute --git-common-dir).Trim()
if (git -C $Repo config --get core.hooksPath) { throw 'core.hooksPath is set; this installer writes to <git-common-dir>/hooks only.' }
$dir = Join-Path $common 'privacy-guard'
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'check-company-data.ps1') (Join-Path $dir 'check-company-data.ps1') -Force
if ($DenyList) {
    $p = (Resolve-Path $DenyList).Path
    git -C $Repo config --local --replace-all webox.denylist $p
}
if (-not (git -C $Repo config --get-all webox.denylist)) { Write-Host 'note: no private deny-list configured (webox.denylist); only the generic rules will run' }

$head = @'
#!/bin/sh
# webox privacy guard - installed by scripts/install-privacy-hooks.ps1; see docs/privacy-guard.md
guard="$(git rev-parse --path-format=absolute --git-common-dir)/privacy-guard/check-company-data.ps1"
pwsh="$(command -v pwsh || command -v pwsh.exe)"
if [ -z "$pwsh" ] || [ ! -f "$guard" ]; then
  echo "privacy guard: pwsh or $guard missing - commit blocked (run scripts/install-privacy-hooks.ps1)." >&2
  exit 1
fi
'@
$hooks = @{
    'pre-commit'       = '"$pwsh" -NoProfile -NonInteractive -File "$guard" -Mode staged -Root "$(git rev-parse --show-toplevel)" -Quiet -MaxShow 50 || { echo "privacy guard: commit blocked (see above)." >&2; exit 1; }'
    'pre-merge-commit' = '"$pwsh" -NoProfile -NonInteractive -File "$guard" -Mode staged -Root "$(git rev-parse --show-toplevel)" -Quiet -MaxShow 50 || { echo "privacy guard: merge blocked (see above)." >&2; exit 1; }'
    'commit-msg'       = '"$pwsh" -NoProfile -NonInteractive -File "$guard" -MessageFile "$1" -Root "$(git rev-parse --show-toplevel)" -Quiet || { echo "privacy guard: commit message or identity blocked (see above)." >&2; exit 1; }'
}
$hd = Join-Path $common 'hooks'
New-Item -ItemType Directory -Force $hd | Out-Null
foreach ($k in $hooks.Keys) {
    $f = Join-Path $hd $k
    if ((Test-Path $f) -and -not (Select-String -Path $f -SimpleMatch 'webox privacy guard' -Quiet)) { Copy-Item $f "$f.before-privacy-guard" -Force }
    [IO.File]::WriteAllText($f, ($head + "`n" + $hooks[$k] + "`n").Replace("`r`n", "`n"), [Text.UTF8Encoding]::new($false))
    if (-not $IsWindows) { chmod +x $f }
}
Write-Host "privacy hooks installed in $hd (pre-commit, pre-merge-commit, commit-msg); guard copy in $dir"
