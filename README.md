# Solace-rs

[![crates.io](https://img.shields.io/crates/v/solace-rs.svg)](https://crates.io/crates/solace-rs)
[![docs.rs](https://docs.rs/solace-rs/badge.svg)](https://docs.rs/solace-rs/)
[![ci](https://github.com/asimsedhain/solace-rs/actions/workflows/ci.yaml/badge.svg)](https://github.com/asimsedhain/solace-rs/actions/workflows/ci.yaml)


The Unofficial Solace Platform Rust Client Library.

Focuses on providing safe and idiomatic rust API over the C Solace library.



## Features

- [x] Publishing and subscribing
    - [x] Direct
    - [x] Persistent
- [x] Solcache - (Untested)
- [x] Request Reply
- [x] Async - `AsyncSession` and `AsyncFlow` available via the `async` feature flag (requires tokio)

## Installation

```bash
cargo add solace-rs

```

### Configuring Solace Library Link

Only static linking is supported. The build downloads the Solace C API 7.33.2.3 automatically for Linux x64, macOS, and musl. Priority order (first found wins):

1. **`SOLCLIENT_LIB_PATH`** — path to a directory containing the pre-extracted C library files
2. **`SOLCLIENT_TARBALL_URL`** — URL to download a tarball (required for Windows and Linux aarch64, which have no public download URL)
3. **Default** — downloads from the official Solace download URL for your platform

Set via [configurable-env](https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#configurable-env) in your [`.cargo/config.toml`](https://doc.rust-lang.org/cargo/reference/config.html):

```toml
[env]
# Option 1: pre-extracted library
SOLCLIENT_LIB_PATH = "/path/to/solclient/lib"

# Option 2: custom tarball URL
SOLCLIENT_TARBALL_URL = "https://example.com/solclient-7.33.2.3.tar.gz"
```


## Examples

You can find examples in the [examples folder](./examples). The examples assume you have solace running on `localhost:55554`. To run them:

```bash
cargo run --example <example_name> -- <example_args>
```

## Minimum supported Rust version (MSRV)

The current minimum supported Rust version (MSRV) is 1.82

## OS Support / CI Tests

- [x] linux
- [x] linux-musl
- [x] macos-12
- [x] windows

