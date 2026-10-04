//! Reliable lifecycle ordering and compatibility, using bounded local traffic only.
use super::*;
use woven_protocol::{
    DeliveryClass as WireDelivery, EntityEntered, EntityLeaveReason, EntityLeft, Envelope,
};

fn lifecycle_envelope(entity_id: u64, entered: bool) -> Envelope {
    let control = if entered {
        ControlPayload::EntityEntered(EntityEntered {
            owner_entity_id: Some(entity_id),
        })
    } else {
        ControlPayload::EntityLeft(EntityLeft {
            reason: EntityLeaveReason::Disconnected,
        })
    };
    Envelope {
        namespace_id: 1,
        session_id: 1,
        space_id: 1,
        space_epoch: 1,
        entity_id: Some(entity_id),
        ..Envelope::control(WireDelivery::ReliableOrdered, control)
    }
}

#[test]
fn lifecycle_filter_checks_scope_epoch_class_and_nonzero_entity() {
    let config = WovenConfig::default();
    for entered in [false, true] {
        let envelope = lifecycle_envelope(7, entered);
        let expected = if entered {
            RealtimeEvent::EntityEntered { entity_id: 7 }
        } else {
            RealtimeEvent::EntityLeft { entity_id: 7 }
        };
        assert_eq!(entity_lifecycle_event(&config, &envelope), Some(expected));
        for field in 0..8 {
            let mut envelope = lifecycle_envelope(7, entered);
            match field {
                0 => envelope.namespace_id += 1,
                1 => envelope.session_id += 1,
                2 => envelope.space_id += 1,
                3 => envelope.space_epoch += 1,
                4 => envelope.delivery_class = WireDelivery::ReliableUnordered,
                5 => envelope.entity_id = None,
                6 => envelope.entity_id = Some(0),
                _ => {
                    envelope.message = MessagePayload::EntityState(woven_protocol::OpaquePayload {
                        type_id: APPLICATION_PAYLOAD_TYPE_ID,
                        bytes: vec![0xff],
                    });
                }
            }
            assert!(
                entity_lifecycle_event(&config, &envelope).is_none(),
                "field {field}"
            );
        }
    }
}

#[test]
fn bounded_lifecycle_drain_preserves_interleaved_entry_leave_order() {
    let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
    for (entity_id, entered) in [(7, true), (7, false), (8, true), (8, false)] {
        let event =
            entity_lifecycle_event(&adapter.config, &lifecycle_envelope(entity_id, entered))
                .unwrap();
        queue_lifecycle_event(&mut adapter.entity_lifecycle, event).unwrap();
    }
    assert!(adapter.drain_entity_lifecycle_bounded(0).is_empty());
    assert_eq!(
        adapter.drain_entity_lifecycle_bounded(1),
        [RealtimeEvent::EntityEntered { entity_id: 7 }]
    );
    assert!(adapter.has_pending_entity_lifecycle());
    assert_eq!(
        adapter.drain_entity_lifecycle_bounded(2),
        [
            RealtimeEvent::EntityLeft { entity_id: 7 },
            RealtimeEvent::EntityEntered { entity_id: 8 },
        ]
    );
    assert_eq!(
        adapter.drain_entity_lifecycle(),
        [RealtimeEvent::EntityLeft { entity_id: 8 }]
    );
    assert!(!adapter.has_pending_entity_lifecycle());
}

#[test]
fn leave_only_compatibility_consumes_entries_without_reordering_leaves() {
    let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
    for event in [
        RealtimeEvent::EntityEntered { entity_id: 7 },
        RealtimeEvent::EntityEntered { entity_id: 8 },
        RealtimeEvent::EntityLeft { entity_id: 8 },
        RealtimeEvent::EntityLeft { entity_id: 7 },
    ] {
        queue_lifecycle_event(&mut adapter.entity_lifecycle, event).unwrap();
    }
    assert_eq!(adapter.drain_entity_leaves(), [8, 7]);
    assert!(adapter.drain_entity_lifecycle().is_empty());
}

#[test]
fn lifecycle_overflow_is_explicit_and_stop_clears_pending_events() {
    let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
    for entity_id in 1..=MAX_PENDING_LIFECYCLE_EVENTS as u64 {
        queue_lifecycle_event(
            &mut adapter.entity_lifecycle,
            RealtimeEvent::EntityEntered { entity_id },
        )
        .unwrap();
    }
    let capacity = adapter.entity_lifecycle.capacity();
    assert!(
        matches!(queue_lifecycle_event(&mut adapter.entity_lifecycle,
            RealtimeEvent::EntityLeft { entity_id: 1 }), Err(WovenAdapterError::ClientFailed(reason))
            if reason == "Woven lifecycle event queue overflow"
        )
    );
    assert_eq!(adapter.entity_lifecycle.len(), MAX_PENDING_LIFECYCLE_EVENTS);
    assert_eq!(adapter.entity_lifecycle.capacity(), capacity);
    assert_eq!(
        adapter.entity_lifecycle.front(),
        Some(&RealtimeEvent::EntityEntered { entity_id: 1 })
    );
    adapter.stop();
    assert!(adapter.drain_entity_lifecycle().is_empty());
}

#[test]
fn loopback_entries_and_leaves_share_one_ordered_queue() {
    let server_runtime = Runtime::new().unwrap();
    let urls = server_runtime
        .block_on(woven_server::serve_dev_ephemeral(false))
        .unwrap();
    let config = WovenConfig {
        mode: ConnectivityMode::Loopback,
        endpoint: Some(urls.quic),
        ..WovenConfig::default()
    };
    let mut observer = WovenAdapter::new(config.clone()).unwrap();
    observer.start().unwrap();
    let mut expected = Vec::with_capacity(4);
    for _ in 0..2 {
        let mut peer = WovenAdapter::new(config.clone()).unwrap();
        peer.start().unwrap();
        let entity_id = peer.entity_id().unwrap();
        expected.push(RealtimeEvent::EntityEntered { entity_id });
        expected.push(RealtimeEvent::EntityLeft { entity_id });
        peer.stop();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while observer.entity_lifecycle.len() < expected.len()
            && std::time::Instant::now() < deadline
        {
            assert!(observer.drain_envelopes().unwrap().is_empty());
        }
        assert_eq!(observer.entity_lifecycle.len(), expected.len());
    }
    assert_eq!(observer.drain_entity_lifecycle_bounded(3), expected[..3]);
    assert_eq!(observer.drain_entity_lifecycle(), expected[3..]);
}
