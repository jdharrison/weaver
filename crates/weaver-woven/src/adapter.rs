//! Woven network adapter using local development or verified remote QUIC.

use crate::config::WovenConfig;
use crate::error::WovenAdapterError;
use crate::mode::ConnectivityMode;
use crate::payload::{
    DeliveryClass, Payload, PayloadEnvelope, PersistenceClass, UnreliablePayloadEnvelope,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use tokio::runtime::Runtime;
use weaver_app_core::RealtimeEvent;

use woven_client::{
    Client, ClientConfig, DatagramReceiver, ManagedAdmissionOutcome, RoutingPosition3D,
};
use woven_protocol::{
    AdmissionRejectionCode, AdmissionStatus, AuthenticationScheme, ControlPayload, MessageKind,
    MessagePayload, QueueState,
};

const APPLICATION_PAYLOAD_TYPE_ID: u64 = 1;
const UNRELIABLE_CHANNEL: u64 = 4;
const DRAIN_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_DRAIN_TIME: Duration = Duration::from_millis(4);
const MAX_DRAIN_MESSAGES: usize = 128;
const MAX_PENDING_LIFECYCLE_EVENTS: usize = 1_024;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const LOG_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_ADMISSION_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ADMISSION_TIMEOUT: Duration = Duration::from_mins(15);

/// Current status of the Woven adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WovenStatus {
    /// The adapter is not running.
    Stopped,
    /// A local Woven development node is running and connected over loopback QUIC.
    EmbeddedLocalNode,
    /// Connected to an externally started local Woven node over loopback QUIC.
    Loopback,
    /// Connected to a Host-provisioned scope with verified TLS and Bearer admission.
    ManagedQuic,
    /// Connected using explicit CA trust and a file-supplied static credential over QUIC.
    RemoteQuic,
    /// The adapter encountered an error.
    Error(String),
}

/// A narrow Woven protocol adapter that does not expose Woven implementation types to Weaver.
pub struct WovenAdapter {
    config: WovenConfig,
    status: WovenStatus,
    runtime: Option<Runtime>,
    client: Option<Client>,
    datagram_receiver: Option<DatagramReceiver>,
    entity_id: Option<u64>,
    next_sequence: HashMap<u64, u64>,
    entity_lifecycle: VecDeque<RealtimeEvent>,
}

impl WovenAdapter {
    /// Create a new adapter from configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested connectivity mode is unsupported.
    pub fn new(config: WovenConfig) -> Result<Self, WovenAdapterError> {
        config.validate()?;
        Ok(Self {
            config,
            status: WovenStatus::Stopped,
            runtime: None,
            client: None,
            datagram_receiver: None,
            entity_id: None,
            next_sequence: HashMap::new(),
            entity_lifecycle: VecDeque::with_capacity(MAX_PENDING_LIFECYCLE_EVENTS),
        })
    }

    /// Current adapter status.
    #[must_use]
    pub fn status(&self) -> WovenStatus {
        self.status.clone()
    }

    /// Server-assigned entity ID for this connection, when running.
    #[must_use]
    pub const fn entity_id(&self) -> Option<u64> {
        self.entity_id
    }

    /// Whether the connected server negotiated atomic positioned entity state.
    #[must_use]
    pub fn supports_positioned_state(&self) -> bool {
        self.client
            .as_ref()
            .is_some_and(Client::supports_positioned_state)
    }

    /// Replace the monotonic deadline used to bound subsequent verified-network operations.
    ///
    /// This supports a separately bounded startup/admission phase followed by a run deadline.
    /// It does not schedule shutdown; the caller remains responsible for stopping the adapter.
    pub fn set_run_deadline(&mut self, deadline: Option<std::time::Instant>) {
        self.config.run_deadline = deadline;
    }

    /// Connect through the public QUIC protocol, starting a node only in embedded mode.
    ///
    /// # Errors
    ///
    /// Returns an error when the local node, client handshake, or subscription cannot start.
    pub fn start(&mut self) -> Result<(), WovenAdapterError> {
        if self.client.is_some() {
            return Ok(());
        }
        let verified = if self.config.mode.uses_verified_tls() {
            Some(crate::remote::load(&self.config)?)
        } else {
            None
        };
        let runtime = Runtime::new()
            .map_err(|error| WovenAdapterError::InitializationFailed(error.to_string()))?;
        let (url, status) = match self.config.mode {
            ConnectivityMode::EmbeddedLocalNode => {
                let urls = runtime
                    .block_on(woven_server::serve_dev_ephemeral(false))
                    .map_err(|error| WovenAdapterError::InitializationFailed(error.to_string()))?;
                (urls.quic, WovenStatus::EmbeddedLocalNode)
            }
            ConnectivityMode::Loopback
            | ConnectivityMode::ManagedQuic
            | ConnectivityMode::RemoteQuic => {
                let status = match self.config.mode {
                    ConnectivityMode::Loopback => WovenStatus::Loopback,
                    ConnectivityMode::ManagedQuic => WovenStatus::ManagedQuic,
                    ConnectivityMode::RemoteQuic => WovenStatus::RemoteQuic,
                    ConnectivityMode::EmbeddedLocalNode => unreachable!(),
                };
                (
                    self.config.endpoint.clone().ok_or_else(|| {
                        WovenAdapterError::InitializationFailed(
                            "explicit mode requires an endpoint".to_owned(),
                        )
                    })?,
                    status,
                )
            }
        };
        let (tls, token) = match verified {
            Some((tls, token)) => (Some(tls), token),
            None => (None, self.config.dev_token.clone()),
        };
        let client_config = ClientConfig {
            url,
            token,
            max_frame_bytes: self.config.max_frame_bytes,
            max_payload_bytes: self.config.max_payload_bytes,
        };
        let mut client = runtime.block_on(remote_operation(&self.config, async {
            match (tls, self.config.mode) {
                (Some(tls), ConnectivityMode::ManagedQuic) => {
                    Client::connect_with_tls_and_auth(
                        client_config,
                        tls,
                        AuthenticationScheme::Bearer,
                    )
                    .await
                }
                (Some(tls), ConnectivityMode::RemoteQuic) => {
                    Client::connect_with_tls(client_config, tls).await
                }
                (None, _) => Client::connect(client_config).await,
                (Some(_), _) => unreachable!("verified TLS is mode-validated"),
            }
            .map_err(client_error)
        }))?;
        if self.config.mode == ConnectivityMode::ManagedQuic {
            let timeout = managed_admission_timeout(&self.config)?;
            let key = managed_admission_key();
            let (admitted, outcome) = runtime
                .block_on(client.admit_with_cancellation(
                    self.config.namespace_id,
                    self.config.session_id,
                    key,
                    timeout,
                    std::future::pending::<()>(),
                ))
                .map_err(admission_error)?;
            ensure_managed_admitted(outcome)?;
            client = admitted;
        } else {
            runtime.block_on(remote_operation(&self.config, async {
                client
                    .join_session(self.config.namespace_id, self.config.session_id)
                    .await
                    .map_err(client_error)
            }))?;
        }
        let entity_id = runtime.block_on(remote_operation(&self.config, async {
            client
                .subscribe_space(
                    self.config.namespace_id,
                    self.config.session_id,
                    self.config.space_id,
                    self.config.space_epoch,
                    1,
                )
                .await
                .map_err(client_error)?;
            expect_subscription(&mut client).await?;
            expect_entity_entered(&mut client).await
        }))?;

        // Admission and subscription consume control messages before receiver handout.
        let datagram_receiver = client.take_datagram_receiver().map_err(client_error)?;
        self.runtime = Some(runtime);
        self.client = Some(client);
        self.datagram_receiver = Some(datagram_receiver);
        self.entity_id = Some(entity_id);
        self.status = status;

        Ok(())
    }

    /// Stop the client, allowing up to two seconds for transport close before
    /// shutting down its runtime, independently of the configured run deadline.
    ///
    /// This synchronous method blocks the caller, including in a Tokio context;
    /// async callers should offload it to `spawn_blocking` to remain responsive.
    /// It does not guarantee peer receipt or delivery of pending application data.
    pub fn stop(&mut self) {
        self.datagram_receiver = None;
        let client = self.client.take();
        if let Some(runtime) = self.runtime.take() {
            // Neither nested block_on nor dropping a runtime in an async context
            // is safe. A joined thread also supports current-thread Tokio callers.
            if tokio::runtime::Handle::try_current().is_ok() {
                std::thread::scope(|scope| {
                    if scope
                        .spawn(move || shutdown_client(runtime, client))
                        .join()
                        .is_err()
                    {
                        tracing::warn!(
                            "Woven shutdown thread panicked; peer cleanup is not confirmed"
                        );
                    }
                });
            } else {
                shutdown_client(runtime, client);
            }
        } else if let Some(client) = client {
            let _ = client.close();
        }
        self.entity_id = None;
        self.next_sequence.clear();
        self.entity_lifecycle.clear();
        self.status = WovenStatus::Stopped;
    }

    /// Return the entity ID assigned when the subscription was accepted.
    ///
    /// This compatibility method does not create a new entity: Woven assigns it while subscribing.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running.
    pub fn spawn_entity(&mut self) -> Result<u64, WovenAdapterError> {
        self.entity_id.ok_or(WovenAdapterError::NotRunning)
    }

    /// Send one session-scoped info log through the client's ordered control stream.
    ///
    /// Success means sent, not persisted in Host Logs. Local validation and unsupported
    /// capabilities do not stop the adapter, and failures are never retried. A cancelled
    /// write closes the potentially partial control stream rather than reusing it.
    ///
    /// # Errors
    ///
    /// Returns an error when stopped, the SDK rejects the message, or the bounded write fails.
    pub fn log_info(&mut self, message: &str) -> Result<(), WovenAdapterError> {
        let client = self.client.as_mut().ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let result = runtime.block_on(remote_operation(&self.config, async {
            tokio::time::timeout(LOG_TIMEOUT, client.logger().info(message))
                .await
                .map_err(|_| {
                    WovenAdapterError::ClientFailed("client log write timed out".to_owned())
                })
        }));
        match result {
            Ok(result) => result.map_err(client_error),
            Err(error) => {
                // A cancelled control write may be partial; its stream is no longer safe to reuse.
                self.stop();
                self.status = WovenStatus::Error(error.to_string());
                Err(error)
            }
        }
    }

    /// Publish a typed payload through the configured Woven channel.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, channel-policy validation, or transport fails.
    pub fn publish<T>(
        &mut self,
        channel: u64,
        entity: Option<u64>,
        payload: &Payload<T>,
        delivery: DeliveryClass,
        persistence: PersistenceClass,
    ) -> Result<(), WovenAdapterError>
    where
        T: Serialize,
    {
        let envelope =
            PayloadEnvelope::from_payload(payload, channel, entity, delivery, persistence)?;
        self.publish_envelope(entity, envelope)
    }

    /// Publish a pre-serialized envelope through the Woven client.
    ///
    /// Managed mode permits only channel `1` as reliable/ephemeral. Development and
    /// static remote modes also permit channel `2` as latest-value/stateful. Other
    /// client-side policy combinations are rejected before any network write.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running, the sequence is stale, or the
    /// local policy rejects it. Successful return means the frame was sent; asynchronous
    /// server rejection is surfaced by [`Self::drain_envelopes`].
    pub fn publish_envelope(
        &mut self,
        entity: Option<u64>,
        envelope: PayloadEnvelope,
    ) -> Result<(), WovenAdapterError> {
        if !supports_channel_policy(
            self.config.mode,
            envelope.channel,
            envelope.delivery,
            envelope.persistence,
        ) {
            return Err(WovenAdapterError::UnsupportedChannelPolicy);
        }
        let last_sequence = self
            .next_sequence
            .get(&envelope.channel)
            .copied()
            .unwrap_or(0);
        if envelope.sequence <= last_sequence {
            return Err(WovenAdapterError::StalePayload);
        }
        let entity_id = entity
            .or(self.entity_id)
            .ok_or(WovenAdapterError::NotRunning)?;
        let client = self.client.as_mut().ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let body = envelope.body_json.into_bytes();
        let result = runtime.block_on(remote_operation(&self.config, async {
            let result = match (envelope.channel, envelope.delivery, envelope.persistence) {
                (1, DeliveryClass::ReliableOrdered, PersistenceClass::Ephemeral) => {
                    client
                        .publish_event(
                            self.config.namespace_id,
                            self.config.session_id,
                            self.config.space_id,
                            self.config.space_epoch,
                            1,
                            entity_id,
                            envelope.sequence,
                            1,
                            body,
                        )
                        .await
                }
                (2, DeliveryClass::LatestValue, PersistenceClass::Stateful) => {
                    client
                        .publish_state(
                            self.config.namespace_id,
                            self.config.session_id,
                            self.config.space_id,
                            self.config.space_epoch,
                            2,
                            entity_id,
                            envelope.sequence,
                            1,
                            body,
                        )
                        .await
                }
                _ => unreachable!("channel policy is validated before sending"),
            };
            result.map_err(client_error)
        }));
        if let Err(error) = &result
            && self.config.mode.uses_verified_tls()
        {
            // A cancelled write may be partial; do not reuse its stream or silently retry.
            self.stop();
            self.status = WovenStatus::Error(error.to_string());
        }
        result?;
        self.next_sequence
            .insert(envelope.channel, envelope.sequence);

        Ok(())
    }

    /// Submit opaque bytes on channel `4` as an ephemeral, sequenced datagram (type `1`).
    ///
    /// Sequences must increase strictly on this channel. Success means local transport
    /// submission, not server acceptance or delivery. An oversized or unsupported
    /// datagram returns an error without retry, fragmentation, or reliable fallback.
    ///
    /// # Errors
    ///
    /// Returns an error when stopped, the sequence is stale, the verified run deadline
    /// has elapsed, or the client's codec or datagram transport rejects the payload.
    pub fn publish_unreliable_payload(
        &mut self,
        entity: Option<u64>,
        sequence: u64,
        payload: Vec<u8>,
    ) -> Result<(), WovenAdapterError> {
        if sequence
            <= self
                .next_sequence
                .get(&UNRELIABLE_CHANNEL)
                .copied()
                .unwrap_or(0)
        {
            return Err(WovenAdapterError::StalePayload);
        }
        let entity_id = entity
            .or(self.entity_id)
            .ok_or(WovenAdapterError::NotRunning)?;
        let client = self.client.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        runtime.block_on(remote_operation(&self.config, async {
            client
                .publish_unreliable_state(
                    self.config.namespace_id,
                    self.config.session_id,
                    self.config.space_id,
                    self.config.space_epoch,
                    UNRELIABLE_CHANNEL,
                    entity_id,
                    sequence,
                    APPLICATION_PAYLOAD_TYPE_ID,
                    payload,
                )
                .map_err(client_error)
        }))?;
        self.next_sequence.insert(UNRELIABLE_CHANNEL, sequence);
        Ok(())
    }

    /// Submit opaque bytes and an atomic 3D routing position on channel `4`.
    ///
    /// There is no unpositioned or reliable fallback. The server validates the selected
    /// preconfigured spatial space and its authoritative bounds.
    ///
    /// # Errors
    ///
    /// Returns an error when positioned state was not negotiated, the position or sequence
    /// is invalid, or the datagram transport rejects the payload.
    pub fn publish_unreliable_positioned_payload(
        &mut self,
        entity: Option<u64>,
        sequence: u64,
        position: [f64; 3],
        payload: Vec<u8>,
    ) -> Result<(), WovenAdapterError> {
        if sequence
            <= self
                .next_sequence
                .get(&UNRELIABLE_CHANNEL)
                .copied()
                .unwrap_or(0)
        {
            return Err(WovenAdapterError::StalePayload);
        }
        let entity_id = entity
            .or(self.entity_id)
            .ok_or(WovenAdapterError::NotRunning)?;
        let client = self.client.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        runtime.block_on(remote_operation(&self.config, async {
            client
                .publish_unreliable_positioned_state(
                    self.config.namespace_id,
                    self.config.session_id,
                    self.config.space_id,
                    self.config.space_epoch,
                    UNRELIABLE_CHANNEL,
                    entity_id,
                    sequence,
                    APPLICATION_PAYLOAD_TYPE_ID,
                    RoutingPosition3D {
                        x: position[0],
                        y: position[1],
                        z: position[2],
                    },
                    payload,
                )
                .map_err(client_error)
        }))?;
        self.next_sequence.insert(UNRELIABLE_CHANNEL, sequence);
        Ok(())
    }

    /// Drain at most 128 channel-`4` datagrams, retaining application bytes unchanged.
    ///
    /// Only the configured scope/epoch, `UnreliableSequenced` class, type `1`, and
    /// nonzero sending entities are accepted. This is independent of the control
    /// stream; callers should also drain JSON envelopes and entity leaves.
    ///
    /// # Errors
    ///
    /// Returns an error when stopped, the deadline expires, or datagram decoding/transport fails.
    pub fn drain_unreliable(
        &mut self,
    ) -> Result<Vec<UnreliablePayloadEnvelope>, WovenAdapterError> {
        self.drain_unreliable_bounded(MAX_DRAIN_MESSAGES)
    }

    pub(crate) fn drain_unreliable_bounded(
        &mut self,
        max_messages: usize,
    ) -> Result<Vec<UnreliablePayloadEnvelope>, WovenAdapterError> {
        let receiver = self
            .datagram_receiver
            .as_mut()
            .ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let started = std::time::Instant::now();
        let mut payloads = Vec::new();
        for _ in 0..max_messages.min(MAX_DRAIN_MESSAGES) {
            if started.elapsed() >= MAX_DRAIN_TIME {
                break;
            }
            let Some(envelope) = runtime.block_on(remote_operation(&self.config, async {
                receiver
                    .recv_timeout(DRAIN_TIMEOUT)
                    .await
                    .map_err(client_error)
            }))?
            else {
                break;
            };
            if let Some(payload) = unreliable_payload(&self.config, envelope) {
                payloads.push(payload);
            }
        }
        Ok(payloads)
    }

    /// Drain currently available Woven application envelopes.
    ///
    /// # Errors
    ///
    /// Returns an error when Woven returns an invalid UTF-8 payload or transport error,
    /// or the bounded lifecycle queue overflows. Drain lifecycle events alongside payloads.
    pub fn drain_envelopes(&mut self) -> Result<Vec<PayloadEnvelope>, WovenAdapterError> {
        self.drain_envelopes_bounded(MAX_DRAIN_MESSAGES)
    }

    pub(crate) fn drain_envelopes_bounded(
        &mut self,
        max_messages: usize,
    ) -> Result<Vec<PayloadEnvelope>, WovenAdapterError> {
        let client = self.client.as_mut().ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let mut payloads = Vec::new();
        let started = std::time::Instant::now();
        for _ in 0..max_messages.min(MAX_DRAIN_MESSAGES) {
            if started.elapsed() >= MAX_DRAIN_TIME {
                break;
            }
            let Some(envelope) = runtime.block_on(remote_operation(&self.config, async {
                client
                    .recv_timeout(DRAIN_TIMEOUT)
                    .await
                    .map_err(client_error)
            }))?
            else {
                break;
            };
            if let MessagePayload::Control(ControlPayload::ProtocolError(error)) = &envelope.message
            {
                if error.related_message_kind == MessageKind::ClientLog {
                    tracing::warn!(code = ?error.code, "Woven client log rejected; continuing realtime connection");
                    continue;
                }
                return Err(WovenAdapterError::ServerRejected(error.code));
            }
            let matches_scope = envelope.namespace_id == self.config.namespace_id
                && envelope.session_id == self.config.session_id
                && envelope.space_id == self.config.space_id
                && envelope.space_epoch == self.config.space_epoch;
            if let Some(event) = entity_lifecycle_event(&self.config, &envelope) {
                queue_lifecycle_event(&mut self.entity_lifecycle, event)?;
                continue;
            }
            let (body, delivery) = match envelope.message {
                MessagePayload::ReliableEvent(payload)
                    if matches_scope && payload.type_id == APPLICATION_PAYLOAD_TYPE_ID =>
                {
                    (payload.bytes, DeliveryClass::ReliableOrdered)
                }
                MessagePayload::EntityState(payload)
                    if matches_scope && payload.type_id == APPLICATION_PAYLOAD_TYPE_ID =>
                {
                    (payload.bytes, DeliveryClass::LatestValue)
                }
                _ => continue,
            };
            let persistence = match envelope.channel_id {
                Some(1) => PersistenceClass::Ephemeral,
                Some(2) => PersistenceClass::Stateful,
                _ => continue,
            };
            payloads.push(PayloadEnvelope {
                body_json: String::from_utf8(body)
                    .map_err(|error| WovenAdapterError::ClientFailed(error.to_string()))?,
                sequence: envelope.sender_sequence,
                revision: 0,
                channel: envelope.channel_id.unwrap_or_default(),
                entity: envelope.entity_id,
                delivery,
                persistence,
            });
        }
        Ok(payloads)
    }

    /// Drain queued `EntityEntered`/`EntityLeft` events in reliable-stream order.
    ///
    /// This does not read the network: call [`Self::drain_envelopes`] first.
    /// The queue holds at most 1,024 events and never silently evicts lifecycle changes.
    /// Startup's own entry is represented by the connection's assigned entity ID;
    /// no existing-peer roster is synthesized.
    #[must_use]
    pub fn drain_entity_lifecycle(&mut self) -> Vec<RealtimeEvent> {
        self.drain_entity_lifecycle_bounded(MAX_PENDING_LIFECYCLE_EVENTS)
    }

    pub(crate) fn drain_entity_lifecycle_bounded(
        &mut self,
        max_messages: usize,
    ) -> Vec<RealtimeEvent> {
        (0..max_messages.min(MAX_PENDING_LIFECYCLE_EVENTS))
            .map_while(|_| self.entity_lifecycle.pop_front())
            .collect()
    }

    pub(crate) fn has_pending_entity_lifecycle(&self) -> bool {
        !self.entity_lifecycle.is_empty()
    }

    /// Drain entity IDs that Woven reported as having left the subscribed space.
    ///
    /// Compatibility projection for leave-only consumers. It also consumes queued
    /// entries; use [`Self::drain_entity_lifecycle`] instead when entries or lifecycle
    /// ordering matter. Both methods consume the same queue and should not be mixed.
    #[must_use]
    pub fn drain_entity_leaves(&mut self) -> Vec<u64> {
        self.drain_entity_lifecycle()
            .into_iter()
            .filter_map(|event| match event {
                RealtimeEvent::EntityLeft { entity_id } => Some(entity_id),
                _ => None,
            })
            .collect()
    }

    /// Drain currently available Woven application payloads.
    ///
    /// # Errors
    ///
    /// Returns an error when Woven returns an invalid payload or transport error.
    pub fn drain<T>(&mut self) -> Result<Vec<Payload<T>>, WovenAdapterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.drain_envelopes()?
            .iter()
            .map(PayloadEnvelope::to_payload)
            .collect::<Result<Vec<_>, _>>()
            .map_err(WovenAdapterError::Serialization)
    }
}

impl Drop for WovenAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

fn entity_lifecycle_event(
    config: &WovenConfig,
    envelope: &woven_protocol::Envelope,
) -> Option<RealtimeEvent> {
    if envelope.namespace_id != config.namespace_id
        || envelope.session_id != config.session_id
        || envelope.space_id != config.space_id
        || envelope.space_epoch != config.space_epoch
        || envelope.delivery_class != woven_protocol::DeliveryClass::ReliableOrdered
    {
        return None;
    }
    let entity_id = envelope.entity_id.filter(|entity| *entity != 0)?;
    match &envelope.message {
        MessagePayload::Control(ControlPayload::EntityEntered(_)) => {
            Some(RealtimeEvent::EntityEntered { entity_id })
        }
        MessagePayload::Control(ControlPayload::EntityLeft(_)) => {
            Some(RealtimeEvent::EntityLeft { entity_id })
        }
        _ => None,
    }
}

fn queue_lifecycle_event(
    queue: &mut VecDeque<RealtimeEvent>,
    event: RealtimeEvent,
) -> Result<(), WovenAdapterError> {
    if queue.len() == MAX_PENDING_LIFECYCLE_EVENTS {
        return Err(WovenAdapterError::ClientFailed(
            "Woven lifecycle event queue overflow".to_owned(),
        ));
    }
    queue.push_back(event);
    Ok(())
}

fn unreliable_payload(
    config: &WovenConfig,
    envelope: woven_protocol::Envelope,
) -> Option<UnreliablePayloadEnvelope> {
    if envelope.namespace_id != config.namespace_id
        || envelope.session_id != config.session_id
        || envelope.space_id != config.space_id
        || envelope.space_epoch != config.space_epoch
        || envelope.channel_id != Some(UNRELIABLE_CHANNEL)
        || envelope.delivery_class != woven_protocol::DeliveryClass::UnreliableSequenced
    {
        return None;
    }
    let entity_id = envelope.entity_id.filter(|entity| *entity != 0)?;
    let MessagePayload::EntityState(payload) = envelope.message else {
        return None;
    };
    if payload.type_id != APPLICATION_PAYLOAD_TYPE_ID {
        return None;
    }
    Some(UnreliablePayloadEnvelope {
        payload: payload.bytes,
        sequence: envelope.sender_sequence,
        entity_id,
    })
}

fn shutdown_client(runtime: Runtime, client: Option<Client>) {
    if let Some(client) = client {
        // Cleanup must still run after the traffic deadline has expired.
        if runtime
            .block_on(client.close_gracefully(CLOSE_TIMEOUT))
            .is_err()
        {
            tracing::warn!("Woven transport close timed out; peer cleanup is not confirmed");
        }
    }
    // Abort remaining tasks (including an embedded node) without an unbounded
    // runtime drop wait. The transport has already received its separate budget.
    runtime.shutdown_background();
}

async fn remote_operation<T>(
    config: &WovenConfig,
    operation: impl std::future::Future<Output = Result<T, WovenAdapterError>>,
) -> Result<T, WovenAdapterError> {
    if !config.mode.uses_verified_tls() {
        return operation.await;
    }
    let limit = config
        .run_deadline
        .map_or(Duration::from_secs(10), |deadline| {
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_secs(10))
        });
    if limit.is_zero() {
        return Err(WovenAdapterError::ClientFailed(
            "remote run deadline elapsed".to_owned(),
        ));
    }
    tokio::time::timeout(limit, operation).await.map_err(|_| {
        WovenAdapterError::ClientFailed(
            "remote operation timed out or run deadline elapsed".to_owned(),
        )
    })?
}

fn managed_admission_timeout(config: &WovenConfig) -> Result<Duration, WovenAdapterError> {
    let timeout = config
        .run_deadline
        .map_or(DEFAULT_ADMISSION_TIMEOUT, |deadline| {
            deadline.saturating_duration_since(std::time::Instant::now())
        });
    let timeout = timeout.min(MAX_ADMISSION_TIMEOUT);
    if timeout.is_zero() {
        return Err(WovenAdapterError::ClientFailed(
            "managed admission deadline elapsed".to_owned(),
        ));
    }
    Ok(timeout)
}

fn managed_admission_key() -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("weaver-{}-{timestamp}", std::process::id())
}

fn supports_channel_policy(
    mode: ConnectivityMode,
    channel: u64,
    delivery: DeliveryClass,
    persistence: PersistenceClass,
) -> bool {
    matches!(
        (channel, delivery, persistence),
        (
            1,
            DeliveryClass::ReliableOrdered,
            PersistenceClass::Ephemeral
        )
    ) || (mode != ConnectivityMode::ManagedQuic
        && matches!(
            (channel, delivery, persistence),
            (2, DeliveryClass::LatestValue, PersistenceClass::Stateful)
        ))
}

fn ensure_managed_admitted(outcome: ManagedAdmissionOutcome) -> Result<(), WovenAdapterError> {
    match outcome {
        ManagedAdmissionOutcome::Admission(result)
            if result.status == AdmissionStatus::Admitted =>
        {
            Ok(())
        }
        ManagedAdmissionOutcome::Admission(result) => {
            Err(WovenAdapterError::ManagedAdmissionFailed(
                match (result.status, result.rejection_code) {
                    (AdmissionStatus::Paused, _)
                    | (AdmissionStatus::Rejected, AdmissionRejectionCode::ServerPaused) => {
                        "server paused"
                    }
                    (AdmissionStatus::Rejected, AdmissionRejectionCode::QueueFull) => "queue full",
                    (AdmissionStatus::Rejected, AdmissionRejectionCode::QueueDisabled) => {
                        "queue disabled"
                    }
                    (AdmissionStatus::Rejected, AdmissionRejectionCode::AlreadyQueued) => {
                        "already queued"
                    }
                    (AdmissionStatus::Rejected, AdmissionRejectionCode::InvalidIdempotencyKey) => {
                        "invalid idempotency key"
                    }
                    (AdmissionStatus::Rejected, _) => "rejected",
                    _ => "unexpected admission state",
                },
            ))
        }
        ManagedAdmissionOutcome::Queue(update) if update.state == QueueState::Admitted => Ok(()),
        ManagedAdmissionOutcome::Queue(update) => Err(WovenAdapterError::ManagedAdmissionFailed(
            match update.state {
                QueueState::Cancelled => "queue cancelled",
                QueueState::Expired => "queue expired",
                QueueState::Missing => "queue ticket missing",
                _ => "unexpected queue state",
            },
        )),
    }
}

fn admission_error(error: woven_client::ClientError) -> WovenAdapterError {
    match error {
        woven_client::ClientError::Transport(message)
            if message == "admission cancelled; connection closed" =>
        {
            WovenAdapterError::ManagedAdmissionFailed("cancelled")
        }
        woven_client::ClientError::Transport(message)
            if message == "admission deadline exceeded; connection closed" =>
        {
            WovenAdapterError::ManagedAdmissionFailed("deadline exceeded")
        }
        woven_client::ClientError::Transport(message)
            if message == "admission operation timed out; connection closed" =>
        {
            WovenAdapterError::ManagedAdmissionFailed("operation timed out")
        }
        other => client_error(other),
    }
}

async fn expect_subscription(client: &mut Client) -> Result<(), WovenAdapterError> {
    match client.recv().await.map_err(client_error)?.message {
        MessagePayload::Control(ControlPayload::SubscriptionAccepted(_)) => Ok(()),
        other => Err(WovenAdapterError::UnexpectedMessage(format!(
            "expected SubscriptionAccepted, got {:?}",
            other.message_kind()
        ))),
    }
}

async fn expect_entity_entered(client: &mut Client) -> Result<u64, WovenAdapterError> {
    let envelope = client.recv().await.map_err(client_error)?;
    match envelope.message {
        MessagePayload::Control(ControlPayload::EntityEntered(_)) => {
            envelope.entity_id.ok_or_else(|| {
                WovenAdapterError::UnexpectedMessage(
                    "EntityEntered without an entity ID".to_owned(),
                )
            })
        }
        other => Err(WovenAdapterError::UnexpectedMessage(format!(
            "expected EntityEntered, got {:?}",
            other.message_kind()
        ))),
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "used point-free as map_err(client_error), which requires FnOnce(E)"
)]
fn client_error(error: woven_client::ClientError) -> WovenAdapterError {
    // Transport close reasons and handshake diagnostics can contain peer-supplied text.
    let message = match error {
        woven_client::ClientError::Transport(_) => {
            "transport/TLS failure (details redacted)".to_owned()
        }
        woven_client::ClientError::HandshakeFailed(_) => {
            "handshake failed (details redacted)".to_owned()
        }
        woven_client::ClientError::UnsupportedScheme(_) => "unsupported URL scheme".to_owned(),
        other => other.to_string(),
    };
    WovenAdapterError::ClientFailed(message)
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod shutdown_tests;

#[cfg(test)]
#[path = "datagram_tests.rs"]
mod datagram_tests;

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_deadline_prevents_polling_operation() {
        let config = WovenConfig {
            mode: ConnectivityMode::RemoteQuic,
            run_deadline: Some(std::time::Instant::now()),
            ..WovenConfig::default()
        };
        let runtime = Runtime::new().unwrap();
        let result: Result<(), _> = runtime.block_on(remote_operation(&config, async {
            panic!("expired remote operation must not be polled");
        }));
        assert!(result.is_err());
        assert!(
            !client_error(woven_client::ClientError::Transport("secret".into()))
                .to_string()
                .contains("secret")
        );
    }

    #[test]
    fn remote_deadline_cancels_pending_operation() {
        let config = WovenConfig {
            mode: ConnectivityMode::RemoteQuic,
            run_deadline: Some(std::time::Instant::now() + Duration::from_millis(10)),
            ..WovenConfig::default()
        };
        let runtime = Runtime::new().unwrap();
        let result: Result<(), _> =
            runtime.block_on(remote_operation(&config, std::future::pending()));
        assert!(result.is_err());
    }

    #[test]
    fn local_node_startup_shutdown() {
        let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
        assert_eq!(adapter.status(), WovenStatus::Stopped);
        adapter.start().unwrap();
        assert_eq!(adapter.status(), WovenStatus::EmbeddedLocalNode);
        assert!(adapter.entity_id().is_some());
        adapter.stop();
        assert_eq!(adapter.status(), WovenStatus::Stopped);
    }

    #[test]
    fn managed_policy_and_admission_failures_are_classified_locally() {
        assert!(!supports_channel_policy(
            ConnectivityMode::ManagedQuic,
            2,
            DeliveryClass::LatestValue,
            PersistenceClass::Stateful,
        ));
        assert!(supports_channel_policy(
            ConnectivityMode::RemoteQuic,
            2,
            DeliveryClass::LatestValue,
            PersistenceClass::Stateful,
        ));
        assert!(matches!(
            ensure_managed_admitted(ManagedAdmissionOutcome::Admission(
                woven_protocol::AdmissionResult {
                    status: AdmissionStatus::Rejected,
                    rejection_code: AdmissionRejectionCode::QueueFull,
                    ticket_id: None,
                    poll_after_ms: 0,
                    ticket_remaining_ms: 0,
                }
            )),
            Err(WovenAdapterError::ManagedAdmissionFailed("queue full"))
        ));
        assert!(matches!(
            ensure_managed_admitted(ManagedAdmissionOutcome::Queue(
                woven_protocol::QueueUpdate {
                    ticket_id: 1,
                    state: QueueState::Expired,
                    position: 0,
                    poll_after_ms: 0,
                    ticket_remaining_ms: 0,
                    offer_remaining_ms: 0,
                }
            )),
            Err(WovenAdapterError::ManagedAdmissionFailed("queue expired"))
        ));
    }

    #[test]
    fn unsupported_channel_policy_is_rejected_before_send() {
        let mut adapter = WovenAdapter::new(WovenConfig::default()).unwrap();
        adapter.start().unwrap();
        let payload = Payload {
            body: serde_json::json!({"value": 42}),
            sequence: 1,
            revision: 1,
        };
        assert!(matches!(
            adapter.publish(
                1,
                None,
                &payload,
                DeliveryClass::ReliableOrdered,
                PersistenceClass::Stateful
            ),
            Err(WovenAdapterError::UnsupportedChannelPolicy)
        ));
    }
}
