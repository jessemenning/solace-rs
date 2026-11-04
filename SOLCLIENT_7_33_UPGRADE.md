# Solace C API 7.33.2.3 Upgrade Notes

Upgrade from **7.26.1.8** to **7.33.2.3**.

---

## OpenSSL

This is the most significant build-process change in 7.33.x.

### What changed

| | 7.26.x | 7.33.x (Linux) | 7.33.x (macOS) |
|---|---|---|---|
| OpenSSL built/tested against | 1.1 | 3.0.8 | 3.x (Homebrew) |
| OpenSSL at runtime | separate `.so` resolved at load | **system `libssl`/`libcrypto`, `dlopen`ed by name at runtime** | Homebrew `libssl`/`libcrypto`, dynamically linked |
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

No separate ssl/crypto **link** targets are needed on Linux — but this is **not** because
OpenSSL is statically embedded. `libsolclient.a` does not contain OpenSSL: its OpenSSL
entry points (`SSL_CTX_new`, `SSL_connect`, `EVP_*`, …) are **weak, undefined** symbols, and
the library links `libdl` and **`dlopen`s the system OpenSSL at runtime**, probing the SONAME
list `libcrypto.so.3, libcrypto.so.1.1, libcrypto.so` (and the `libssl` equivalents). You can
verify: `nm libsolclient.a | grep -E ' [TU] SSL_CTX_new'` shows it undefined, and
`strings libsolclient.a | grep -i 'libcrypto.so'` shows the search list.

Practically, this means **the OpenSSL that actually runs is whichever one is present on the
host/container at runtime**, not a fixed 3.0.8 baked into the archive. The runtime image must
therefore provide a `libssl`/`libcrypto` (the current `debian:bookworm-slim` runtime stage
installs only `ca-certificates`/`curl`, so TLS rides whatever `libssl` the base image happens
to carry).

### Runtime OpenSSL, pinning, and FIPS

Because OpenSSL is loaded at runtime, you control it:

- `SOLCLIENT_GLOBAL_PROP_SSL_LIB` / `SOLCLIENT_GLOBAL_PROP_CRYPTO_LIB` (global init properties,
  exposed by this crate via `GlobalCryptoConfig` / `Context::with_crypto`) pin the exact
  library the API loads — e.g. a FIPS-validated OpenSSL build whose `fips` provider is active.
- `SESSION_SSL_CIPHER_SUITES` / `SESSION_OPENSSL_SECURITY_LEVEL` (session properties, exposed
  on the session builder) restrict the handshake. **The API default cipher list contains
  non-FIPS-approved ciphers (RC4, MD5, DES/3DES)**, so a FIPS deployment must set an approved
  subset explicitly.

In other words, FIPS for the broker TLS path is a deployment concern (ship a FIPS OpenSSL +
restrict ciphers), not a vendor blocker — there is no separate "FIPS SDK" to obtain.

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

CVE-2025-31498 (CVSS 9.8 Critical) affected OpenSSL 1.x. The 7.33.x upgrade targets the
OpenSSL 3.x ABI; because OpenSSL is loaded from the host at runtime (see above), the actual
fix depends on the host/container shipping a patched OpenSSL 3.x — keep the base image's
`libssl`/`libcrypto` current.

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
