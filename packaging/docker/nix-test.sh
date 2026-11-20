#!/bin/sh
# Runs the commands of docs/INSTALL-NIX.md in a nixos/nix container against a git copy of the repo at $1 (default /work).
#   docker run --rm -v webox-nix:/nix -v "$PWD/shots:/out" -v <tar of the repo>:/in/src.tar nixos/nix sh /work/packaging/docker/nix-test.sh
set -u
REPO=${1:-/work}
OUT=${2:-/out}
export NIX_CONFIG="experimental-features = nix-command flakes
max-jobs = auto
cores = 0"
cd "$REPO"
step() { echo; echo "== $*"; }

step "nix flake check"
nix flake check --no-build 2>&1 | tail -5; echo "exit $?"
step "nix build (default + webox-nixos)"
nix build -L .#default .#webox-nixos 2>&1 | grep -E "warning|error" | head; echo "build exit $?"
ls -l result*/bin; nix path-info -Sh ./result ./result-1
step "wrapper"
cat result/bin/webox | grep -v '^#' | head -20
step "nix run under Xvfb (no /run/opengl-driver -> Mesa fallback)"
nix shell nixpkgs#xvfb-run -c xvfb-run -a -s "-screen 0 1400x900x24" nix run path:$REPO -- --sample --screenshot "$OUT/nix_run_sample.png" 2>&1 | tail -3
ls -l "$OUT/nix_run_sample.png"
step "nix profile install / list / upgrade / remove"
nix profile remove work >/dev/null 2>&1   # a leftover of an earlier run (the entry is named after the folder / repository)
nix profile install path:$REPO 2>&1 | tail -2
nix profile list 2>&1 | head -8
command -v webox && ls ~/.nix-profile/share/applications ~/.nix-profile/share/icons/hicolor | head -12
nix profile upgrade work 2>&1 | tail -2; echo "upgrade exit $?"
nix profile remove work 2>&1 | tail -2; echo "remove exit $?"
nix profile list 2>&1 | head -3
step "modules"
sh "$REPO/packaging/nix/module-test.sh" "$REPO" 2>&1 | grep -v '^warning'
step "nix develop"
nix develop path:$REPO -c sh -c 'cargo --version; rustc --version; pkg-config --modversion webkit2gtk-4.1; echo "LD_LIBRARY_PATH set: ${LD_LIBRARY_PATH:+yes}"' 2>&1 | tail -4
