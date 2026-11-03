//! §12 task 5 output recorder: subscribe to the replicas' output topic and append every
//! row to a JSONL file as `<recv_ms> <payload-json>`. `analyze.py` reads this file to
//! compute the failover gap, the overlap-duplicate count, and the split-brain count.
//!
//! Usage:
//!   cargo run --example output_recorder
//! Env (all optional):
//!   SOLACE_BROKER_URL (tcp://localhost:55555), SOLACE_BROKER_VPN (default),
//!   SOLACE_BROKER_USERNAME (default), SOLACE_BROKER_PASSWORD (""),
//!   OUTPUT_TOPIC (failover/out), OUT_FILE (/tmp/failover-out.jsonl), RUN_S (0 = until killed)

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use solace_rs::{
    message::{InboundMessage, Message},
    session::SessionEvent,
    Context, SolaceLogLevel,
};

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}

fn main() {
    let host = std::env::var("SOLACE_BROKER_URL").unwrap_or_else(|_| "tcp://localhost:55555".into());
    let vpn = std::env::var("SOLACE_BROKER_VPN").unwrap_or_else(|_| "default".into());
    let user = std::env::var("SOLACE_BROKER_USERNAME").unwrap_or_else(|_| "default".into());
    let pass = std::env::var("SOLACE_BROKER_PASSWORD").unwrap_or_else(|_| "".into());
    let topic = std::env::var("OUTPUT_TOPIC").unwrap_or_else(|_| "failover/out".into());
    let out_file = std::env::var("OUT_FILE").unwrap_or_else(|_| "/tmp/failover-out.jsonl".into());
    let run_s: u64 = std::env::var("RUN_S").ok().and_then(|v| v.parse().ok()).unwrap_or(0);

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&out_file)
        .expect("open out file");
    let file = Arc::new(Mutex::new(file));
    let count = Arc::new(Mutex::new(0u64));

    let f = Arc::clone(&file);
    let c = Arc::clone(&count);
    let on_message = move |msg: InboundMessage| {
        if let Ok(Some(bytes)) = msg.get_payload() {
            // Record `<recv_ms> <topic> <payload>` — the topic carries the relaying gate id
            // in the egress rig; `-` when unavailable.
            let topic = msg
                .get_destination()
                .ok()
                .flatten()
                .and_then(|d| d.dest.into_string().ok())
                .unwrap_or_else(|| "-".into());
            let payload = String::from_utf8_lossy(bytes);
            let line = format!("{} {} {}\n", now_ms(), topic, payload);
            f.lock().unwrap().write_all(line.as_bytes()).ok();
            *c.lock().unwrap() += 1;
        }
    };

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
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

    session.subscribe(topic.as_str()).expect("subscribe output topic");
    eprintln!("output_recorder: recording '{topic}' → {out_file}");

    if run_s == 0 {
        loop {
            sleep(Duration::from_secs(3600));
        }
    } else {
        sleep(Duration::from_secs(run_s));
        let n = *count.lock().unwrap();
        file.lock().unwrap().flush().ok();
        eprintln!("output_recorder: recorded {n} rows to {out_file}");
    }
}
