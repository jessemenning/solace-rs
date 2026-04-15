use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    sync::{mpsc, Arc, Barrier, Mutex},
    thread::{self, sleep},
    time::Duration,
};

use solace_rs::{
    message::{
        DeliveryMode, DestinationType, InboundMessage, Message, MessageDestination,
        OutboundMessageBuilder,
    },
    session::SessionEvent,
    Context, SolaceLogLevel,
};

/// Short sleep used to ensure subscriptions are registered before publishing.
static SLEEP_TIME: std::time::Duration = Duration::from_millis(10);

/// Per-message receive timeout. Long enough for cloud broker round-trip latency.
static RECV_TIMEOUT: Duration = Duration::from_secs(10);

const DEFAULT_URL: &str = "tcp://localhost:55555";
const DEFAULT_VPN: &str = "default";
const DEFAULT_USERNAME: &str = "default";
const DEFAULT_PASSWORD: &str = "";

#[test]
#[ignore]
fn subscribe_and_publish() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");

    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let (tx, rx) = mpsc::channel();
    let tx_msgs = vec!["helo", "hello2", "hello4", "helo5"];
    let topic = "publish_and_receive";

    let on_message = move |message: InboundMessage| {
        let Ok(Some(payload)) = message.get_payload() else {
            return;
        };
        let _ = tx.send(payload.to_owned());
    };

    let mut builder = solace_context
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
    let session = builder.build().expect("creating session");

    session.subscribe(topic).expect("subscribing to topic");

    // need to wait before publishing so that the client is properly subscribed
    sleep(SLEEP_TIME);

    for msg in tx_msgs.clone() {
        let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
        let outbound_msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(msg)
            .build()
            .expect("building outbound msg");
        session.publish(outbound_msg).expect("publishing message");
    }

    let mut rx_msgs = vec![];
    loop {
        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(msg) => {
                let str = String::from_utf8_lossy(&msg).to_string();
                rx_msgs.push(str);
                if rx_msgs.len() == tx_msgs.len() {
                    break;
                }
            }
            Err(_) => panic!("timed out waiting for messages"),
        }
    }

    assert_eq!(tx_msgs, rx_msgs);
}

#[test]
#[ignore]
fn multi_subscribe_and_publish() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");
    let msg_multiplier = 2;

    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let (tx0, rx) = mpsc::channel();
    let tx1 = tx0.clone();
    let tx_msgs = vec!["helo", "hello2", "hello4", "helo5"];
    let topic = "multi_subscribe_and_publish";

    let mut builder0 = solace_context
        .session_builder()
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .on_message(move |message: InboundMessage| {
            let Ok(Some(payload)) = message.get_payload() else {
                return;
            };
            let _ = tx0.send(payload.to_owned());
        })
        .on_event(|_: SessionEvent| {});
    if let Some(dir) = trust_store_dir {
        builder0 = builder0.ssl_trust_store_dir(dir);
    }
    let session0 = builder0.build().expect("creating session");
    session0.subscribe(topic).expect("subscribing to topic");

    let mut builder1 = solace_context
        .session_builder()
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .on_message(move |message: InboundMessage| {
            let Ok(Some(payload)) = message.get_payload() else {
                return;
            };
            let _ = tx1.send(payload.to_owned());
        })
        .on_event(|_: SessionEvent| {});
    if let Some(dir) = trust_store_dir {
        builder1 = builder1.ssl_trust_store_dir(dir);
    }
    let session1 = builder1.build().expect("creating session");
    session1.subscribe(topic).expect("subscribing to topic");

    // need to wait before publishing so that the client is properly subscribed
    sleep(SLEEP_TIME);

    for msg in tx_msgs.clone() {
        let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
        let outbound_msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(msg)
            .build()
            .expect("building outbound msg");
        session0.publish(outbound_msg).expect("publishing message");
    }

    let mut rx_msgs = vec![];
    loop {
        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(msg) => {
                let str = String::from_utf8_lossy(&msg).to_string();
                rx_msgs.push(str);
                if rx_msgs.len() == tx_msgs.len() * msg_multiplier {
                    break;
                }
            }
            Err(_) => panic!("timed out waiting for messages"),
        }
    }

    let mut rx_msg_map = HashMap::new();
    for msg in rx_msgs {
        *rx_msg_map.entry(msg).or_insert(0) += 1;
    }

    assert_eq!(
        tx_msgs.clone().into_iter().collect::<HashSet<_>>(),
        rx_msg_map
            .keys()
            .map(|v| v.as_str())
            .collect::<HashSet<_>>()
    );

    assert_eq!(
        tx_msgs.iter().map(|_| msg_multiplier).collect::<Vec<_>>(),
        rx_msg_map.into_values().collect::<Vec<_>>()
    )
}

#[test]
#[ignore]
fn unsubscribe_and_publish() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");

    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let (tx, rx) = mpsc::channel();
    let tx_msgs = vec!["helo", "hello2", "hello4", "helo5"];
    let topic = "unsubscribe_and_publish";

    let on_message = move |message: InboundMessage| {
        let Ok(Some(payload)) = message.get_payload() else {
            return;
        };
        let _ = tx.send(payload.to_owned());
    };

    let mut builder = solace_context
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
    let session = builder.build().expect("creating session");

    session.subscribe(topic).expect("subscribing to topic");

    sleep(SLEEP_TIME);

    for msg in tx_msgs.clone() {
        let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
        let outbound_msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(msg)
            .build()
            .expect("building outbound msg");
        session.publish(outbound_msg).expect("publishing message");
    }

    // Receive messages published before unsubscribe
    let mut rx_msgs = vec![];
    loop {
        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(msg) => {
                let str = String::from_utf8_lossy(&msg).to_string();
                rx_msgs.push(str);
                if rx_msgs.len() == tx_msgs.len() {
                    break;
                }
            }
            Err(_) => panic!("timed out waiting for messages"),
        }
    }

    assert_eq!(tx_msgs, rx_msgs);

    session.unsubscribe(topic).expect("unsubscribing to topic");

    // Brief pause to ensure unsubscribe is processed
    sleep(SLEEP_TIME);

    for msg in tx_msgs.clone() {
        let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
        let outbound_msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(msg)
            .build()
            .expect("building outbound msg");
        session.publish(outbound_msg).expect("publishing message");
    }

    // Give time for any stray messages to arrive, then verify none did
    sleep(SLEEP_TIME);

    if rx.try_recv().is_ok() {
        panic!("received message after unsubscribe")
    }
}

#[test]
#[ignore]
fn multi_thread_publisher() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");

    let msg_multiplier = 3;

    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let (tx, rx) = mpsc::channel();
    let tx_msgs = vec!["helo", "hello2", "hello4", "helo5"];
    let topic = "multi_thread_publisher";

    let on_message = move |message: InboundMessage| {
        let Ok(Some(payload)) = message.get_payload() else {
            return;
        };
        let _ = tx.send(payload.to_owned());
    };

    let mut builder = solace_context
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
    let session = Arc::new(Mutex::new(builder.build().expect("creating session")));

    session
        .lock()
        .unwrap()
        .subscribe(topic)
        .expect("multi_thread_publisher");

    // need to wait before publishing so that the client is properly subscribed
    sleep(SLEEP_TIME);

    let mut handles = vec![];

    for _ in 0..msg_multiplier {
        let session_clone = session.clone();
        let tx_msgs_clone = tx_msgs.clone();
        let thread_h = std::thread::spawn(move || {
            let session_clone_lock = session_clone.lock().unwrap();
            for msg in &tx_msgs_clone {
                let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
                let outbound_msg = OutboundMessageBuilder::new()
                    .destination(dest)
                    .delivery_mode(DeliveryMode::Direct)
                    .payload(*msg)
                    .build()
                    .expect("building outbound msg");
                session_clone_lock
                    .publish(outbound_msg)
                    .expect("publishing message");
            }
            drop(session_clone_lock);
        });
        handles.push(thread_h);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Collect all expected messages before dropping the session
    let expected = tx_msgs.len() * msg_multiplier;
    let mut rx_msgs = vec![];
    loop {
        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(msg) => {
                let str = String::from_utf8_lossy(&msg).to_string();
                rx_msgs.push(str);
                if rx_msgs.len() == expected {
                    break;
                }
            }
            Err(_) => panic!(
                "timed out: received {}/{} messages",
                rx_msgs.len(),
                expected
            ),
        }
    }

    drop(session);
    drop(solace_context);

    assert!(rx_msgs.len() == tx_msgs.len() * msg_multiplier);

    let mut rx_msg_map = HashMap::new();
    for msg in rx_msgs {
        *rx_msg_map.entry(msg).or_insert(0) += 1;
    }

    assert_eq!(
        tx_msgs.clone().into_iter().collect::<HashSet<_>>(),
        rx_msg_map
            .keys()
            .map(|v| v.as_str())
            .collect::<HashSet<_>>()
    );

    assert_eq!(
        tx_msgs.iter().map(|_| msg_multiplier).collect::<Vec<_>>(),
        rx_msg_map.into_values().collect::<Vec<_>>()
    )
}

#[test]
#[ignore]
fn no_local_session() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");

    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let (tx, rx) = mpsc::channel();
    let tx_msgs = vec!["helo", "hello2", "hello4", "helo5"];
    let topic = "no_local_session";

    let on_message = move |message: InboundMessage| {
        let _ = tx.send(message);
    };

    let mut builder = solace_context
        .session_builder()
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .on_message(on_message)
        .on_event(|_: SessionEvent| {})
        .no_local(true);
    if let Some(dir) = trust_store_dir {
        builder = builder.ssl_trust_store_dir(dir);
    }
    let session = builder.build().expect("creating session");

    session.subscribe(topic).expect("subscribing to topic");

    // need to wait before publishing so that the client is properly subscribed
    sleep(SLEEP_TIME);

    for msg in tx_msgs.clone() {
        let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
        let outbound_msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(msg)
            .build()
            .expect("building outbound msg");
        session.publish(outbound_msg).expect("publishing message");
    }
    sleep(SLEEP_TIME * 2);

    assert!(rx.try_recv().is_err());
}

#[test]
#[ignore]
fn auto_generate_tx_rx_session_fields() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");

    let (tx, rx) = mpsc::channel();

    let tx_msgs = vec!["helo", "hello2", "hello4", "helo5"];
    let topic = "auto_generate_tx_rx_session_fields";
    let send_count = 1000;
    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let on_message = move |message: InboundMessage| {
        let _ = tx.send(message);
    };

    let mut builder = solace_context
        .session_builder()
        .host_name(url)
        .vpn_name(vpn)
        .username(username)
        .password(password)
        .on_message(on_message)
        .on_event(|_: SessionEvent| {})
        // NOTE: there is bug in the solace lib where it does not copy over the message if there is
        // not enough space in the buffer. This can cause the TSan to trigger.
        .buffer_size_bytes(900_000)
        .generate_rcv_timestamps(true)
        .generate_sender_id(true)
        .generate_send_timestamp(true)
        .generate_sender_sequence_number(true);
    if let Some(dir) = trust_store_dir {
        builder = builder.ssl_trust_store_dir(dir);
    }
    let session = builder.build().expect("creating session");

    session.subscribe(topic).expect("subscribing to topic");

    // need to wait before publishing so that the client is properly subscribed
    sleep(SLEEP_TIME);

    for msg in tx_msgs.clone().into_iter().cycle().take(send_count) {
        let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();
        let outbound_msg = OutboundMessageBuilder::new()
            .destination(dest)
            .delivery_mode(DeliveryMode::Direct)
            .payload(msg)
            .build()
            .expect("building outbound msg");
        session.publish(outbound_msg).expect("publishing message");
    }

    // Collect all expected messages before disconnecting to avoid race with drop
    let mut iter = tx_msgs.clone().into_iter().cycle();
    let mut rx_count = 0;
    loop {
        match rx.recv_timeout(RECV_TIMEOUT) {
            Ok(msg) => {
                assert!(msg.get_payload().unwrap().unwrap() == iter.next().unwrap().as_bytes());
                assert!(msg.get_receive_timestamp().is_ok_and(|v| v.is_some()));
                assert!(msg.get_sender_id().is_ok_and(|v| v.is_some()));
                assert!(msg.get_sender_timestamp().is_ok_and(|v| v.is_some()));
                assert!(msg.get_sequence_number().is_ok_and(|v| v.is_some()));
                rx_count += 1;
                if rx_count == send_count {
                    break;
                }
            }
            Err(_) => panic!("timed out: received {}/{} messages", rx_count, send_count),
        }
    }

    let _ = session.disconnect();
    drop(solace_context);

    assert!(rx_count == send_count);
}

#[test]
#[ignore]
fn request_and_reply() {
    let url = option_env!("SOLACE_BROKER_URL").unwrap_or(DEFAULT_URL);
    let vpn = option_env!("SOLACE_BROKER_VPN").unwrap_or(DEFAULT_VPN);
    let username = option_env!("SOLACE_BROKER_USERNAME").unwrap_or(DEFAULT_USERNAME);
    let password = option_env!("SOLACE_BROKER_PASSWORD").unwrap_or(DEFAULT_PASSWORD);
    let trust_store_dir = option_env!("SOLACE_BROKER_TRUST_STORE_DIR");
    let topic = "request_and_reply";

    let solace_context = Context::new(SolaceLogLevel::Warning).unwrap();
    let g_barrier = Arc::new(Barrier::new(2));

    thread::scope(|s| {
        let context = solace_context.clone();
        let barrier = g_barrier.clone();
        // requester
        let req = s.spawn(move || {
            let mut builder = context
                .session_builder()
                .host_name(url)
                .vpn_name(vpn)
                .username(username)
                .password(password)
                .on_message(|_: InboundMessage| {})
                .on_event(|_: SessionEvent| {});
            if let Some(dir) = trust_store_dir {
                builder = builder.ssl_trust_store_dir(dir);
            }
            let session = builder.build().unwrap();

            barrier.wait();
            sleep(SLEEP_TIME);

            let dest = MessageDestination::new(DestinationType::Topic, topic).unwrap();

            let request = OutboundMessageBuilder::new()
                .destination(dest)
                .delivery_mode(DeliveryMode::Direct)
                .payload("ping".to_string())
                .build()
                .expect("could not build message");
            let reply = session
                .request(request, NonZeroU32::new(5_000).unwrap())
                .unwrap();
            assert!(reply.get_payload().unwrap().unwrap() == b"pong");
        });

        let context = solace_context.clone();
        let res = s.spawn(move || {
            let (tx, rx) = mpsc::channel();
            let mut builder = context
                .session_builder()
                .host_name(url)
                .vpn_name(vpn)
                .username(username)
                .password(password)
                .on_message(move |message: InboundMessage| {
                    let _ = tx.send(message);
                })
                .on_event(|_: SessionEvent| {});
            if let Some(dir) = trust_store_dir {
                builder = builder.ssl_trust_store_dir(dir);
            }
            let session = builder.build().unwrap();

            session.subscribe(topic).unwrap();

            g_barrier.wait();

            let msg = rx.recv().unwrap();

            let reply_msg = OutboundMessageBuilder::new()
                .destination(msg.get_reply_to().unwrap().unwrap())
                .delivery_mode(DeliveryMode::Direct)
                .payload("pong".to_string())
                .is_reply(true)
                .correlation_id(msg.get_correlation_id().unwrap().unwrap())
                .build()
                .expect("could not build message");
            let _ = session.publish(reply_msg);
        });
        assert!(res.join().is_ok());
        assert!(req.join().is_ok());
    });
}
