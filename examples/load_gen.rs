//! §12 task 5 load generator: publish a paced keyed stream to the input topic that the
//! exclusive `leader-q` subscribes to. Messages are Persistent so they are spooled into
//! the queue as guaranteed (the queue promotes them), giving the replicas at-least-once
//! delivery with measurable redelivery on an ungraceful failover.
//!
//! Usage:
//!   cargo run --example load_gen
//! Env (all optional):
//!   SOLACE_BROKER_URL (tcp://localhost:55555), SOLACE_BROKER_VPN (default),
//!   SOLACE_BROKER_USERNAME (default), SOLACE_BROKER_PASSWORD (""),
//!   INPUT_TOPIC (failover/in), RATE (msg/s, 200), DURATION_S (20), KEYS (8)
//!
//! Payload: `seq=<n>;key=k<n % KEYS>` (plain text).

use std::thread::sleep;
use std::time::{Duration, Instant};

use solace_rs::{
    message::{DeliveryMode, DestinationType, InboundMessage, MessageDestination, OutboundMessageBuilder},
    session::SessionEvent,
    Context, SolaceLogLevel,
};

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() {
    let host = std::env::var("SOLACE_BROKER_URL").unwrap_or_else(|_| "tcp://localhost:55555".into());
    let vpn = std::env::var("SOLACE_BROKER_VPN").unwrap_or_else(|_| "default".into());
    let user = std::env::var("SOLACE_BROKER_USERNAME").unwrap_or_else(|_| "default".into());
    let pass = std::env::var("SOLACE_BROKER_PASSWORD").unwrap_or_else(|_| "".into());
    let topic = std::env::var("INPUT_TOPIC").unwrap_or_else(|_| "failover/in".into());
    let rate = env_u64("RATE", 200).max(1);
    let duration_s = env_u64("DURATION_S", 20);
    let keys = env_u64("KEYS", 8).max(1);

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let session = ctx
        .session_builder()
        .host_name(host.as_str())
        .vpn_name(vpn.as_str())
        .username(user.as_str())
        .password(pass.as_str())
        .on_message(|_: InboundMessage| {})
        .on_event(|_: SessionEvent| {})
        .build()
        .expect("session build");

    let interval = Duration::from_micros(1_000_000 / rate);
    let total = rate * duration_s;
    eprintln!("load_gen: {rate} msg/s for {duration_s}s ({total} msgs) over {keys} keys → '{topic}'");

    let start = Instant::now();
    let mut seq: u64 = 0;
    while seq < total {
        let next = start + interval * (seq as u32);
        let now = Instant::now();
        if next > now {
            sleep(next - now);
        }
        let payload = format!("seq={};key=k{}", seq, seq % keys);
        let dest = MessageDestination::new(DestinationType::Topic, topic.as_str()).unwrap();
        let msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Persistent)
            .payload(payload)
            .build()
            .expect("build input msg");
        if let Err(e) = session.publish(msg) {
            eprintln!("load_gen publish error at seq={seq}: {e}");
        }
        seq += 1;
    }
    eprintln!("load_gen: done, published {seq} msgs");
    // Let the last persistent messages flush to the broker before the session drops.
    sleep(Duration::from_millis(500));
}
