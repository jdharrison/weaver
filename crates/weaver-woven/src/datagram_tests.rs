//! Real QUIC traffic on ephemeral local nodes; no shared or hosted targets.
use super::*;
use woven_protocol::{DeliveryClass as WireDelivery, Envelope, OpaquePayload};

pub(super) fn assert_byte_and_json_roundtrip(
    adapter: &mut WovenAdapter,
    reliable_sequence: u64,
    datagram_sequence: u64,
) {
    let entity_id = adapter.entity_id().unwrap();
    let bytes = vec![0xff, 0, 0x80, b'{', 0xfe, b'}'];
    let body = serde_json::json!({"chat": "reliable JSON alongside binary state"});
    adapter
        .publish_unreliable_payload(None, datagram_sequence, bytes.clone())
        .unwrap();
    adapter
        .publish(
            1,
            None,
            &Payload {
                body: body.clone(),
                sequence: reliable_sequence,
                revision: 0,
            },
            DeliveryClass::ReliableOrdered,
            PersistenceClass::Ephemeral,
        )
        .unwrap();

    let mut saw_bytes = false;
    let mut saw_json = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline && !(saw_bytes && saw_json) {
        for payload in adapter.drain_unreliable_bounded(1).unwrap() {
            assert_eq!(payload.entity_id, entity_id);
            assert_eq!(payload.sequence, datagram_sequence);
            assert_eq!(payload.payload, bytes);
            saw_bytes = true;
        }
        for envelope in adapter.drain_envelopes_bounded(1).unwrap() {
            assert_eq!(envelope.channel, 1);
            assert_eq!(envelope.entity, Some(entity_id));
            assert_eq!(envelope.sequence, reliable_sequence);
            assert_eq!(envelope.delivery, DeliveryClass::ReliableOrdered);
            assert_eq!(envelope.persistence, PersistenceClass::Ephemeral);
            assert_eq!(
                envelope.to_payload::<serde_json::Value>().unwrap().body,
                body
            );
            saw_json = true;
        }
    }
    assert!(
        saw_bytes,
        "binary state must arrive through the separate datagram receiver"
    );
    assert!(
        saw_json,
        "reliable JSON must remain readable on the control stream"
    );
}

#[test]
fn embedded_binary_datagrams_and_reliable_json_coexist_and_restart() {
    let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
    assert!(matches!(
        adapter.drain_unreliable(),
        Err(WovenAdapterError::NotRunning)
    ));
    adapter.start().unwrap();
    assert!(adapter.datagram_receiver.is_some());
    assert_byte_and_json_roundtrip(&mut adapter, 1, 101);
    assert!(matches!(
        adapter.publish_unreliable_payload(None, 101, vec![0xff]),
        Err(WovenAdapterError::StalePayload)
    ));
    // A failed MTU check must not consume the sequence or fall back to the stream.
    assert!(
        adapter
            .publish_unreliable_payload(None, 102, vec![0; 65_536])
            .is_err()
    );
    assert_eq!(adapter.next_sequence.get(&UNRELIABLE_CHANNEL), Some(&101));
    assert_byte_and_json_roundtrip(&mut adapter, 2, 102);
    adapter.stop();
    assert!(adapter.datagram_receiver.is_none());
    assert!(adapter.next_sequence.is_empty());
    assert!(matches!(
        adapter.drain_unreliable(),
        Err(WovenAdapterError::NotRunning)
    ));
    adapter.start().unwrap();
    assert_byte_and_json_roundtrip(&mut adapter, 1, 1);
}

#[test]
fn loopback_datagrams_reach_a_distinct_client_and_leave_stays_reliable() {
    let server_runtime = Runtime::new().unwrap();
    let urls = server_runtime
        .block_on(woven_server::serve_dev_ephemeral(false))
        .unwrap();
    let config = WovenConfig {
        mode: ConnectivityMode::Loopback,
        endpoint: Some(urls.quic),
        ..WovenConfig::default()
    };
    let mut sender = WovenAdapter::new(config.clone()).unwrap();
    let mut observer = WovenAdapter::new(config).unwrap();
    sender.start().unwrap();
    observer.start().unwrap();
    assert_byte_and_json_roundtrip(&mut sender, 1, 7);
    let entity_id = sender.entity_id().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut saw_bytes = false;
    let mut saw_json = false;
    while std::time::Instant::now() < deadline && !(saw_bytes && saw_json) {
        saw_bytes |= observer.drain_unreliable().unwrap().iter().any(|payload| {
            payload.entity_id == entity_id && payload.payload == [0xff, 0, 0x80, b'{', 0xfe, b'}']
        });
        saw_json |= observer
            .drain_envelopes()
            .unwrap()
            .iter()
            .any(|payload| payload.entity == Some(entity_id) && payload.channel == 1);
    }
    assert!(saw_bytes && saw_json);
    sender.stop();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut saw_leave = false;
    while std::time::Instant::now() < deadline && !saw_leave {
        assert!(observer.drain_unreliable().unwrap().is_empty());
        observer.drain_envelopes().unwrap();
        saw_leave |= observer.drain_entity_leaves().contains(&entity_id);
    }
    assert!(
        saw_leave,
        "EntityLeft must remain a reliable lifecycle event"
    );
}

fn datagram() -> Envelope {
    Envelope {
        delivery_class: WireDelivery::UnreliableSequenced,
        namespace_id: 11,
        session_id: 17,
        space_id: 3,
        channel_id: Some(UNRELIABLE_CHANNEL),
        entity_id: Some(29),
        space_epoch: 5,
        server_tick: 0,
        sender_sequence: 7,
        correlation_id: None,
        ..Envelope::entity_state(
            WireDelivery::UnreliableSequenced,
            OpaquePayload {
                type_id: APPLICATION_PAYLOAD_TYPE_ID,
                bytes: vec![0xff, 0, 0x80],
            },
        )
    }
}

#[test]
fn unreliable_filter_checks_actual_scope_class_channel_type_and_entity() {
    let config = WovenConfig {
        namespace_id: 11,
        session_id: 17,
        space_id: 3,
        space_epoch: 5,
        ..WovenConfig::default()
    };
    let payload = unreliable_payload(&config, datagram()).unwrap();
    assert_eq!(payload.payload, [0xff, 0, 0x80]);
    assert_eq!(payload.entity_id, 29);
    assert_eq!(payload.sequence, 7);
    for field in 0..10 {
        let mut envelope = datagram();
        match field {
            0 => envelope.namespace_id += 1,
            1 => envelope.session_id += 1,
            2 => envelope.space_id += 1,
            3 => envelope.space_epoch += 1,
            4 => envelope.channel_id = Some(1),
            5 => envelope.delivery_class = WireDelivery::LatestValue,
            6 => envelope.entity_id = None,
            7 => envelope.entity_id = Some(0),
            8 => {
                let MessagePayload::EntityState(payload) = &mut envelope.message else {
                    unreachable!()
                };
                payload.type_id += 1;
            }
            _ => {
                envelope.message = MessagePayload::ReliableEvent(OpaquePayload {
                    type_id: 1,
                    bytes: vec![0xff],
                });
            }
        }
        assert!(
            unreliable_payload(&config, envelope).is_none(),
            "field {field}"
        );
    }
}
