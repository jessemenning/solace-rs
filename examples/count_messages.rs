/// Subscribes to `>` and counts all messages received.
/// Exits after QUIET_PERIOD_SECS of silence (default 3s).
/// Prints the count and exits 0 if it matches EXPECTED_COUNT, 1 otherwise.
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use solace_rs::{message::InboundMessage, session::SessionEvent, Context, SolaceLogLevel};

fn main() {
    let url =
        std::env::var("SOLACE_BROKER_URL").unwrap_or_else(|_| "tcp://localhost:55555".to_string());
    let vpn = std::env::var("SOLACE_BROKER_VPN").unwrap_or_else(|_| "default".to_string());
    let username =
        std::env::var("SOLACE_BROKER_USERNAME").unwrap_or_else(|_| "default".to_string());
    let password = std::env::var("SOLACE_BROKER_PASSWORD").unwrap_or_default();
    let trust_store_dir = std::env::var("SOLACE_BROKER_TRUST_STORE_DIR").ok();
    let expected: usize = std::env::var("EXPECTED_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1033);
    let quiet_secs: u64 = std::env::var("QUIET_PERIOD_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let count = Arc::new(Mutex::new(0usize));
    let last_msg_time: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

    let count_clone = count.clone();
    let last_msg_clone = last_msg_time.clone();

    let on_message = move |_: InboundMessage| {
        *count_clone.lock().unwrap() += 1;
        *last_msg_clone.lock().unwrap() = Some(Instant::now());
    };

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let mut builder = ctx
        .session_builder()
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .on_message(on_message)
        .on_event(|_: SessionEvent| {});
    if let Some(dir) = trust_store_dir {
        builder = builder.ssl_trust_store_dir(dir);
    }
    let session = builder.build().expect("connecting to broker");
    session.subscribe(">").expect("subscribing to >");

    eprintln!(
        "Subscribed to >. Waiting for messages (quiet period: {}s)...",
        quiet_secs
    );

    loop {
        sleep(Duration::from_millis(200));
        let last = *last_msg_time.lock().unwrap();
        if let Some(t) = last {
            if t.elapsed() >= Duration::from_secs(quiet_secs) {
                break;
            }
        }
    }

    let final_count = *count.lock().unwrap();
    if final_count == expected {
        eprintln!(
            "PASS: received {} messages (expected {})",
            final_count, expected
        );
        std::process::exit(0);
    } else {
        eprintln!(
            "FAIL: received {} messages (expected {})",
            final_count, expected
        );
        std::process::exit(1);
    }
}
