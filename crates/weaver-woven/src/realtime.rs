//! `weaver-app-core` realtime driver backed by a dedicated Woven worker.

use crate::{
    DeliveryClass, PayloadEnvelope, PersistenceClass, WovenAdapter, WovenAdapterError, WovenConfig,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::thread::JoinHandle;
use weaver_app_core::{RealtimeCommand, RealtimeDriver, RealtimeEvent};

const APPLICATION_CHANNEL: u64 = 1;
const CHANNEL_CAPACITY: usize = 128;
const MAX_ITERATION_WORK: usize = 128;
const MAX_OUTBOUND_COMMANDS: usize = MAX_ITERATION_WORK / 2;
const MAX_POLL_EVENTS: usize = 128;

#[derive(Debug)]
enum WorkerCommand {
    PublishLatest {
        sequence: u64,
        payload: String,
    },
    PublishReliable {
        sequence: u64,
        payload: String,
    },
    PublishUnreliable {
        sequence: u64,
        payload: Vec<u8>,
    },
    PublishPositionedUnreliable {
        sequence: u64,
        position: [f64; 3],
        payload: Vec<u8>,
    },
    LogInfo {
        message: String,
    },
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForwardError {
    Full,
    Disconnected,
}

/// A bounded `weaver-app-core` realtime driver using Woven's public protocol.
///
/// The Woven adapter and all network operations live on one dedicated worker
/// thread. [`RealtimeDriver::poll`] only performs bounded nonblocking channel
/// operations.
pub struct WovenRealtimeDriver {
    entity_id: u64,
    command_tx: SyncSender<WorkerCommand>,
    pending_pose: Option<WorkerCommand>,
    event_rx: Receiver<RealtimeEvent>,
    terminal_rx: Receiver<RealtimeEvent>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    connected_emitted: bool,
    disconnected: bool,
}

impl WovenRealtimeDriver {
    /// Connect to Woven and move the connected adapter to a dedicated worker thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter cannot be constructed, connected, assigned
    /// an entity, or moved to its worker thread.
    pub fn connect(config: WovenConfig) -> Result<Self, WovenAdapterError> {
        let mut adapter = WovenAdapter::new(config)?;
        adapter.start()?;
        if !adapter.supports_positioned_state() {
            adapter.stop();
            return Err(WovenAdapterError::InitializationFailed(
                "Woven server did not negotiate positioned entity state".to_owned(),
            ));
        }
        let entity_id = adapter.entity_id().ok_or_else(|| {
            WovenAdapterError::UnexpectedMessage(
                "Woven subscription completed without an assigned entity".to_owned(),
            )
        })?;

        let (command_tx, command_rx) = sync_channel(CHANNEL_CAPACITY);
        let (event_tx, event_rx) = sync_channel(CHANNEL_CAPACITY);
        let (terminal_tx, terminal_rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("weaver-woven-realtime".to_owned())
            .spawn(move || {
                run_worker(
                    adapter,
                    entity_id,
                    &command_rx,
                    &event_tx,
                    &terminal_tx,
                    worker_stop.as_ref(),
                );
            })
            .map_err(|error| WovenAdapterError::InitializationFailed(error.to_string()))?;

        Ok(Self {
            entity_id,
            command_tx,
            pending_pose: None,
            event_rx,
            terminal_rx,
            stop,
            worker: Some(worker),
            connected_emitted: false,
            disconnected: false,
        })
    }

    /// Return the entity assigned to this connection by Woven.
    #[must_use]
    pub const fn entity_id(&self) -> u64 {
        self.entity_id
    }

    /// Enqueue one info log without blocking the render thread or retrying.
    ///
    /// Success means queued, not delivered or persisted in Host Logs. The SDK's
    /// message byte limit bounds each queue entry; logging failures do not disconnect.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid message, a full queue, or an unavailable worker.
    pub fn log_info(&self, message: &str) -> Result<(), WovenAdapterError> {
        if self.disconnected || self.stop.load(Ordering::Acquire) {
            return Err(WovenAdapterError::NotRunning);
        }
        if message.is_empty() || message.len() > woven_protocol::MAX_LOG_MESSAGE_BYTES {
            return Err(WovenAdapterError::ClientFailed(
                "client log message must contain 1 to 1024 UTF-8 bytes".to_owned(),
            ));
        }
        self.command_tx
            .try_send(WorkerCommand::LogInfo {
                message: message.to_owned(),
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => {
                    WovenAdapterError::ClientFailed("client log command queue is full".to_owned())
                }
                TrySendError::Disconnected(_) => WovenAdapterError::NotRunning,
            })
    }

    // One overwriteable slot absorbs replaceable state when the fixed worker queue is full.
    // Flush it before reliable JSON so an older channel-1 pose cannot follow newer chat.
    fn flush_pending_pose(&mut self) -> Result<bool, &'static str> {
        let Some(command) = self.pending_pose.take() else {
            return Ok(true);
        };
        match self.command_tx.try_send(command) {
            Ok(()) => Ok(true),
            Err(TrySendError::Full(command)) => {
                self.pending_pose = Some(command);
                Ok(false)
            }
            Err(TrySendError::Disconnected(_)) => Err("Woven realtime worker is unavailable"),
        }
    }

    fn signal_stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.try_send(WorkerCommand::Stop);
    }

    fn fail_closed(&mut self, reason: String, events: &mut Vec<RealtimeEvent>) {
        if self.disconnected {
            return;
        }
        self.signal_stop();
        self.pending_pose = None;
        self.disconnected = true;
        events.push(RealtimeEvent::Disconnected { reason });
    }

    fn receive_terminal(&mut self, events: &mut Vec<RealtimeEvent>) -> bool {
        match self.terminal_rx.try_recv() {
            Ok(RealtimeEvent::Disconnected { reason }) => {
                self.fail_closed(reason, events);
                true
            }
            Ok(_) => {
                self.fail_closed(
                    "Woven realtime worker returned an invalid terminal event".to_owned(),
                    events,
                );
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.fail_closed(
                    "Woven realtime worker stopped without a disconnect reason".to_owned(),
                    events,
                );
                true
            }
        }
    }
}

impl RealtimeDriver for WovenRealtimeDriver {
    fn poll(&mut self, commands: &[RealtimeCommand], events: &mut Vec<RealtimeEvent>) {
        if self.disconnected {
            return;
        }

        let mut emitted = 0;
        if !self.connected_emitted {
            events.push(RealtimeEvent::Connected {
                entity_id: self.entity_id,
            });
            self.connected_emitted = true;
            emitted += 1;
        }

        if self.receive_terminal(events) {
            return;
        }
        if commands.len() > MAX_ITERATION_WORK {
            self.fail_closed(
                format!(
                    "Woven realtime poll command limit exceeded: {} > {MAX_ITERATION_WORK}",
                    commands.len()
                ),
                events,
            );
            return;
        }

        for command in commands {
            let worker_command = match command {
                RealtimeCommand::PublishLatest { sequence, payload } => {
                    self.pending_pose = Some(WorkerCommand::PublishLatest {
                        sequence: *sequence,
                        payload: payload.clone(),
                    });
                    continue;
                }
                RealtimeCommand::PublishUnreliable { sequence, payload } => {
                    self.pending_pose = Some(WorkerCommand::PublishUnreliable {
                        sequence: *sequence,
                        payload: payload.clone(),
                    });
                    continue;
                }
                RealtimeCommand::PublishPositionedUnreliable {
                    sequence,
                    position,
                    payload,
                } => {
                    self.pending_pose = Some(WorkerCommand::PublishPositionedUnreliable {
                        sequence: *sequence,
                        position: *position,
                        payload: payload.clone(),
                    });
                    continue;
                }
                RealtimeCommand::PublishReliable { sequence, payload } => {
                    WorkerCommand::PublishReliable {
                        sequence: *sequence,
                        payload: payload.clone(),
                    }
                }
            };
            match self.flush_pending_pose() {
                Ok(true) => {}
                Ok(false) => {
                    self.fail_closed("Woven realtime command queue overflow".to_owned(), events);
                    return;
                }
                Err(reason) => {
                    self.fail_closed(reason.to_owned(), events);
                    return;
                }
            }
            match self.command_tx.try_send(worker_command) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.fail_closed("Woven realtime command queue overflow".to_owned(), events);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.fail_closed("Woven realtime worker is unavailable".to_owned(), events);
                    return;
                }
            }
        }
        if let Err(reason) = self.flush_pending_pose() {
            self.fail_closed(reason.to_owned(), events);
            return;
        }

        if self.receive_terminal(events) {
            return;
        }

        let event_budget =
            (MAX_ITERATION_WORK - commands.len()).min(MAX_POLL_EVENTS.saturating_sub(emitted + 1));
        for _ in 0..event_budget {
            match self.event_rx.try_recv() {
                Ok(event) => {
                    events.push(event);
                    emitted += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.receive_terminal(events) {
                        self.fail_closed(
                            "Woven realtime event channel disconnected".to_owned(),
                            events,
                        );
                    }
                    return;
                }
            }
        }

        if emitted < MAX_POLL_EVENTS {
            let _ = self.receive_terminal(events);
        }
    }
}

impl Drop for WovenRealtimeDriver {
    fn drop(&mut self) {
        self.signal_stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    mut adapter: WovenAdapter,
    entity_id: u64,
    command_rx: &Receiver<WorkerCommand>,
    event_tx: &SyncSender<RealtimeEvent>,
    terminal_tx: &SyncSender<RealtimeEvent>,
    stop: &AtomicBool,
) {
    let failure = worker_loop(&mut adapter, entity_id, command_rx, event_tx, stop);
    stop.store(true, Ordering::Release);
    if let Some(reason) = failure {
        let _ = terminal_tx.try_send(RealtimeEvent::Disconnected { reason });
    }
    adapter.stop();
}

fn worker_loop(
    adapter: &mut WovenAdapter,
    entity_id: u64,
    command_rx: &Receiver<WorkerCommand>,
    event_tx: &SyncSender<RealtimeEvent>,
    stop: &AtomicBool,
) -> Option<String> {
    loop {
        if stop.load(Ordering::Acquire) {
            return None;
        }

        let mut work = 0;
        let mut pending_pose = None;
        // Reserve inbound work even under a continuous pose/command stream.
        while work < MAX_OUTBOUND_COMMANDS {
            if stop.load(Ordering::Acquire) {
                return None;
            }
            let command = match command_rx.try_recv() {
                Ok(command) => command,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return None,
            };
            match command {
                command @ (WorkerCommand::PublishLatest { .. }
                | WorkerCommand::PublishUnreliable { .. }
                | WorkerCommand::PublishPositionedUnreliable { .. }) => {
                    pending_pose = Some(command);
                }
                command @ WorkerCommand::PublishReliable { .. } => {
                    if let Some(pose) = pending_pose.take()
                        && let Err(error) = publish_command(adapter, entity_id, pose)
                    {
                        return Some(error.to_string());
                    }
                    if let Err(error) = publish_command(adapter, entity_id, command) {
                        return Some(error.to_string());
                    }
                }
                WorkerCommand::LogInfo { message } => {
                    if adapter.log_info(&message).is_err() {
                        // Never expose the message or peer-supplied error text in diagnostics.
                        tracing::warn!("Woven client info log failed; not retrying");
                    }
                }
                WorkerCommand::Stop => return None,
            }
            work += 1;
        }

        if stop.load(Ordering::Acquire) {
            return None;
        }
        if let Some(pose) = pending_pose
            && let Err(error) = publish_command(adapter, entity_id, pose)
        {
            return Some(error.to_string());
        }

        let inbound_budget = MAX_ITERATION_WORK - work;
        let stream_budget = inbound_budget / 2;
        let datagram_budget = inbound_budget - stream_budget;
        // Never select a datagram read against the client's partial framed stream read.
        let envelopes = match adapter.drain_envelopes_bounded(stream_budget) {
            Ok(envelopes) => envelopes,
            Err(error) => return Some(error.to_string()),
        };
        let lifecycle_budget = stream_budget.saturating_sub(envelopes.len());
        // Keep entry/leave order and apply authorization changes before application traffic.
        for event in adapter.drain_entity_lifecycle_bounded(lifecycle_budget) {
            match forward_event(event_tx, event) {
                Ok(()) => {}
                Err(ForwardError::Full) => {
                    return Some("Woven realtime event queue overflow".to_owned());
                }
                Err(ForwardError::Disconnected) => return None,
            }
        }
        for envelope in envelopes {
            if envelope.channel != APPLICATION_CHANNEL
                || envelope.delivery != DeliveryClass::ReliableOrdered
                || envelope.persistence != PersistenceClass::Ephemeral
            {
                continue;
            }
            let Some(entity_id) = envelope.entity else {
                return Some(
                    WovenAdapterError::UnexpectedMessage(
                        "application payload did not include an entity ID".to_owned(),
                    )
                    .to_string(),
                );
            };
            let event = RealtimeEvent::Payload {
                entity_id,
                sequence: envelope.sequence,
                payload: envelope.body_json,
            };
            match forward_event(event_tx, event) {
                Ok(()) => {}
                Err(ForwardError::Full) => {
                    return Some("Woven realtime event queue overflow".to_owned());
                }
                Err(ForwardError::Disconnected) => return None,
            }
        }
        if adapter.has_pending_entity_lifecycle() {
            // A bounded lifecycle backlog must not be overtaken by datagrams.
            continue;
        }
        let datagrams = match adapter.drain_unreliable_bounded(datagram_budget) {
            Ok(datagrams) => datagrams,
            Err(error) => return Some(error.to_string()),
        };
        for datagram in datagrams {
            let event = RealtimeEvent::UnreliablePayload {
                entity_id: datagram.entity_id,
                sequence: datagram.sequence,
                payload: datagram.payload,
            };
            match forward_event(event_tx, event) {
                Ok(()) => {}
                Err(ForwardError::Full) => {
                    return Some("Woven realtime event queue overflow".to_owned());
                }
                Err(ForwardError::Disconnected) => return None,
            }
        }
    }
}

fn publish_command(
    adapter: &mut WovenAdapter,
    entity_id: u64,
    command: WorkerCommand,
) -> Result<(), WovenAdapterError> {
    match command {
        WorkerCommand::PublishUnreliable { sequence, payload } => {
            adapter.publish_unreliable_payload(Some(entity_id), sequence, payload)
        }
        WorkerCommand::PublishPositionedUnreliable {
            sequence,
            position,
            payload,
        } => adapter.publish_unreliable_positioned_payload(
            Some(entity_id),
            sequence,
            position,
            payload,
        ),
        WorkerCommand::PublishLatest { sequence, payload }
        | WorkerCommand::PublishReliable { sequence, payload } => adapter.publish_envelope(
            Some(entity_id),
            PayloadEnvelope {
                body_json: payload,
                sequence,
                revision: 0,
                channel: APPLICATION_CHANNEL,
                entity: Some(entity_id),
                delivery: DeliveryClass::ReliableOrdered,
                persistence: PersistenceClass::Ephemeral,
            },
        ),
        WorkerCommand::LogInfo { message } => adapter.log_info(&message),
        WorkerCommand::Stop => Ok(()),
    }
}

fn forward_event(
    event_tx: &SyncSender<RealtimeEvent>,
    event: RealtimeEvent,
) -> Result<(), ForwardError> {
    match event_tx.try_send(event) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(ForwardError::Full),
        Err(TrySendError::Disconnected(_)) => Err(ForwardError::Disconnected),
    }
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod logging_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_driver_forwards_opaque_payload_and_disconnects_once_on_error() {
        let mut driver = WovenRealtimeDriver::connect(WovenConfig::default()).unwrap();
        let entity_id = driver.entity_id();
        let mut events = Vec::new();

        driver.poll(&[], &mut events);
        assert!(matches!(
            events.as_slice(),
            [RealtimeEvent::Connected {
                entity_id: connected
            }] if *connected == entity_id
        ));

        events.clear();
        let payload = "opaque-not-json".to_owned();
        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 1,
                payload: payload.clone(),
            }],
            &mut events,
        );
        let mut received = false;
        for _ in 0..100 {
            received |= events.iter().any(|event| {
                matches!(
                    event,
                    RealtimeEvent::Payload {
                        entity_id: sender,
                        sequence: 1,
                        payload: received,
                    } if *sender == entity_id && received == &payload
                )
            });
            if received {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            events.clear();
            driver.poll(&[], &mut events);
        }
        assert!(received);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RealtimeEvent::Connected { .. }))
        );

        events.clear();
        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 1,
                payload,
            }],
            &mut events,
        );
        let mut disconnected = 0;
        for _ in 0..100 {
            disconnected += events
                .iter()
                .filter(|event| matches!(event, RealtimeEvent::Disconnected { .. }))
                .count();
            if disconnected != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            events.clear();
            driver.poll(&[], &mut events);
        }
        assert_eq!(disconnected, 1);

        events.clear();
        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 2,
                payload: "still opaque".to_owned(),
            }],
            &mut events,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn command_queue_overflow_fails_closed_without_blocking() {
        let (command_tx, command_rx) = sync_channel(1);
        command_tx
            .try_send(WorkerCommand::PublishReliable {
                sequence: 1,
                payload: "queued".to_owned(),
            })
            .unwrap();
        let (event_tx, event_rx) = sync_channel(1);
        let (terminal_tx, terminal_rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let mut driver = WovenRealtimeDriver {
            entity_id: 7,
            command_tx,
            pending_pose: None,
            event_rx,
            terminal_rx,
            stop: Arc::clone(&stop),
            worker: None,
            connected_emitted: false,
            disconnected: false,
        };
        let mut events = Vec::new();

        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 2,
                payload: "overflow".to_owned(),
            }],
            &mut events,
        );

        assert!(stop.load(Ordering::Acquire));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, RealtimeEvent::Disconnected { .. }))
                .count(),
            1
        );
        events.clear();
        driver.poll(&[], &mut events);
        assert!(events.is_empty());

        drop((command_rx, event_tx, terminal_tx));
    }

    fn idle_driver(
        capacity: usize,
    ) -> (
        WovenRealtimeDriver,
        Receiver<WorkerCommand>,
        SyncSender<RealtimeEvent>,
        SyncSender<RealtimeEvent>,
    ) {
        let (command_tx, command_rx) = sync_channel(capacity);
        let (event_tx, event_rx) = sync_channel(CHANNEL_CAPACITY);
        let (terminal_tx, terminal_rx) = sync_channel(1);
        let driver = WovenRealtimeDriver {
            entity_id: 7,
            command_tx,
            pending_pose: None,
            event_rx,
            terminal_rx,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            connected_emitted: false,
            disconnected: false,
        };
        (driver, command_rx, event_tx, terminal_tx)
    }

    #[test]
    fn info_log_queue_is_bounded_and_validation_does_not_disconnect() {
        let (mut driver, command_rx, _event_tx, _terminal_tx) = idle_driver(1);
        for message in [String::new(), "x".repeat(1_025), "é".repeat(513)] {
            assert!(driver.log_info(&message).is_err());
        }
        assert!(matches!(command_rx.try_recv(), Err(TryRecvError::Empty)));
        driver.log_info(&"é".repeat(512)).unwrap();
        assert!(driver.log_info("full queue").is_err());
        assert!(!driver.disconnected);
        assert!(!driver.stop.load(Ordering::Acquire));
        assert!(
            matches!(command_rx.try_recv(), Ok(WorkerCommand::LogInfo { message }) if message.len() == 1_024)
        );
        assert!(matches!(command_rx.try_recv(), Err(TryRecvError::Empty)));
        let mut events = Vec::new();
        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 1,
                payload: "chat after dropped log".to_owned(),
            }],
            &mut events,
        );
        assert!(matches!(
            command_rx.try_recv(),
            Ok(WorkerCommand::PublishReliable { sequence: 1, .. })
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RealtimeEvent::Disconnected { .. }))
        );
    }

    #[test]
    fn info_log_rejects_disconnected_worker_without_retries() {
        let (mut driver, command_rx, _event_tx, terminal_tx) = idle_driver(1);
        terminal_tx
            .try_send(RealtimeEvent::Disconnected {
                reason: "test disconnect".to_owned(),
            })
            .unwrap();
        driver.poll(&[], &mut Vec::new());
        assert!(matches!(
            driver.log_info("too late"),
            Err(WovenAdapterError::NotRunning)
        ));
        assert!(matches!(command_rx.try_recv(), Ok(WorkerCommand::Stop)));
        assert!(matches!(command_rx.try_recv(), Err(TryRecvError::Empty)));
        let (driver, command_rx, _event_tx, _terminal_tx) = idle_driver(1);
        drop(command_rx);
        assert!(matches!(
            driver.log_info("worker gone"),
            Err(WovenAdapterError::NotRunning)
        ));
    }

    #[test]
    fn driver_drop_cancels_queued_info_log_before_worker_dispatch() {
        let (driver, command_rx, event_tx, _terminal_tx) = idle_driver(1);
        let stop = Arc::clone(&driver.stop);
        driver.log_info("queued before drop").unwrap();
        drop(driver);
        assert!(stop.load(Ordering::Acquire));
        let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
        assert_eq!(
            worker_loop(&mut adapter, 7, &command_rx, &event_tx, &stop),
            None
        );
        assert!(matches!(
            command_rx.try_recv(),
            Ok(WorkerCommand::LogInfo { .. })
        ));
    }

    #[test]
    fn full_worker_queue_retains_only_the_newest_binary_pose() {
        let (mut driver, command_rx, _event_tx, _terminal_tx) = idle_driver(1);
        driver
            .command_tx
            .try_send(WorkerCommand::PublishReliable {
                sequence: 1,
                payload: "queued chat".to_owned(),
            })
            .unwrap();
        let mut events = Vec::new();
        for sequence in 2..100 {
            driver.poll(
                &[RealtimeCommand::PublishUnreliable {
                    sequence,
                    payload: vec![0xff, sequence as u8],
                }],
                &mut events,
            );
        }
        assert!(!driver.disconnected);
        assert!(!driver.stop.load(Ordering::Acquire));
        assert!(matches!(&driver.pending_pose,
            Some(WorkerCommand::PublishUnreliable { sequence: 99, payload }) if payload == &[0xff, 99]
        ));
        assert!(matches!(
            command_rx.try_recv(),
            Ok(WorkerCommand::PublishReliable { sequence: 1, .. })
        ));
        assert!(matches!(command_rx.try_recv(), Err(TryRecvError::Empty)));
        driver.poll(&[], &mut events);
        assert!(driver.pending_pose.is_none());
        assert!(matches!(command_rx.try_recv(),
            Ok(WorkerCommand::PublishUnreliable { sequence: 99, payload }) if payload == [0xff, 99]
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RealtimeEvent::Disconnected { .. }))
        );
    }

    #[test]
    fn replaceable_commands_collapse_but_never_overwrite_ordered_chat() {
        let (mut driver, command_rx, _event_tx, _terminal_tx) = idle_driver(CHANNEL_CAPACITY);
        let mut events = Vec::new();
        driver.poll(
            &[
                RealtimeCommand::PublishUnreliable {
                    sequence: 1,
                    payload: vec![0xff, 1],
                },
                RealtimeCommand::PublishUnreliable {
                    sequence: 2,
                    payload: vec![0xff, 2],
                },
                RealtimeCommand::PublishReliable {
                    sequence: 3,
                    payload: "chat one".to_owned(),
                },
                RealtimeCommand::PublishLatest {
                    sequence: 4,
                    payload: "old JSON pose".to_owned(),
                },
                RealtimeCommand::PublishLatest {
                    sequence: 5,
                    payload: "new JSON pose".to_owned(),
                },
                RealtimeCommand::PublishReliable {
                    sequence: 6,
                    payload: "chat two".to_owned(),
                },
            ],
            &mut events,
        );
        assert!(matches!(
            command_rx.try_recv(),
            Ok(WorkerCommand::PublishUnreliable { sequence: 2, .. })
        ));
        assert!(matches!(command_rx.try_recv(),
            Ok(WorkerCommand::PublishReliable { sequence: 3, payload }) if payload == "chat one"
        ));
        assert!(matches!(command_rx.try_recv(),
            Ok(WorkerCommand::PublishLatest { sequence: 5, payload }) if payload == "new JSON pose"
        ));
        assert!(matches!(command_rx.try_recv(),
            Ok(WorkerCommand::PublishReliable { sequence: 6, payload }) if payload == "chat two"
        ));
        assert!(matches!(command_rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(!driver.disconnected);
    }

    #[test]
    fn ordered_chat_overflow_with_a_pending_pose_still_fails_closed() {
        let (mut driver, command_rx, _event_tx, _terminal_tx) = idle_driver(1);
        let mut events = Vec::new();
        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 1,
                payload: "queued".to_owned(),
            }],
            &mut events,
        );
        driver.poll(
            &[RealtimeCommand::PublishUnreliable {
                sequence: 1,
                payload: vec![0xff],
            }],
            &mut events,
        );
        assert!(driver.pending_pose.is_some());
        driver.poll(
            &[RealtimeCommand::PublishReliable {
                sequence: 2,
                payload: "must not disappear".to_owned(),
            }],
            &mut events,
        );
        assert!(driver.disconnected);
        assert!(driver.pending_pose.is_none());
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, RealtimeEvent::Disconnected { .. }))
                .count(),
            1
        );
        assert!(matches!(
            command_rx.try_recv(),
            Ok(WorkerCommand::PublishReliable { sequence: 1, .. })
        ));
    }

    pub(super) fn poll_until(
        driver: &mut WovenRealtimeDriver,
        events: &mut Vec<RealtimeEvent>,
        predicate: impl Fn(&[RealtimeEvent]) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !predicate(events) && std::time::Instant::now() < deadline {
            driver.poll(&[], events);
            assert!(events.len() <= MAX_POLL_EVENTS);
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, RealtimeEvent::Disconnected { .. })),
                "{events:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            predicate(events),
            "expected realtime events did not arrive: {events:?}"
        );
    }

    #[test]
    fn buffered_entry_and_leave_keep_wire_order_through_worker() {
        let server_runtime = tokio::runtime::Runtime::new().unwrap();
        let urls = server_runtime
            .block_on(woven_server::serve_dev_ephemeral(false))
            .unwrap();
        let config = WovenConfig {
            mode: crate::ConnectivityMode::Loopback,
            endpoint: Some(urls.quic),
            ..WovenConfig::default()
        };
        let mut observer = WovenAdapter::new(config.clone()).unwrap();
        observer.start().unwrap();
        let observer_id = observer.entity_id().unwrap();
        let mut peer = WovenAdapter::new(config).unwrap();
        peer.start().unwrap();
        let entity_id = peer.entity_id().unwrap();
        peer.stop();

        let (command_tx, command_rx) = sync_channel(CHANNEL_CAPACITY);
        let (event_tx, event_rx) = sync_channel(CHANNEL_CAPACITY);
        let (terminal_tx, _terminal_rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            run_worker(
                observer,
                observer_id,
                &command_rx,
                &event_tx,
                &terminal_tx,
                worker_stop.as_ref(),
            );
        });
        let received = [
            event_rx.recv_timeout(std::time::Duration::from_secs(2)),
            event_rx.recv_timeout(std::time::Duration::from_secs(2)),
        ];
        stop.store(true, Ordering::Release);
        let _ = command_tx.try_send(WorkerCommand::Stop);
        worker.join().unwrap();
        assert_eq!(
            received.map(Result::unwrap),
            [
                RealtimeEvent::EntityEntered { entity_id },
                RealtimeEvent::EntityLeft { entity_id },
            ]
        );
    }

    #[test]
    fn loopback_worker_forwards_entry_before_bytes_and_reliable_json_with_separate_sequences() {
        let server_runtime = tokio::runtime::Runtime::new().unwrap();
        let urls = server_runtime
            .block_on(woven_server::serve_dev_ephemeral(false))
            .unwrap();
        let config = WovenConfig {
            mode: crate::ConnectivityMode::Loopback,
            endpoint: Some(urls.quic),
            ..WovenConfig::default()
        };
        let mut observer = WovenRealtimeDriver::connect(config.clone()).unwrap();
        let mut sender = WovenAdapter::new(config).unwrap();
        sender.start().unwrap();
        let entity_id = sender.entity_id().unwrap();
        let mut events = Vec::new();
        poll_until(&mut observer, &mut events, |events| {
            events.iter().any(|event| {
            matches!(event, RealtimeEvent::EntityEntered { entity_id: entered } if *entered == entity_id)
        })
        });
        for datagram_sequence in 1..=2 {
            let reliable_sequence = datagram_sequence + 10;
            let bytes = vec![0xff, 0, 0x80, datagram_sequence as u8];
            let json = serde_json::json!({"chat": reliable_sequence});
            sender
                .publish_unreliable_payload(None, datagram_sequence, bytes.clone())
                .unwrap();
            sender
                .publish(
                    1,
                    None,
                    &crate::Payload {
                        body: json.clone(),
                        sequence: reliable_sequence,
                        revision: 0,
                    },
                    DeliveryClass::ReliableOrdered,
                    PersistenceClass::Ephemeral,
                )
                .unwrap();
            let body_json = json.to_string();
            poll_until(&mut observer, &mut events, |events| {
                events.iter().any(|event| matches!(event,
                    RealtimeEvent::UnreliablePayload { entity_id: sender, sequence, payload }
                    if *sender == entity_id && *sequence == datagram_sequence && payload == &bytes
                )) && events.iter().any(|event| matches!(event,
                    RealtimeEvent::Payload { entity_id: sender, sequence, payload }
                    if *sender == entity_id && *sequence == reliable_sequence && payload == &body_json
                ))
            });
        }
        sender.stop();
        poll_until(&mut observer, &mut events, |events| {
            events.iter().any(|event| {
            matches!(event, RealtimeEvent::EntityLeft { entity_id: departed } if *departed == entity_id)
        })
        });
        let entered = events
            .iter()
            .position(|event| matches!(event, RealtimeEvent::EntityEntered { .. }))
            .unwrap();
        let pose = events
            .iter()
            .position(|event| matches!(event, RealtimeEvent::UnreliablePayload { .. }))
            .unwrap();
        let left = events
            .iter()
            .position(|event| matches!(event, RealtimeEvent::EntityLeft { .. }))
            .unwrap();
        assert!(entered < pose && pose < left);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, RealtimeEvent::EntityEntered { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn embedded_worker_preserves_positioned_binary_bytes_and_reliable_json_order() {
        let config = WovenConfig {
            space_id: 3,
            ..WovenConfig::default()
        };
        let mut driver = WovenRealtimeDriver::connect(config).unwrap();
        let entity_id = driver.entity_id();
        let mut events = Vec::new();
        let bytes = vec![0xff, 0, 0x80, 0xfe];
        driver.poll(
            &[
                RealtimeCommand::PublishPositionedUnreliable {
                    sequence: 1,
                    position: [0.0, 0.0, 0.0],
                    payload: vec![0xff, 1],
                },
                RealtimeCommand::PublishPositionedUnreliable {
                    sequence: 2,
                    position: [1.0, 0.0, 0.0],
                    payload: bytes.clone(),
                },
                RealtimeCommand::PublishReliable {
                    sequence: 1,
                    payload: "{\"chat\":1}".to_owned(),
                },
                RealtimeCommand::PublishLatest {
                    sequence: 2,
                    payload: "{\"pose\":2}".to_owned(),
                },
                RealtimeCommand::PublishReliable {
                    sequence: 3,
                    payload: "{\"chat\":3}".to_owned(),
                },
            ],
            &mut events,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut saw_bytes = false;
        let mut reliable = Vec::new();
        while std::time::Instant::now() < deadline && !(saw_bytes && reliable.len() == 3) {
            for event in events.drain(..) {
                match event {
                    RealtimeEvent::UnreliablePayload {
                        entity_id: sender,
                        sequence,
                        payload,
                    } => {
                        assert_eq!(sender, entity_id);
                        assert_eq!(sequence, 2);
                        assert_eq!(payload, bytes);
                        saw_bytes = true;
                    }
                    RealtimeEvent::Payload {
                        entity_id: sender,
                        sequence,
                        payload,
                    } => {
                        assert_eq!(sender, entity_id);
                        reliable.push((sequence, payload));
                    }
                    RealtimeEvent::Disconnected { reason } => {
                        panic!("unexpected disconnect: {reason}")
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
            driver.poll(&[], &mut events);
        }
        assert!(saw_bytes);
        assert_eq!(
            reliable,
            [
                (1, "{\"chat\":1}".to_owned()),
                (2, "{\"pose\":2}".to_owned()),
                (3, "{\"chat\":3}".to_owned()),
            ]
        );
    }

    #[test]
    fn full_event_channel_is_reported_without_retry() {
        let (event_tx, event_rx) = sync_channel(1);
        event_tx
            .try_send(RealtimeEvent::EntityLeft { entity_id: 1 })
            .unwrap();

        assert_eq!(
            forward_event(
                &event_tx,
                RealtimeEvent::Payload {
                    entity_id: 2,
                    sequence: 3,
                    payload: "opaque".to_owned(),
                }
            ),
            Err(ForwardError::Full)
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RealtimeEvent::EntityLeft { entity_id: 1 })
        ));
    }
}
