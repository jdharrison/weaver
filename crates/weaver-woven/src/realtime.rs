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
const MAX_POLL_EVENTS: usize = 128;

#[derive(Debug)]
enum WorkerCommand {
    Publish { sequence: u64, payload: String },
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

    fn signal_stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.try_send(WorkerCommand::Stop);
    }

    fn fail_closed(&mut self, reason: String, events: &mut Vec<RealtimeEvent>) {
        if self.disconnected {
            return;
        }
        self.signal_stop();
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
            let RealtimeCommand::Publish { sequence, payload } = command;
            let worker_command = WorkerCommand::Publish {
                sequence: *sequence,
                payload: payload.clone(),
            };
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
        while work < MAX_ITERATION_WORK {
            if stop.load(Ordering::Acquire) {
                return None;
            }
            let command = match command_rx.try_recv() {
                Ok(command) => command,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return None,
            };
            match command {
                WorkerCommand::Publish { sequence, payload } => {
                    let envelope = PayloadEnvelope {
                        body_json: payload,
                        sequence,
                        revision: 0,
                        channel: APPLICATION_CHANNEL,
                        entity: Some(entity_id),
                        delivery: DeliveryClass::ReliableOrdered,
                        persistence: PersistenceClass::Ephemeral,
                    };
                    if let Err(error) = adapter.publish_envelope(Some(entity_id), envelope) {
                        return Some(error.to_string());
                    }
                }
                WorkerCommand::Stop => return None,
            }
            work += 1;
        }

        let inbound_budget = MAX_ITERATION_WORK - work;
        if inbound_budget == 0 {
            continue;
        }
        let envelopes = match adapter.drain_envelopes_bounded(inbound_budget) {
            Ok(envelopes) => envelopes,
            Err(error) => return Some(error.to_string()),
        };
        let leave_budget = inbound_budget.saturating_sub(envelopes.len());
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
        for entity_id in adapter.drain_entity_leaves_bounded(leave_budget) {
            match forward_event(event_tx, RealtimeEvent::EntityLeft { entity_id }) {
                Ok(()) => {}
                Err(ForwardError::Full) => {
                    return Some("Woven realtime event queue overflow".to_owned());
                }
                Err(ForwardError::Disconnected) => return None,
            }
        }
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
            &[RealtimeCommand::Publish {
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
            &[RealtimeCommand::Publish {
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
            &[RealtimeCommand::Publish {
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
            .try_send(WorkerCommand::Publish {
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
            event_rx,
            terminal_rx,
            stop: Arc::clone(&stop),
            worker: None,
            connected_emitted: false,
            disconnected: false,
        };
        let mut events = Vec::new();

        driver.poll(
            &[RealtimeCommand::Publish {
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
