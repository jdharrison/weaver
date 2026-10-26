use super::*;

const INTERVAL: Duration = Duration::from_millis(40);

fn reliable(sequence: u64) -> WorkerCommand {
    WorkerCommand::PublishReliable {
        sequence,
        payload: sequence.to_string(),
    }
}

fn pose(sequence: u64) -> WorkerCommand {
    WorkerCommand::PublishPositionedUnreliable {
        sequence,
        position: [sequence as f64, 0.0, 0.0],
        payload: sequence.to_le_bytes().to_vec(),
    }
}

fn sequence(command: &WorkerCommand) -> u64 {
    match command {
        WorkerCommand::PublishReliable { sequence, .. }
        | WorkerCommand::PublishLatest { sequence, .. }
        | WorkerCommand::PublishUnreliable { sequence, .. }
        | WorkerCommand::PublishPositionedUnreliable { sequence, .. } => *sequence,
        _ => panic!("not a publication"),
    }
}

fn send(pacing: &mut PublishPacing, now: Instant) -> u64 {
    let command = pacing.take_ready(now).expect("publication ready");
    pacing.completed(now);
    sequence(&command)
}

#[test]
fn mixed_backlog_coalesces_pose_preserves_events_and_never_catches_up() {
    let mut pacing = PublishPacing::new(INTERVAL);
    // Model commands queued during a worker stall, including reliable events
    // between poses: only replaceable state may disappear.
    for command in [pose(10), reliable(1), pose(11), reliable(2), pose(12)] {
        pacing.enqueue(command).unwrap();
    }
    let start = Instant::now();
    assert_eq!(send(&mut pacing, start), 1);
    assert!(
        pacing
            .take_ready(start + INTERVAL.checked_sub(Duration::from_nanos(1)).unwrap())
            .is_none()
    );
    let stalled = start + Duration::from_secs(10);
    assert_eq!(send(&mut pacing, stalled), 12);
    assert!(pacing.take_ready(stalled).is_none());
    assert!(
        pacing
            .take_ready(stalled + INTERVAL.checked_sub(Duration::from_nanos(1)).unwrap())
            .is_none()
    );
    assert_eq!(send(&mut pacing, stalled + INTERVAL), 2);
    assert!(pacing.take_ready(stalled + INTERVAL * 2).is_none());
}

#[test]
fn slow_publish_uses_completion_not_dispatch_time() {
    let mut pacing = PublishPacing::new(INTERVAL);
    pacing.enqueue(reliable(1)).unwrap();
    pacing.enqueue(pose(2)).unwrap();
    let start = Instant::now();
    assert_eq!(sequence(&pacing.take_ready(start).unwrap()), 1);
    let completed = start + Duration::from_secs(3);
    pacing.completed(completed);
    assert!(pacing.take_ready(completed).is_none());
    assert!(
        pacing
            .take_ready(completed + Duration::from_millis(39))
            .is_none()
    );
    assert_eq!(send(&mut pacing, completed + INTERVAL), 2);
}

#[test]
fn continuously_pending_classes_alternate_and_reliable_is_fifo() {
    let mut pacing = PublishPacing::new(INTERVAL);
    for n in 0..20 {
        pacing.enqueue(reliable(n)).unwrap();
    }
    let start = Instant::now();
    for tick in 0..40 {
        pacing.enqueue(pose(100 + u64::from(tick))).unwrap();
        let command = pacing.take_ready(start + INTERVAL * tick).unwrap();
        if tick % 2 == 0 {
            assert!(matches!(command, WorkerCommand::PublishReliable { .. }));
            assert_eq!(sequence(&command), u64::from(tick / 2));
        } else {
            assert!(matches!(
                command,
                WorkerCommand::PublishPositionedUnreliable { .. }
            ));
            assert_eq!(sequence(&command), 100 + u64::from(tick));
        }
        pacing.completed(start + INTERVAL * tick);
    }
    assert!(pacing.reliable.is_empty());
}

#[test]
fn reliable_overflow_is_explicit_and_does_not_overwrite_accepted_events() {
    let mut pacing = PublishPacing::new(INTERVAL);
    for n in 0..CHANNEL_CAPACITY {
        pacing.enqueue(reliable(n as u64)).unwrap();
    }
    assert_eq!(
        pacing.enqueue(reliable(999)),
        Err("Woven realtime paced reliable queue overflow")
    );
    for n in 0..1000 {
        pacing.enqueue(pose(n)).unwrap();
    }
    assert_eq!(pacing.reliable.len(), CHANNEL_CAPACITY);
    let start = Instant::now();
    assert_eq!(send(&mut pacing, start), 0);
    assert_eq!(send(&mut pacing, start + INTERVAL), 999);
    for n in 1..CHANNEL_CAPACITY {
        assert_eq!(
            send(&mut pacing, start + INTERVAL * (n as u32 + 1)),
            n as u64
        );
    }
    assert!(pacing.reliable.is_empty());
    assert!(pacing.pose.is_none());
}

#[test]
fn legacy_latest_respects_shared_channel_order_while_coalescing() {
    let mut pacing = PublishPacing::new(INTERVAL);
    pacing.enqueue(reliable(1)).unwrap();
    pacing
        .enqueue(WorkerCommand::PublishLatest {
            sequence: 2,
            payload: "old".into(),
        })
        .unwrap();
    pacing.enqueue(reliable(3)).unwrap();
    pacing
        .enqueue(WorkerCommand::PublishLatest {
            sequence: 4,
            payload: "new".into(),
        })
        .unwrap();
    pacing.enqueue(reliable(5)).unwrap();
    let start = Instant::now();
    for (tick, expected) in [1, 3, 4, 5].into_iter().enumerate() {
        assert_eq!(send(&mut pacing, start + INTERVAL * tick as u32), expected);
    }
}

#[test]
fn all_pose_variants_share_one_slot_and_one_publication_budget() {
    let mut pacing = PublishPacing::new(INTERVAL);
    let start = Instant::now();
    pacing.enqueue(pose(1)).unwrap();
    pacing
        .enqueue(WorkerCommand::PublishLatest {
            sequence: 2,
            payload: "state".into(),
        })
        .unwrap();
    pacing
        .enqueue(WorkerCommand::PublishUnreliable {
            sequence: 3,
            payload: vec![3],
        })
        .unwrap();
    assert_eq!(send(&mut pacing, start), 3);
    pacing.enqueue(pose(4)).unwrap();
    // Polling more often is harmless and never grants extra publication credit.
    for millis in 0..40 {
        assert!(
            pacing
                .take_ready(start + Duration::from_millis(millis))
                .is_none()
        );
    }
    assert_eq!(send(&mut pacing, start + INTERVAL), 4);
    let idle = start + Duration::from_secs(100);
    assert!(pacing.take_ready(idle).is_none());
    pacing.enqueue(reliable(5)).unwrap();
    pacing.enqueue(reliable(6)).unwrap();
    assert_eq!(send(&mut pacing, idle), 5);
    assert!(pacing.take_ready(idle).is_none());
    assert_eq!(send(&mut pacing, idle + INTERVAL), 6);
}

#[test]
fn forty_millisecond_spacing_allows_only_twenty_six_sends_at_inclusive_edges() {
    let mut pacing = PublishPacing::new(INTERVAL);
    let start = Instant::now();
    let mut sent = 0;
    for millis in 0..=1000 {
        pacing.enqueue(pose(millis)).unwrap();
        let now = start + Duration::from_millis(millis);
        if pacing.take_ready(now).is_some() {
            pacing.completed(now);
            sent += 1;
        }
    }
    assert_eq!(sent, 26);
}
