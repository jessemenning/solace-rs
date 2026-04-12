#![cfg(feature = "async")]

//! Integration tests for the async API: `AsyncSession`, `AsyncSessionBuilder`, `OwnedAsyncFlow`.
//!
//! All tests are `#[ignore]` and require a running broker.  Run them with:
//!
//!   cargo test --features async --test async_integration_test -- --include-ignored
//!
//! Queue-based tests (flow_*, disconnect_*) require a pre-provisioned durable queue named
//! `rust-test-queue` (or set `SOLACE_TEST_QUEUE`).
//! Create it via:  Broker CLI: `create queue rust-test-queue`
//!                 Broker Manager: Queues → Add Queue → Name: rust-test-queue, Access: Exclusive

use std::{future::poll_fn, pin::Pin, time::Duration};

use solace_rs::{
    async_support::{AsyncSession, AsyncSessionBuilder, OwnedAsyncFlow},
    flow::AckMode,
    message::{DeliveryMode, DestinationType, Message, MessageDestination, OutboundMessageBuilder},
    Context, SolaceLogLevel,
};

/// Per-message receive timeout – long enough for cloud broker round-trips.
static RECV_TIMEOUT: Duration = Duration::from_secs(10);

/// Brief pause to let subscriptions register before publishing.
static SLEEP_TIME: Duration = Duration::from_millis(50);

/// Longer pause used after publish when testing non-blocking try_recv().
/// 50ms is sufficient for local brokers but cloud (WSS) round-trips can
/// approach 200ms; 2s gives ample headroom.
static MSG_DELIVERY_SLEEP: Duration = Duration::from_millis(2000);

const DEFAULT_URL: &str = "tcp://localhost:55555";
const DEFAULT_VPN: &str = "default";
const DEFAULT_USERNAME: &str = "default";
const DEFAULT_PASSWORD: &str = "";

/// Pre-provisioned durable queue used by flow tests.
const DEFAULT_QUEUE: &str = "rust-test-queue";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_session(ctx: &Context) -> AsyncSession {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);

    let mut builder = AsyncSessionBuilder::new(ctx)
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password);

    if let Some(dir) = option_env!("SOLACE_BROKER_TRUST_STORE_DIR") {
        builder = builder.ssl_trust_store_dir(dir);
    }

    builder.build().expect("failed to create AsyncSession")
}

fn queue_name() -> &'static str {
    option_env!("SOLACE_TEST_QUEUE").unwrap_or(DEFAULT_QUEUE)
}

// ---------------------------------------------------------------------------
// AsyncSession — basic pub/sub via recv().await
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn async_subscribe_and_publish() {
    let topic = "async/subscribe_and_publish";
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let mut session = make_session(&ctx);

    session.subscribe(topic).expect("subscribe");
    tokio::time::sleep(SLEEP_TIME).await;

    let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Direct)
        .payload("hello-async")
        .build()
        .unwrap();
    session.publish(msg).expect("publish");

    let received = tokio::time::timeout(RECV_TIMEOUT, session.recv())
        .await
        .expect("timed out waiting for message")
        .expect("channel closed");

    assert_eq!(received.get_payload().unwrap().unwrap(), b"hello-async");
}

// ---------------------------------------------------------------------------
// AsyncSession — try_recv (non-blocking)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn async_try_recv_empty() {
    // No subscription, no published messages – channel must be empty.
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let mut session = make_session(&ctx);
    assert!(session.try_recv().is_err());
}

#[tokio::test]
#[ignore]
async fn async_try_recv_with_message() {
    let topic = "async/try_recv_with_message";
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let mut session = make_session(&ctx);

    session.subscribe(topic).expect("subscribe");
    tokio::time::sleep(SLEEP_TIME).await;

    let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Direct)
        .payload("try-recv-payload")
        .build()
        .unwrap();
    session.publish(msg).expect("publish");

    // Give the Solace callback time to deliver into the channel.
    // Use MSG_DELIVERY_SLEEP (2s) rather than SLEEP_TIME (50ms) because
    // WSS cloud brokers have ~100-200ms round-trip latency.
    tokio::time::sleep(MSG_DELIVERY_SLEEP).await;

    let received = session.try_recv().expect("expected a message in the channel");
    assert_eq!(
        received.get_payload().unwrap().unwrap(),
        b"try-recv-payload"
    );
}

// ---------------------------------------------------------------------------
// AsyncSessionBuilder — reconnect / TLS config passes through
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn async_session_builder_with_reconnect_config() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();

    let mut builder = AsyncSessionBuilder::new(&ctx)
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .reconnect_retries(-1) // unlimited
        .reconnect_retry_wait_ms(1_000)
        .reapply_subscriptions(true)
        .connect_timeout_ms(5_000);

    if let Some(dir) = option_env!("SOLACE_BROKER_TRUST_STORE_DIR") {
        builder = builder.ssl_trust_store_dir(dir);
    }

    let mut session = builder.build().expect("builder with reconnect config");

    let topic = "async/reconnect_config_test";
    session.subscribe(topic).expect("subscribe");
    tokio::time::sleep(SLEEP_TIME).await;

    let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Direct)
        .payload("reconnect-test")
        .build()
        .unwrap();
    session.publish(msg).expect("publish");

    let received = tokio::time::timeout(RECV_TIMEOUT, session.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    assert_eq!(received.get_payload().unwrap().unwrap(), b"reconnect-test");
}

// ---------------------------------------------------------------------------
// AsyncSession — futures_core::Stream impl
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn async_session_as_stream() {
    use futures_core::Stream;

    let topic = "async/session_as_stream";
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let mut session = make_session(&ctx);

    session.subscribe(topic).expect("subscribe");
    tokio::time::sleep(SLEEP_TIME).await;

    let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Direct)
        .payload("stream-payload")
        .build()
        .unwrap();
    session.publish(msg).expect("publish");

    // Drive the Stream implementation via poll_fn / poll_next.
    let received = tokio::time::timeout(
        RECV_TIMEOUT,
        poll_fn(|cx| Pin::new(&mut session).poll_next(cx)),
    )
    .await
    .expect("timed out")
    .expect("stream ended unexpectedly");

    assert_eq!(received.get_payload().unwrap().unwrap(), b"stream-payload");
}

// ---------------------------------------------------------------------------
// OwnedAsyncFlow — co-located with AsyncSession in the same struct (Gap 1)
//
// This is the key test for the Arc-based session redesign.  With the old
// lifetime API (`AsyncFlow<'flow, ...>`) this struct would not compile because
// the borrow of `session` for `flow` would outlive `session` itself.
// ---------------------------------------------------------------------------

struct SolaceSource {
    session: AsyncSession,
    flow: OwnedAsyncFlow,
}

#[tokio::test]
#[ignore]
async fn owned_flow_colocated_with_session() {
    let queue = queue_name();
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = make_session(&ctx);
    let flow = session.create_flow(queue, AckMode::Auto).expect("create_flow");

    // Store both in the same struct — lifetime-free, no borrow issues.
    let mut source = SolaceSource { session, flow };

    let dest = MessageDestination::new(DestinationType::Queue, queue).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Persistent)
        .payload("colocated-test")
        .build()
        .unwrap();
    source.session.publish(msg).expect("publish");

    let received = tokio::time::timeout(RECV_TIMEOUT, source.flow.recv())
        .await
        .expect("timed out waiting for guaranteed message")
        .expect("channel closed");

    assert_eq!(received.get_payload().unwrap().unwrap(), b"colocated-test");
}

// ---------------------------------------------------------------------------
// OwnedAsyncFlow — try_recv (non-blocking)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn owned_flow_try_recv_empty() {
    // Note: if the queue has leftover messages from a previous run this will fail.
    // Drain the queue before running this test in isolation.
    let queue = queue_name();
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = make_session(&ctx);
    let mut flow = session.create_flow(queue, AckMode::Auto).expect("create_flow");

    assert!(flow.try_recv().is_err());
}

// ---------------------------------------------------------------------------
// OwnedAsyncFlow — AckMode::Auto
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn owned_flow_auto_ack() {
    let queue = queue_name();
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = make_session(&ctx);
    let mut flow = session.create_flow(queue, AckMode::Auto).expect("create_flow");

    let dest = MessageDestination::new(DestinationType::Queue, queue).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Persistent)
        .payload("auto-ack-payload")
        .build()
        .unwrap();
    session.publish(msg).expect("publish");

    let received = tokio::time::timeout(RECV_TIMEOUT, flow.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    // In Auto mode the API acks implicitly – just verify delivery.
    assert_eq!(received.get_payload().unwrap().unwrap(), b"auto-ack-payload");
}

// ---------------------------------------------------------------------------
// OwnedAsyncFlow — AckMode::Client with explicit ack
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn owned_flow_client_ack() {
    let queue = queue_name();
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = make_session(&ctx);
    let mut flow = session
        .create_flow(queue, AckMode::Client)
        .expect("create_flow");

    let dest = MessageDestination::new(DestinationType::Queue, queue).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Persistent)
        .payload("client-ack-payload")
        .build()
        .unwrap();
    session.publish(msg).expect("publish");

    let received = tokio::time::timeout(RECV_TIMEOUT, flow.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    assert_eq!(
        received.get_payload().unwrap().unwrap(),
        b"client-ack-payload"
    );

    let msg_id = received
        .get_msg_id()
        .expect("get_msg_id error")
        .expect("persistent message must carry a msg_id");
    flow.ack(msg_id).expect("ack failed");
}

// ---------------------------------------------------------------------------
// Disconnect semantics — Arc refcount guards
// ---------------------------------------------------------------------------

/// disconnect() must return an error while any OwnedAsyncFlow still holds an Arc clone.
#[tokio::test]
#[ignore]
async fn disconnect_fails_with_active_flow() {
    let queue = queue_name();
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = make_session(&ctx);
    let _flow = session.create_flow(queue, AckMode::Auto).expect("create_flow");

    // `_flow` is still alive; Arc refcount > 1; disconnect must fail.
    assert!(session.disconnect().is_err());
    // `_flow` is dropped here, which drops the Arc clone.
}

/// After dropping all flows, disconnect() must succeed.
#[tokio::test]
#[ignore]
async fn disconnect_succeeds_after_flow_dropped() {
    let queue = queue_name();
    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = make_session(&ctx);
    let flow = session.create_flow(queue, AckMode::Auto).expect("create_flow");

    drop(flow); // releases the Arc clone → refcount drops to 1

    session
        .disconnect()
        .expect("disconnect should succeed after all flows are dropped");
}
