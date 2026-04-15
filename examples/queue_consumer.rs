/// Demonstrates consuming guaranteed messages from a queue using `FlowBuilder`,
/// with `?` propagation via the top-level `SolaceError` type.
///
/// Usage:
///   cargo run --example queue_consumer
///
/// The example expects a pre-provisioned durable queue named `rust-test-queue`.
/// To use a different queue:
///   SOLACE_QUEUE=my-queue cargo run --example queue_consumer
use std::{
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use solace_rs::{
    flow::{AckMode, BindEntity},
    message::{
        DeliveryMode, DestinationType, InboundMessage, Message, MessageDestination,
        OutboundMessageBuilder,
    },
    session::SessionEvent,
    Context, SolaceError, SolaceLogLevel,
};

fn main() -> Result<(), SolaceError> {
    let host = option_env!("SOLACE_BROKER_URL").unwrap_or("tcp://localhost:55555");
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or("default");
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or("default");
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or("");
    let queue = option_env!("SOLACE_QUEUE").unwrap_or("rust-test-queue");

    // Context::new returns Result<Context, ContextError>; ? converts to SolaceError.
    let ctx = Context::new(SolaceLogLevel::Warning)?;

    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = Arc::clone(&received);

    // on_message callback: record payload strings.
    let on_message = move |msg: InboundMessage| {
        if let Ok(Some(bytes)) = msg.get_payload() {
            let text = String::from_utf8_lossy(bytes).to_string();
            println!("[flow] received: {text}");
            received_clone.lock().unwrap().push(text);
        }
    };

    // session_builder().build() returns Result<Session, SessionBuilderError>; ? converts.
    let session = ctx
        .session_builder()
        .host_name(host)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .on_message(|_: InboundMessage| {})
        .on_event(|e: SessionEvent| println!("[session] event: {e}"))
        .build()?;

    // Publish a test message to the queue before binding the flow.
    let dest = MessageDestination::new(DestinationType::Queue, queue).unwrap();
    let msg = OutboundMessageBuilder::new()
        .destination(dest)
        .delivery_mode(DeliveryMode::Persistent)
        .payload("hello from queue_consumer example")
        .build()?;
    session.publish(msg)?;
    println!("[session] published test message to queue: {queue}");

    // flow_builder().build() returns Result<Flow, FlowError>; ? converts to SolaceError.
    let _flow = session
        .flow_builder()
        .bind_entity(BindEntity::Queue)
        .bind_name(queue)
        .ack_mode(AckMode::Auto)
        .on_message(on_message)
        .build()?;

    println!("[flow] bound to queue '{queue}', waiting for delivery …");
    sleep(Duration::from_secs(2));

    let msgs = received.lock().unwrap();
    println!("[done] received {} message(s)", msgs.len());

    Ok(())
}
