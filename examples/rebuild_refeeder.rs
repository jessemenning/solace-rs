//! Rebuild bake-off core (§12 tasks 2/12): drain a backlog from one of the two OSS
//! rebuild sources and measure throughput, order, and (for shadow) non-destructiveness.
//!
//!   replay  -> bind an EXCLUSIVE flow with `replay_start_location` on the durable ingest
//!              queue; the broker re-delivers from the replay LOG (the acked history).
//!   shadow  -> bind a `browser(true)` flow on the shadow queue; non-destructive browse
//!              drain — messages stay on the queue (verify depth unchanged via SEMP after).
//!
//! Both drain the same preloaded backlog (keyed `load_gen` output, payload = seq), so the
//! difference measured here is purely the SOURCE: replay-log re-delivery vs browse drain.
//! Feldera end-to-end catch-up is layered on top in the bake-off run.sh.
//!
//! Usage: cargo run --example rebuild_refeeder -- <replay|shadow>
//! Env: SOLACE_BROKER_URL (tcp://localhost:55555), *_VPN/_USERNAME/_PASSWORD,
//!      SRC_QUEUE (queue to drain), REPLAY_START (BEGINNING | rmid1:..., replay mode),
//!      IDLE_MS (end-of-drain quiet period, default 2000)
//!
//! Exits via process::exit to skip solace-rs Drop (known to core on teardown in this build;
//! unrelated to the drain — same note as replay_probe).

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use solace_rs::{
    flow::{AckMode, BindEntity},
    message::{InboundMessage, Message},
    session::SessionEvent,
    Context, SolaceLogLevel,
};

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

// Minimal per-message record the callback hands to main: the parsed seq (for order check)
// and the ADF msg_id (so main can ack — which advances the browse cursor on a browser flow,
// and removes the replayed message on a replay flow).
struct Rec {
    seq: Option<u64>,
    msg_id: u64,
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "replay".into());
    let host = env("SOLACE_BROKER_URL", "tcp://localhost:55555");
    let vpn = env("SOLACE_BROKER_VPN", "default");
    let user = env("SOLACE_BROKER_USERNAME", "default");
    let pass = env("SOLACE_BROKER_PASSWORD", "");
    let src_queue = env("SRC_QUEUE", "bakeoff-src-q");
    let replay_start = env("REPLAY_START", "BEGINNING");

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

    // Callback only enqueues; main processes + acks (ack must run on the main-owned flow, and
    // on a browser flow the ack is what advances the cursor past each flow window).
    let inbox: Arc<Mutex<VecDeque<Rec>>> = Arc::new(Mutex::new(VecDeque::new()));
    let ib = Arc::clone(&inbox);
    let on_message = move |m: InboundMessage| {
        let seq = m
            .get_payload()
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8_lossy(b).trim().parse::<u64>().ok());
        let msg_id = m.get_msg_id().ok().flatten().unwrap_or(0);
        ib.lock().unwrap().push_back(Rec { seq, msg_id });
    };

    // Build the source flow for the chosen mode. Client ack so main advances the cursor.
    let max_unacked: i32 = env("MAX_UNACKED", "100000").parse().unwrap_or(100000);
    let browse = mode == "shadow";
    // Browser: Auto ack (transport-level, NON-destructive). Never call flow.ack on a browser
    // — on a browser flow sendAck DELETES the message. Replay: Client ack to advance/remove.
    let ack_mode = if browse {
        AckMode::Auto
    } else {
        AckMode::Client
    };
    // Surface loud failures (replay-log trimmed, non-exclusive refusal, broker reject) as
    // flow events: a rebuild that cannot complete must be visible, never a silent short read.
    let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fe = Arc::clone(&failed);
    let mut fb = session
        .flow_builder()
        .bind_entity(BindEntity::Queue)
        .bind_name(src_queue.as_str())
        .ack_mode(ack_mode)
        .max_unacked_messages(max_unacked)
        .on_event(move |e: solace_rs::flow::event::FlowEvent| {
            use solace_rs::flow::event::FlowEvent::*;
            eprintln!("[flow event] {e}");
            if matches!(
                e,
                DownError | BindFailedError | RejectedMsgError | SessionDown
            ) {
                fe.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .on_message(on_message);
    fb = match mode.as_str() {
        "replay" => fb.replay_start_location(replay_start.as_str()),
        "shadow" => fb.browser(true),
        other => {
            eprintln!("unknown mode '{other}' (expected replay|shadow)");
            std::process::exit(2);
        }
    };
    let flow = match fb.build() {
        Ok(f) => f,
        Err(e) => {
            // e.g. client-initiated replay on a non-exclusive endpoint (subcode 136).
            println!("RESULT mode={mode} drained=0 FLOW_BUILD_ERROR=\"{e:?}\"");
            eprintln!("{mode}: flow build FAILED (loud): {e:?}");
            std::process::exit(0);
        }
    };
    let _ = flow.start();
    eprintln!("{mode}: draining '{src_queue}' ...");

    let mut count: u64 = 0; // unique messages drained
    let mut dups: u64 = 0;
    let mut resets: u64 = 0;
    let mut first: Option<Instant> = None;
    let mut last_new = Instant::now();
    let mut max_seq: u64 = 0;
    let mut out_of_order: u64 = 0;
    // Browse dedup: flow.start() resets the window and may re-deliver from the top, so track
    // seen seqs to count uniques and detect "a reset produced nothing new" (= browsed all).
    let mut seen: HashSet<u64> = HashSet::new();
    let reset_ms: u64 = env("RESET_MS", "500").parse().unwrap_or(500);
    let mut count_at_last_reset: u64 = 0;
    // If nothing ever arrives (e.g. a rejected/trimmed replay), give up after this and report
    // it loudly rather than hanging — a rebuild that delivers nothing must not look like success.
    let empty_timeout_ms: u64 = env("EMPTY_TIMEOUT_MS", "10000").parse().unwrap_or(10000);
    let loop_start = Instant::now();

    loop {
        let rec = inbox.lock().unwrap().pop_front();
        match rec {
            Some(rec) => {
                let is_new = match rec.seq {
                    Some(s) if browse => seen.insert(s),
                    _ => true,
                };
                if is_new {
                    if first.is_none() {
                        first = Some(Instant::now());
                    }
                    last_new = Instant::now();
                    count += 1;
                    if let Some(s) = rec.seq {
                        if s < max_seq {
                            out_of_order += 1;
                        }
                        max_seq = max_seq.max(s);
                    }
                    // Replay: ack to advance/remove. Browser: never ack (would delete).
                    if !browse && rec.msg_id != 0 {
                        if let Err(e) = flow.ack(rec.msg_id) {
                            if count <= 3 {
                                eprintln!("ack error at #{count}: {e}");
                            }
                        }
                    }
                } else {
                    dups += 1;
                }
            }
            None => {
                if count == 0 {
                    if failed.load(std::sync::atomic::Ordering::SeqCst)
                        || loop_start.elapsed() >= Duration::from_millis(empty_timeout_ms)
                    {
                        break; // nothing delivered — rejected/trimmed replay or empty source
                    }
                    sleep(Duration::from_millis(5));
                    continue;
                }
                if last_new.elapsed() < Duration::from_millis(reset_ms) {
                    sleep(Duration::from_millis(5));
                    continue;
                }
                // Stalled. Replay just ends; browser resets the window to browse more, and
                // stops once a reset yields no new uniques.
                if !browse {
                    break;
                }
                if count == count_at_last_reset {
                    break; // a full reset cycle produced nothing new → browsed everything
                }
                count_at_last_reset = count;
                resets += 1;
                let _ = flow.start();
                last_new = Instant::now();
            }
        }
    }

    let span = first
        .map(|f| last_new.duration_since(f).as_secs_f64())
        .unwrap_or(0.0)
        .max(1e-6);
    let tput = count as f64 / span;
    let ooo = out_of_order;
    if browse {
        eprintln!("shadow: {resets} window resets, {dups} duplicate re-deliveries across resets");
    }

    let failure_event = failed.load(std::sync::atomic::Ordering::SeqCst);
    // One machine-readable result line, plus a human summary.
    println!(
        "RESULT mode={mode} drained={count} span_s={span:.3} throughput_msg_s={tput:.0} out_of_order={ooo} failure_event={failure_event}"
    );
    eprintln!(
        "{mode}: drained {count} msgs in {span:.2}s = {tput:.0} msg/s, out_of_order={ooo}\
         {}",
        if mode == "shadow" {
            " (browse was non-destructive; verify queue depth unchanged via SEMP)"
        } else {
            ""
        }
    );
    std::process::exit(0);
}
