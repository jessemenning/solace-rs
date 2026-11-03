//! §12 task 5, final Capability-2 piece: the egress gate.
//!
//! One gate sits in front of one warm Feldera replica. It holds the leader lock (an
//! active-flow binding on an exclusive `leader-q`, used purely for election — no work
//! messages flow through it) and **relays its replica's Feldera output to the shared
//! downstream only while it is the leader**. Both replicas stay warm (they ingest the same
//! stream — see `failover-rig/feldera-lockstep/`), so when the leader dies the standby gate
//! is promoted and relays its *already-correct* Feldera output: downstream continues from
//! the right per-key count with no reset and only the lock-handoff gap.
//!
//! Usage:
//!   cargo run --example egress_gate -- <gate-id>
//! Env:
//!   SOLACE_BROKER_URL (tcp://localhost:55555), *_VPN/_USERNAME/_PASSWORD,
//!   LEADER_Q (leader-q),   GATE_IN (failover/fout/<id>/>),   GATE_OUT (failover/out/<id>)

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use solace_rs::{
    flow::{event::FlowEvent, AckMode, BindEntity},
    message::{
        DeliveryMode, DestinationType, InboundMessage, Message, MessageDestination,
        OutboundMessageBuilder,
    },
    session::SessionEvent,
    Context, SolaceLogLevel,
};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
extern "C" fn on_signal(_sig: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "A".into());
    let host = std::env::var("SOLACE_BROKER_URL").unwrap_or_else(|_| "tcp://localhost:55555".into());
    let vpn = std::env::var("SOLACE_BROKER_VPN").unwrap_or_else(|_| "default".into());
    let user = std::env::var("SOLACE_BROKER_USERNAME").unwrap_or_else(|_| "default".into());
    let pass = std::env::var("SOLACE_BROKER_PASSWORD").unwrap_or_else(|_| "".into());
    let leader_q = std::env::var("LEADER_Q").unwrap_or_else(|_| "leader-q".into());
    let gate_in = std::env::var("GATE_IN").unwrap_or_else(|_| format!("failover/fout/{}/>", id.to_lowercase()));
    let gate_out = std::env::var("GATE_OUT").unwrap_or_else(|_| format!("failover/out/{}", id.to_lowercase()));

    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
    }

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let is_leader = Arc::new(AtomicBool::new(false));
    // Feldera-output payloads received while leader, awaiting relay by the main thread
    // (which owns the session for publishing).
    let relay_q: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));

    // Session on_message receives the GATE_IN subscription (this replica's Feldera output).
    // Enqueue only while leader; non-leaders drop their replica's output on the floor.
    let sess_leader = Arc::clone(&is_leader);
    let sess_q = Arc::clone(&relay_q);
    let on_message = move |msg: InboundMessage| {
        if !sess_leader.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(Some(bytes)) = msg.get_payload() {
            sess_q.lock().unwrap().push_back(bytes.to_vec());
        }
    };

    let session = ctx
        .session_builder()
        .host_name(host.as_str())
        .vpn_name(vpn.as_str())
        .username(user.as_str())
        .password(pass.as_str())
        .on_message(on_message)
        .on_event(|_: SessionEvent| {})
        .build()
        .expect("session build");

    session.subscribe(gate_in.as_str()).expect("subscribe gate_in");

    // Leader lock: exclusive flow on leader-q, election only (no messages expected).
    let ev_leader = Arc::clone(&is_leader);
    let ev_id = id.clone();
    let flow = session
        .flow_builder()
        .bind_entity(BindEntity::Queue)
        .bind_name(leader_q.as_str())
        .ack_mode(AckMode::Auto)
        .active_flow_indication(true)
        .on_event(move |e: FlowEvent| match e {
            FlowEvent::Active => {
                ev_leader.store(true, Ordering::SeqCst);
                println!("{ev_id} ACTIVE {}", now_ms());
            }
            FlowEvent::Inactive => {
                ev_leader.store(false, Ordering::SeqCst);
                println!("{ev_id} INACTIVE {}", now_ms());
            }
            _ => {}
        })
        .on_message(|_: InboundMessage| {})
        .build()
        .expect("flow build");

    eprintln!("gate {id}: lock='{leader_q}', in='{gate_in}', out='{gate_out}'");

    // Relay loop: forward queued Feldera output to the shared downstream (verbatim).
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            break;
        }
        let item = relay_q.lock().unwrap().pop_front();
        let Some(bytes) = item else {
            sleep(Duration::from_millis(1));
            continue;
        };
        let dest = MessageDestination::new(DestinationType::Topic, gate_out.as_str()).unwrap();
        let msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(bytes)
            .build()
            .expect("build relay msg");
        if let Err(e) = session.publish(msg) {
            eprintln!("gate {id} relay publish error: {e}");
        }
    }

    drop(flow); // clean unbind on SIGTERM → standby gate promoted
    println!("{id} STOPPED {}", now_ms());
}
