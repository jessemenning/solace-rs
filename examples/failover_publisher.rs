//! §12 task 5: one replica of the two-replica forced-failover rig.
//!
//! A replica binds an **exclusive** flow to the shared `leader-q`. The broker delivers
//! messages only to the currently-Active flow, so the exclusive queue is simultaneously
//! the **leader lock** and the **work queue** — the standby replica is bound but starved
//! until it is promoted. This mirrors a single-queue sidecar. Each input message is
//! turned into an output row (a per-key rolling count) published to `OUTPUT_TOPIC`, then
//! the input is **acked after the publish** so an ungraceful death leaves the in-flight,
//! unacked messages to be redelivered to the standby — the measurable at-least-once
//! overlap.
//!
//! Run two of these against one broker (see `failover-rig/run.sh`):
//!   - graceful handoff  — `SIGTERM` the leader; the handler drops the flow (clean unbind).
//!   - ungraceful handoff — `SIGKILL` the leader; the broker detects the lost flow by
//!                          keepalive and redelivers whatever was unacked.
//!
//! Usage:
//!   cargo run --example failover_publisher -- <replica-id>
//! Env (all optional):
//!   SOLACE_BROKER_URL (tcp://localhost:55555), SOLACE_BROKER_VPN (default),
//!   SOLACE_BROKER_USERNAME (default), SOLACE_BROKER_PASSWORD (""),
//!   LEADER_Q (leader-q), OUTPUT_TOPIC (failover/out)
//!
//! Input payload format (from `load_gen`): `seq=<n>;key=<k>` (plain text, no serde dep).
//! Output payload: one hand-rolled JSON object per line (parsed by `analyze.py`).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use solace_rs::{
    flow::{event::FlowEvent, AckMode, BindEntity},
    message::{
        DeliveryMode, DestinationType, InboundMessage, Message, MessageDestination,
        OutboundMessageBuilder,
    },
    session::SessionEvent,
    Context, SolaceLogLevel,
};

// Set by the SIGTERM/SIGINT handler; the main loop sees it and drops the flow cleanly.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

struct InRec {
    input_seq: u64,
    key: String,
    msg_id: u64,
    redelivered: bool,
}

/// Parse `seq=<n>;key=<k>`; returns None if malformed.
fn parse_input(s: &str) -> Option<(u64, String)> {
    let mut seq = None;
    let mut key = None;
    for part in s.split(';') {
        let mut it = part.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("seq"), Some(v)) => seq = v.trim().parse::<u64>().ok(),
            (Some("key"), Some(v)) => key = Some(v.trim().to_string()),
            _ => {}
        }
    }
    Some((seq?, key?))
}

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "A".into());
    let host =
        std::env::var("SOLACE_BROKER_URL").unwrap_or_else(|_| "tcp://localhost:55555".into());
    let vpn = std::env::var("SOLACE_BROKER_VPN").unwrap_or_else(|_| "default".into());
    let user = std::env::var("SOLACE_BROKER_USERNAME").unwrap_or_else(|_| "default".into());
    let pass = std::env::var("SOLACE_BROKER_PASSWORD").unwrap_or_else(|_| "".into());
    let leader_q = std::env::var("LEADER_Q").unwrap_or_else(|_| "leader-q".into());
    let out_topic = std::env::var("OUTPUT_TOPIC").unwrap_or_else(|_| "failover/out".into());

    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
    }

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();

    let is_leader = Arc::new(AtomicBool::new(false));
    let inbox: Arc<Mutex<VecDeque<InRec>>> = Arc::new(Mutex::new(VecDeque::new()));

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

    // Flow event callback: track leadership and log the promotion/demotion instants.
    let ev_leader = Arc::clone(&is_leader);
    let ev_id = id.clone();
    let on_event = move |e: FlowEvent| match e {
        FlowEvent::Active => {
            ev_leader.store(true, Ordering::SeqCst);
            println!("{ev_id} ACTIVE {}", now_ms());
        }
        FlowEvent::Inactive => {
            ev_leader.store(false, Ordering::SeqCst);
            println!("{ev_id} INACTIVE {}", now_ms());
        }
        _ => {}
    };

    // Message callback: only the Active flow receives these (exclusive queue). Forward the
    // minimal facts to the main loop — no publish/ack here, so publish-then-ack ordering
    // and the rolling-count state stay single-threaded in main().
    let msg_inbox = Arc::clone(&inbox);
    let on_message = move |msg: InboundMessage| {
        let payload = match msg.get_payload() {
            Ok(Some(b)) => String::from_utf8_lossy(b).to_string(),
            _ => return,
        };
        let Some((input_seq, key)) = parse_input(&payload) else {
            return;
        };
        let msg_id = msg.get_msg_id().ok().flatten().unwrap_or(0);
        let redelivered = msg.is_redelivered();
        msg_inbox.lock().unwrap().push_back(InRec {
            input_seq,
            key,
            msg_id,
            redelivered,
        });
    };

    let flow = session
        .flow_builder()
        .bind_entity(BindEntity::Queue)
        .bind_name(leader_q.as_str())
        .ack_mode(AckMode::Client)
        .active_flow_indication(true)
        .on_event(on_event)
        .on_message(on_message)
        .build()
        .expect("flow build");

    eprintln!("{id} bound to exclusive '{leader_q}', publishing to '{out_topic}' …");

    // Drain loop: pop an input, advance the per-key rolling count, publish the output row,
    // THEN ack the input. We do NOT guard on is_leader: receiving a message on our own
    // exclusive flow already means we are the leader, and the single exclusive queue makes
    // simultaneous double-publish impossible — so guarding would only risk dropping a
    // legitimately-received message in the promotion window.
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut output_seq: u64 = 0;

    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
        let rec = inbox.lock().unwrap().pop_front();
        let Some(rec) = rec else {
            sleep(Duration::from_millis(1));
            continue;
        };

        let count = {
            let c = counts.entry(rec.key.clone()).or_insert(0);
            *c += 1;
            *c
        };
        output_seq += 1;
        let pub_ts = now_ms();
        let payload = format!(
            "{{\"replica\":\"{}\",\"input_seq\":{},\"key\":\"{}\",\"output_seq\":{},\"count\":{},\"pub_ts\":{},\"redelivered\":{}}}",
            id, rec.input_seq, rec.key, output_seq, count, pub_ts, rec.redelivered
        );

        let dest = MessageDestination::new(DestinationType::Topic, out_topic.as_str()).unwrap();
        let msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(payload)
            .build()
            .expect("build output msg");
        if let Err(e) = session.publish(msg) {
            eprintln!("{id} publish error: {e}");
        }
        if rec.msg_id != 0 {
            if let Err(e) = flow.ack(rec.msg_id) {
                eprintln!("{id} ack error: {e}");
            }
        }
    }

    // Clean unbind on SIGTERM: dropping the flow releases the exclusive binding, which the
    // broker turns into an immediate Active grant for the standby (graceful handoff).
    drop(flow);
    println!("{id} STOPPED {}", now_ms());
}
