#!/usr/bin/env bash
# Catch aarch64-only compile failures in solace-rs (the C-FFI / c_char-signedness
# class) BEFORE they reach a native-arm64 CI runner.
#
# It runs `cargo check` inside an emulated linux/arm64 container, so build.rs
# runs natively as aarch64: bindgen generates aarch64 bindings (c_char == u8)
# and the official aarch64 solclient (C_API_ARM) is fetched — the exact
# conditions a cross-compile from an x86 host would NOT reproduce.
#
# Run this after any change to solace-rs, before pushing and before bumping the
# rev in a consumer (e.g. the feldera fork).
#
# One-time host setup (registers the qemu arm64 handler; reversible with
# `--uninstall arm64`):
#   docker run --privileged --rm tonistiigi/binfmt --install arm64
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

RUST_IMG="${RUST_IMG:-rust:1.99-bookworm}"

# --- emulation guard ---------------------------------------------------------
if [[ "$(docker run --rm --platform linux/arm64 alpine uname -m 2>/dev/null || true)" != "aarch64" ]]; then
  echo "ERROR: linux/arm64 emulation is not registered." >&2
  echo "Run once:  docker run --privileged --rm tonistiigi/binfmt --install arm64" >&2
  exit 1
fi

echo "==> cargo check -p solace-rs --features async  (linux/arm64, emulated)"
# Named volumes cache the crate registry and target dir across runs; a separate
# CARGO_TARGET_DIR keeps the aarch64 artifacts out of the host's x86 target/.
docker run --rm --platform linux/arm64 \
  -v "$PWD":/src -w /src \
  -v solace-rs-aarch64-cargo:/usr/local/cargo/registry \
  -v solace-rs-aarch64-target:/target \
  -e CARGO_TARGET_DIR=/target \
  "$RUST_IMG" bash -c '
    apt-get update -qq && apt-get install -y -qq libclang-dev pkg-config >/dev/null
    cargo check -p solace-rs --features async
  '

printf '\n\033[1;32maarch64 check passed.\033[0m\n'
