#!/bin/sh
set -eu

target=aarch64-unknown-linux-gnu
if ! rustup target list --installed | grep -qx "$target"; then
  echo "missing Rust target $target; run: rustup target add $target" >&2
  exit 1
fi
if ! command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
  echo "missing aarch64-linux-gnu-gcc cross linker" >&2
  exit 1
fi

cargo pi-build
file target/$target/release/controlroom
