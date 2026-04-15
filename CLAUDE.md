# solace-rs — Developer Reference

Unofficial Rust client library for the Solace Platform event broker. Safe, idiomatic Rust wrapper around the Solace C library via FFI.

- **Version:** 0.9.0
- **MSRV:** 1.82.0
- **License:** MIT
- **Crates.io:** https://crates.io/crates/solace-rs
- **Docs:** https://docs.rs/solace-rs

---

## Workspace layout

```
solace-rs/
├── src/                    # Main crate — safe high-level API
│   ├── lib.rs              # Error types, SolClientReturnCode, SolaceLogLevel
│   ├── context.rs          # Context — global broker context (Arc<Mutex<RawContext>>)
│   ├── session.rs          # Session — pub/sub, request-reply, flow creation
│   ├── session/
│   │   ├── builder.rs      # SessionBuilder — 30+ configuration options
│   │   └── event.rs        # SessionEvent enum
│   ├── flow/
│   │   ├── mod.rs          # Flow, AckMode, BindEntity, MessageOutcome
│   │   ├── builder.rs      # FlowBuilder — guaranteed message consumer
│   │   └── event.rs        # FlowEvent enum
│   ├── message/
│   │   ├── mod.rs          # Message trait, DeliveryMode, ClassOfService, CacheStatus
│   │   ├── inbound.rs      # InboundMessage
│   │   ├── outbound.rs     # OutboundMessage, OutboundMessageBuilder
│   │   └── destination.rs  # MessageDestination, DestinationType
│   ├── cache_session.rs    # CacheSession — SolCache wrapper (untested)
│   ├── async_support.rs    # AsyncSession, AsyncFlow (feature = "async", tokio)
│   └── util.rs             # FFI trampolines, error helpers
├── solace-rs-sys/          # Low-level FFI bindings (separate workspace member)
│   ├── build.rs            # bindgen + C library download/link logic
│   └── src/
│       ├── lib.rs
│       └── solace_binding.rs
├── examples/               # Seven runnable examples (broker on localhost:55554)
├── tests/
│   ├── integration_test.rs
│   └── async_integration_test.rs
├── docker-compose.yaml     # Spins up a Solace broker for integration tests
├── .env.example            # Environment variables for examples/tests
├── scripts/
│   ├── configure-broker.sh # Provision queues/subscriptions via SEMP
│   └── teardown-broker.sh  # Remove provisioned resources
└── .github/workflows/ci.yaml
```

---

## Key types and API surface

### `Context` (`src/context.rs`)
Thread-safe global context. Create once; clone for multiple sessions.

```rust
let ctx = Context::new(SolaceLogLevel::Warning)?;
let session = ctx.session(host, vpn, username, password, on_msg, on_event)?;
// or use the builder:
let session = ctx.session_builder()
    .host_name(host)
    // ...
    .build()?;
```

### `Session` (`src/session.rs`)
Generic over message callback `M: FnMut(InboundMessage)` and event callback `E: FnMut(SessionEvent)`.
Lifetime `'session` is bound to `Context` — the context must outlive all sessions.

Key methods:
| Method | Description |
|--------|-------------|
| `publish(msg)` | Send a direct or persistent message |
| `subscribe(topic)` | Add a topic subscription |
| `unsubscribe(topic)` | Remove a topic subscription |
| `request(msg, timeout_ms)` | Synchronous request-reply |
| `flow_builder()` | Create a `FlowBuilder` for guaranteed messaging |
| `cache_session(...)` | Create a `CacheSession` |
| `disconnect()` | Disconnect cleanly |

### `SessionBuilder` (`src/session/builder.rs`)
Fluent builder. Required fields: `host_name`, `vpn_name`, `username`, `password`, `on_message`, `on_event`.

Notable optional fields: `compression_level`, `reconnect_retries`, `ssl_trust_store_dir`, `connect_timeout_ms`, `buffer_size_bytes`, `no_local`, `reapply_subscriptions`.

### `Flow` (`src/flow/`)
Guaranteed message consumer bound to a queue or topic endpoint.
Lifetimes: `Flow<'flow, 'session, SM, FM>` — borrows the session. The flow-event type parameter `FE` has been erased; callers no longer need type annotations when omitting `.on_event()`.

| Method | Description |
|--------|-------------|
| `ack(msg_id)` | Acknowledge a message (Client ack mode) |
| `settle(msg_id, outcome)` | Settle with `Accepted / Failed / Rejected` |
| `start()` | Start message delivery |
| `stop()` | Pause delivery (messages remain queued) |

### `Message` trait (`src/message/mod.rs`)
Common interface for `InboundMessage` and `OutboundMessage`.

Key accessors: `get_payload()`, `get_destination()`, `get_reply_to()`, `get_correlation_id()`, `get_sequence_number()`, `get_sender_timestamp()`, `is_reply()`.

### `OutboundMessageBuilder`
Set `delivery_mode` (`Direct | Persistent | NonPersistent`), `destination`, `payload`, `class_of_service`, `priority`, `correlation_id`, `user_properties`, `eliding_eligible`.

### `AsyncSession` / `AsyncFlow` / `OwnedAsyncFlow` (feature `"async"`)
Tokio-based wrappers in `src/async_support.rs`.

| Type | Description |
|------|-------------|
| `AsyncSession` | Shared-ownership session (`Arc<Mutex<...>>`); works across `await` points |
| `AsyncFlow` | Future-based flow; lifetime-coupled to `AsyncSession` |
| `OwnedAsyncFlow` | `'static` flow with its own `Arc` clone of the session — can be co-located in the same struct as `AsyncSession`, eliminating lifetime coupling for RisingWave-style connectors |
| `AsyncSessionBuilder` | Builder for `AsyncSession`; configures reconnect, TLS, and connection timeouts |

`AsyncSessionBuilder` key options: `host_name`, `vpn_name`, `username`, `password`, `reconnect_retries`, `reconnect_retry_wait_ms`, `reapply_subscriptions`, `connect_timeout_ms`, `ssl_trust_store_dir`.

**WSS (WebSocket Secure) transport** is supported — set `host_name` to a `wss://` URL and provide `ssl_trust_store_dir`.

---

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `async` | off | Enables `AsyncSession` + `AsyncFlow` (requires tokio) |

---

## Error types (`src/lib.rs`)

```
SolaceError          — top-level; wraps all sub-errors via ?
  ContextError       — Context initialization
  SessionError       — connect, subscribe, publish, request failures
  SessionBuilderError — missing or invalid builder config
  FlowError          — flow create, start, stop, ack, settle
  MessageError       — field access on InboundMessage
  MessageBuilderError — OutboundMessageBuilder validation
```

`SolClientReturnCode` and `SolClientSubCode` are included in most error variants for C-layer diagnostics.

---

## C library linking (`solace-rs-sys/build.rs`)

Solace C API version: **7.33.2.3** (embedded OpenSSL 3.0.8).

Priority order (first found wins):
1. `SOLCLIENT_LIB_PATH` — local path to pre-extracted C library directory
2. `SOLCLIENT_TARBALL_URL` — URL to download a tarball (required for Windows and Linux aarch64)
3. Default — downloads from the official Solace download URL for the platform (Linux x64, macOS, musl only)

Platforms without a public download URL (Windows, Linux aarch64) will panic at build time with an actionable message if neither env var is set.

Only **static linking** is supported. As of 7.33.x, OpenSSL is embedded in `libsolclient.a` — there are no separate `libssl` / `libcrypto` / `libsolclientssl` link targets.

```toml
# .cargo/config.toml
[env]
SOLCLIENT_LIB_PATH = "/path/to/solclient/lib"
```

---

## Running examples

```bash
# Copy and fill in broker details
cp .env.example .env

cargo run --example topic_publisher
cargo run --example topic_subscriber
cargo run --example queue_consumer
cargo run --example request_reply
```

Examples assume broker on `localhost:55554`, VPN `default`, user `default`.

---

## Integration tests

```bash
# Start a broker
docker compose up -d

# Run integration tests
cargo test

# Async tests
cargo test --features async
```

CI runs tests against Solace standard edition 10.5 on Linux. macOS and Windows run build + release tests only (no broker).

---

## CI

`.github/workflows/ci.yaml` checks:
- `cargo fmt`, `cargo clippy`, doc tests (Rust 1.82)
- Build on macOS, Windows
- Integration tests on Linux with Docker broker

---

## Design notes

- **FFI safety**: `PhantomData` and lifetime parameters prevent C pointers from escaping Rust scopes.
- **Callback trampolines**: `src/util.rs` bridges C callbacks to Rust `FnMut` closures via double-boxing.
- **Lifetime chain**: `Context → Session → Flow` — each must outlive its dependents.
- **Tracing**: uses the `tracing` crate throughout; add `tracing-subscriber` in your app to capture logs.
- **`SolaceLogLevel`** controls C-layer verbosity; set to `Warning` or higher in production.

---

## Environment variables (examples / tests)

| Variable | Description |
|----------|-------------|
| `SOLACE_BROKER_URL` | e.g. `tcp://localhost:55555` |
| `SOLACE_BROKER_VPN` | VPN name (e.g. `default`) |
| `SOLACE_BROKER_USERNAME` | Client username |
| `SOLACE_BROKER_PASSWORD` | Client password |
| `SOLACE_BROKER_TRUST_STORE_DIR` | TLS cert store directory — required for WSS transport; maps to `SOLCLIENT_SESSION_PROP_SSL_TRUST_STORE_DIR` |
| `SEMP_USERNAME` | Management API user (test setup/teardown) |
| `SEMP_PASSWORD` | Management API password |
