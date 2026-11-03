//! De-risk probe for the in-connector egress gate (§12 task 5).
//!
//! The Feldera solace_output connector uses the ASYNC session (`AsyncSession`), whose flow
//! API (`create_flow`) did not expose active-flow indication — the leader lock the gate
//! needs. This probe exercises the new `create_flow_active(.., active_flow_indication=true)`
//! on the async path end to end: two async flows bind the same EXCLUSIVE queue, the broker
//! must grant `Active` to exactly one, and on dropping the leader the standby must be
//! promoted — all through the async event channel, with no crash (the async event trampoline
//! is the same one fixed for the sync path).
//!
//! Run (requires an exclusive queue `leader-q` — e.g. from failover-rig/provision.sh):
//!   SOLACE_BROKER_PASSWORD=default cargo run --features async --example async_leader_probe
//!
//! Env: SOLACE_BROKER_URL (tcp://localhost:55555), *_VPN/_USERNAME/_PASSWORD, LEADER_Q (leader-q)

#[cfg(not(feature = "async"))]
fn main() {
    eprintln!(
        "build with --features async: cargo run --features async --example async_leader_probe"
    );
}

#[cfg(feature = "async")]
#[tokio::main]
async fn main() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use solace_rs::async_support::{AsyncSessionBuilder, OwnedAsyncFlow};
    use solace_rs::flow::{AckMode, FlowEvent};
    use solace_rs::{Context, SolaceLogLevel};

    fn now_ms() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    let host =
        std::env::var("SOLACE_BROKER_URL").unwrap_or_else(|_| "tcp://localhost:55555".into());
    let vpn = std::env::var("SOLACE_BROKER_VPN").unwrap_or_else(|_| "default".into());
    let user = std::env::var("SOLACE_BROKER_USERNAME").unwrap_or_else(|_| "default".into());
    let pass = std::env::var("SOLACE_BROKER_PASSWORD").unwrap_or_else(|_| "".into());
    let queue = std::env::var("LEADER_Q").unwrap_or_else(|_| "leader-q".into());

    let ctx = Context::new(SolaceLogLevel::Warning).unwrap();
    let mk_flow = |label: &'static str| -> OwnedAsyncFlow {
        let session = AsyncSessionBuilder::new(&ctx)
            .host_name(host.clone())
            .vpn_name(vpn.clone())
            .username(user.clone())
            .password(pass.clone())
            .build()
            .unwrap_or_else(|e| panic!("[{label}] session build: {e:?}"));
        let flow = session
            .create_flow_active(&queue, AckMode::Auto, 255, None, true)
            .unwrap_or_else(|e| panic!("[{label}] create_flow_active: {e:?}"));
        flow.start().unwrap();
        // Leak the session so the flow outlives this closure (the flow holds an Arc to the
        // session internals; we only need the flow handle for the probe's lifetime).
        std::mem::forget(session);
        flow
    };

    let mut fa = Some(mk_flow("A"));
    let mut fb = Some(mk_flow("B"));

    // Drain both event channels for 2s, recording Active/Inactive with timestamps.
    let mut log: Vec<(&'static str, FlowEvent, u128)> = Vec::new();
    let t_end = now_ms() + 2000;
    while now_ms() < t_end {
        if let Some(f) = fa.as_mut() {
            while let Ok(e) = f.try_recv_event() {
                eprintln!("[A] {e:?} @ {}", now_ms());
                log.push(("A", e, now_ms()));
            }
        }
        if let Some(f) = fb.as_mut() {
            while let Ok(e) = f.try_recv_event() {
                eprintln!("[B] {e:?} @ {}", now_ms());
                log.push(("B", e, now_ms()));
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let is_active = |who: &str, log: &[(&'static str, FlowEvent, u128)]| {
        log.iter()
            .rev()
            .find(|(w, _, _)| *w == who)
            .map(|(_, e, _)| matches!(e, FlowEvent::Active))
            .unwrap_or(false)
    };
    let a_act = is_active("A", &log);
    let b_act = is_active("B", &log);
    println!("after bind: A active={a_act}, B active={b_act} (expect exactly one)");
    if a_act == b_act {
        println!("FAIL: expected exactly one Active leader (active-flow indication not working on async path)");
        return;
    }
    let leader = if a_act { "A" } else { "B" };
    println!("leader = {leader}");

    // Drop the leader flow and measure promotion of the standby through the async channel.
    let t_drop = now_ms();
    if leader == "A" {
        fa.take();
    } else {
        fb.take();
    }
    println!("dropped leader {leader} @ {t_drop}");

    let standby = if leader == "A" { "B" } else { "A" };
    let mut promo: Option<u128> = None;
    let t_wait = now_ms() + 3000;
    while now_ms() < t_wait && promo.is_none() {
        let f = if standby == "A" {
            fa.as_mut()
        } else {
            fb.as_mut()
        };
        if let Some(f) = f {
            while let Ok(e) = f.try_recv_event() {
                eprintln!("[{standby}] {e:?} @ {}", now_ms());
                if matches!(e, FlowEvent::Active) {
                    promo = Some(now_ms());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    match promo {
        Some(t) => println!(
            "TAKEOVER: standby {standby} became Active {} ms after leader drop",
            t - t_drop
        ),
        None => println!("NO takeover: standby {standby} not promoted within 3s"),
    }
    println!(
        "async active-flow indication: {}",
        if promo.is_some() { "WORKS" } else { "FAILED" }
    );
}
