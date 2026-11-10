#!/usr/bin/env bash
# Build the plugin for this machine and package it:
#   dist/yace-vim-<version>-linux-<arch>.tar.gz
# Needs `yace` on PATH (or $YACE): `yace --package-plugin` validates the
# manifest and prints the package's SHA-256.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
yace="${YACE:-yace}"
id="$(sed -n 's/^id = "\(.*\)"/\1/p' "$root/yace-plugin.toml")"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/yace-plugin.toml")"
arch="$(uname -m)"
cargo build --release --locked --manifest-path "$root/Cargo.toml"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$id/lib" "$root/dist"
# The package says which architecture its library is for.
sed "s/^os = \[\"linux\"\]$/os = [\"linux\"]\narch = [\"$arch\"]/" "$root/yace-plugin.toml" > "$stage/$id/yace-plugin.toml"
cp "$root/target/release/libyace_vim.so" "$stage/$id/lib/"
cp "$root/README.md" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$stage/$id/"
"$yace" --package-plugin "$stage/$id" -o "$root/dist/yace-vim-$version-linux-$arch.tar.gz"
