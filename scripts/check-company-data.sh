#!/usr/bin/env bash
# Fails if files, commit messages or commit identities hold private data or tool attribution.
# The checks live in check-company-data.ps1 (PowerShell 7, also on Linux/NixOS: `nix-shell -p powershell`);
# this wrapper passes every argument through, e.g.
#
#   scripts/check-company-data.sh                                  # working tree, deny-lists from git config webox.denylist
#   scripts/check-company-data.sh -DenyList ../private/denylist.txt -Mode rev -Rev main -Commits
#
# Without pwsh it fails closed: a guard that silently skips its checks is worse than none.
# See docs/privacy-guard.md.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
pwsh="$(command -v pwsh || command -v pwsh.exe || true)"
if [ -z "$pwsh" ]; then
  echo "check-company-data: PowerShell 7 (pwsh) is required (Linux/NixOS: nix-shell -p powershell)." >&2
  exit 3
fi
exec "$pwsh" -NoProfile -NonInteractive -File "$here/check-company-data.ps1" "$@"
