#!/bin/sh
# Sets the app version in Cargo.toml ([package] only) and in Cargo.lock's webox entry, so `cargo build --locked` still passes.
# plugin-api and the example plugins keep their own versions.
#   scripts/bump-version.sh 0.2.0
set -eu
v=${1:?usage: bump-version.sh <version>}
case "$v" in *[!0-9A-Za-z.+-]*|"") echo "not a version: $v" >&2; exit 1 ;; esac
cd "$(dirname "$0")/.."
awk -v v="$v" '
  /^\[package\]/ { p = 1 }
  /^\[/ && !/^\[package\]/ { p = 0 }
  p && !done && /^version *=/ { print "version = \"" v "\""; done = 1; next }
  { print }' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml
awk -v v="$v" '
  /^name = "webox"$/ { w = 1; print; next }
  w && /^version = / { print "version = \"" v "\""; w = 0; next }
  /^$/ { w = 0 }
  { print }' Cargo.lock > Cargo.lock.new
mv Cargo.lock.new Cargo.lock
grep -q "^version = \"$v\"" Cargo.toml || { echo "Cargo.toml was not changed" >&2; exit 1; }
