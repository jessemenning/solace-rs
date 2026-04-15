# Solace C API 7.33.2.3 Upgrade Notes

Upgrade from **7.26.1.8** to **7.33.2.3**.

---

## OpenSSL

This is the most significant build-process change in 7.33.x.

### What changed

| | 7.26.x | 7.33.x (Linux) | 7.33.x (macOS) |
|---|---|---|---|
| OpenSSL version | 1.1 | 3.0.8 | 3.x (Homebrew) |
| Delivery | Separate `.a` / `.so` files | **Embedded in `libsolclient.a`** | External (Homebrew) |
| Link targets needed | `solclient` + `solclientssl` + `crypto` + `ssl` | `solclient` only | `solclient` + `ssl` + `crypto` |

### Before (7.26.x link directives in `build.rs`)

```
cargo:rustc-link-lib=static=solclient
cargo:rustc-link-lib=static=solclientssl
cargo:rustc-link-lib=static=crypto
cargo:rustc-link-lib=static=ssl
```

### After (7.33.x — Linux)

```
cargo:rustc-link-lib=static=solclient
```

OpenSSL 3.0.8 is statically compiled into `libsolclient.a` on Linux. No separate ssl/crypto
link targets exist in the Linux 7.33.x distribution.

### After (7.33.x — macOS)

```
cargo:rustc-link-lib=static=solclient
cargo:rustc-link-lib=dylib=ssl
cargo:rustc-link-lib=dylib=crypto
```

On macOS, `libsolclient.a` does **not** embed OpenSSL. The library expects dynamic OpenSSL 3
from Homebrew. `build.rs` adds both the arm64 (`/opt/homebrew/opt/openssl@3/lib`) and x86_64
(`/usr/local/opt/openssl@3/lib`) Homebrew search paths so the correct dylib is found on
either architecture.

### Security impact

CVE-2025-31498 (CVSS 9.8 Critical) affected OpenSSL 1.x. The 7.33.x upgrade
closes this by moving to OpenSSL 3.0.8.

### Protocol changes

TLSv1.0 and SSLv3 are compiled out of OpenSSL 3.x and can no longer be negotiated.
The binding constants `SOLCLIENT_SESSION_PROP_SSL_PROTOCOL_TLSV1` and
`SOLCLIENT_SESSION_PROP_SSL_PROTOCOL_SSLV3` still exist for API compatibility
but setting them has no effect.

---

## Library download and linking

### Priority order (unchanged in behaviour, updated implementation)

The build resolves the C library location in this order:

1. **`SOLCLIENT_LIB_PATH`** — path to a directory containing pre-extracted library files.
   Skips all download logic.
2. **`SOLCLIENT_TARBALL_URL`** — URL to a `.tar.gz` to download. Required for platforms
   with no official public download URL (Windows, Linux aarch64).
3. **Default** — downloads from the official Solace URL for the current platform.
   Supported platforms: Linux x86_64, Linux x86_64 musl, macOS.

### Platform support matrix

| Platform | Official URL | Requires env var |
|---|---|---|
| Linux x86_64 | Yes | No |
| Linux x86_64 musl | Yes | No |
| macOS (universal2) | Yes | No |
| Windows | No | `SOLCLIENT_TARBALL_URL` or `SOLCLIENT_LIB_PATH` |
| Linux aarch64 | No | `SOLCLIENT_TARBALL_URL` or `SOLCLIENT_LIB_PATH` |

For platforms without an official URL, `build.rs` panics at build time with an
actionable error message if neither env var is set.

### Setting env vars via `.cargo/config.toml`

```toml
[env]
# Option 1: pre-extracted library directory
SOLCLIENT_LIB_PATH = "/path/to/solclient/lib"

# Option 2: custom tarball (required for Windows / aarch64)
SOLCLIENT_TARBALL_URL = "https://example.com/solclient-7.33.2.3.tar.gz"
```

---

## Binding changes (`solace_binding.rs`)

`solace_binding.rs` is regenerated from the 7.33.2.3 headers via `bindgen`.
Two breaking changes require call-site fixes in Rust code:

### 1. `solClient_propertyArray_pt` pointer mutability

The type changed from `*mut *const c_char` to `*mut *mut c_char`.

Any call site that passes a properties array must cast accordingly:

```rust
// Before
props.as_ptr() as ffi::solClient_propertyArray_pt

// After
props.as_ptr() as *mut *mut _
```

Affected call sites in this codebase: `context.rs`, `session/builder.rs`,
`flow/builder.rs`, `async_support.rs`.

### 2. `Default` impl removed from `solClient_flow_createRxCallbackFuncInfo_t`

The `Default` derive was removed from this struct in the new bindings.
Replace any `.default()` call with `unsafe { mem::zeroed() }`:

```rust
// Before
rxInfo: ffi::solClient_flow_createRxCallbackFuncInfo_t::default(),

// After
rxInfo: unsafe { mem::zeroed() },
```

`mem::zeroed()` is safe here because the struct contains only C primitive types
and pointers where zero/null is the correct unset value.

---

## macOS additional link flag

macOS requires `gssapi_krb5` as a dynamic link (unchanged from 7.26.x):

```
cargo:rustc-link-lib=dylib=gssapi_krb5
```

## Windows link flags

Windows links against the static OpenSSL libs bundled in the Win64 subdirectory
of the distribution plus the main client library as a dynamic import lib:

```
cargo:rustc-link-lib-static=libcrypto_s
cargo:rustc-link-lib-static=libssl_s
cargo:rustc-link-lib=libsolclient_s
```

Note: Windows still uses separate OpenSSL static libs — the embedded-OpenSSL
change described above applies to Linux and macOS only.
