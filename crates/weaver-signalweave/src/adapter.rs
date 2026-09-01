//! Embedded Signalweave worker adapter.

use crate::config::SignalweaveConfig;
use crate::error::SignalweaveAdapterError;
use crate::mode::ConnectivityMode;
use crate::payload::{DeliveryClass, Payload, PayloadEnvelope, PersistenceClass};
use serde::{Deserialize, Serialize};
use signalweave_core::{
    AccessGrant, AuthenticatedPrincipal, AuthorityContext, AuthorityOutcome, AuthorityPolicy,
    AuthorityRejection, AuthorizationGrants, ChannelDefinition, ChannelId, Command as SwCommand,
    CommandResult as SwCommandResult, CoordinateFrame, Credentials,
    DeliveryClass as SwDeliveryClass, DevAuthenticator, EntityId as SwEntityId,
    NamespaceId as SwNamespaceId, PersistenceClass as SwPersistenceClass, PrincipalId,
    ProposedMessage, PublishRequest, RoutingPolicy, SessionId as SwSessionId, SessionKey,
    SpaceDescriptor, SpaceEpoch as SwSpaceEpoch, SpaceId as SwSpaceId, SpaceKey, WorkerHarness,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, trace};

/// Current status of the Signalweave adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignalweaveStatus {
    /// The adapter is not running.
    Stopped,
    /// The adapter is connected in offline embedded mode.
    OfflineEmbedded,
    /// The adapter encountered an error.
    Error(String),
}

/// A narrow adapter that exposes Signalweave capabilities to Weaver without
/// leaking Signalweave types.
pub struct SignalweaveAdapter {
    config: SignalweaveConfig,
    status: SignalweaveStatus,
    harness: Option<WorkerHarness<DevAuthenticator>>,
    connection: Option<signalweave_core::ConnectionId>,
    namespace: SwNamespaceId,
    space: SwSpaceId,
    epoch: SwSpaceEpoch,
    session_key: SessionKey,
    space_key: SpaceKey,
    next_sequence: HashMap<u64, u64>,
}

impl SignalweaveAdapter {
    /// Create a new adapter from configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested mode is not supported.
    pub fn new(config: SignalweaveConfig) -> Result<Self, SignalweaveAdapterError> {
        if config.mode != ConnectivityMode::OfflineEmbedded {
            return Err(SignalweaveAdapterError::UnsupportedMode(
                config.mode.description().to_string(),
            ));
        }

        let namespace = SwNamespaceId::new(config.namespace_id);
        let session = SwSessionId::new(config.session_id);
        let space = SwSpaceId::new(config.space_id);
        let epoch = SwSpaceEpoch::new(config.space_epoch);
        let session_key = SessionKey::new(namespace, session);
        let space_key = SpaceKey::new(session_key, space);

        Ok(Self {
            config,
            status: SignalweaveStatus::Stopped,
            harness: None,
            connection: None,
            namespace,
            space,
            epoch,
            session_key,
            space_key,
            next_sequence: HashMap::new(),
        })
    }

    /// Current adapter status.
    #[must_use]
    pub fn status(&self) -> SignalweaveStatus {
        self.status.clone()
    }

    /// Start the adapter in the configured mode.
    ///
    /// In offline embedded mode this provisions a local namespace, session,
    /// space, connection, and authenticates with development credentials.
    /// No network sockets are opened.
    ///
    /// # Errors
    ///
    /// Returns an error if initialization fails.
    pub fn start(&mut self) -> Result<(), SignalweaveAdapterError> {
        match self.config.mode {
            ConnectivityMode::OfflineEmbedded => self.start_offline_embedded(),
            other => Err(SignalweaveAdapterError::UnsupportedMode(
                other.description().to_string(),
            )),
        }
    }

    /// Stop the adapter, releasing the embedded worker.
    pub fn stop(&mut self) {
        if let Some(harness) = self.harness.as_mut() {
            if let Some(connection) = self.connection {
                let _ = harness.submit(SwCommand::TransportLost { connection });
                let _ = harness.run_pending();
            }
        }
        self.harness = None;
        self.connection = None;
        self.status = SignalweaveStatus::Stopped;
        info!("Signalweave adapter stopped");
    }

    /// Spawn a Signalweave entity and return its id.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running or the command fails.
    pub fn spawn_entity(&mut self) -> Result<u64, SignalweaveAdapterError> {
        let connection = self.connection.ok_or(SignalweaveAdapterError::NotRunning)?;
        let harness = self
            .harness
            .as_mut()
            .ok_or(SignalweaveAdapterError::NotRunning)?;
        harness
            .submit(SwCommand::SpawnEntity {
                connection,
                space: self.space_key,
                epoch: self.epoch,
            })
            .map_err(|_| SignalweaveAdapterError::CommandFailed("harness full".to_string()))?;
        let result = harness.step().ok_or(SignalweaveAdapterError::NotRunning)?;
        match result {
            Ok(SwCommandResult::EntitySpawned(id)) => Ok(id.get()),
            Ok(other) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "unexpected result: {other:?}"
            ))),
            Err(err) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "spawn entity failed: {err:?}"
            ))),
        }
    }

    /// Publish a typed payload on a channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running, the payload is stale,
    /// or the publish command fails.
    pub fn publish<T>(
        &mut self,
        channel: u64,
        entity: Option<u64>,
        payload: &Payload<T>,
        delivery: DeliveryClass,
        persistence: PersistenceClass,
    ) -> Result<(), SignalweaveAdapterError>
    where
        T: Serialize,
    {
        let envelope =
            PayloadEnvelope::from_payload(payload, channel, entity, delivery, persistence)?;
        self.publish_envelope(entity, envelope)
    }

    /// Publish a pre-serialized envelope.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running, the sequence is stale,
    /// or the publish command fails.
    pub fn publish_envelope(
        &mut self,
        entity: Option<u64>,
        envelope: PayloadEnvelope,
    ) -> Result<(), SignalweaveAdapterError> {
        let last_sequence = self
            .next_sequence
            .get(&envelope.channel)
            .copied()
            .unwrap_or(0);
        if envelope.sequence <= last_sequence {
            return Err(SignalweaveAdapterError::StalePayload);
        }

        let connection = self.connection.ok_or(SignalweaveAdapterError::NotRunning)?;
        let harness = self
            .harness
            .as_mut()
            .ok_or(SignalweaveAdapterError::NotRunning)?;
        let request = PublishRequest {
            connection,
            session: self.session_key,
            space: self.space,
            space_epoch: self.epoch,
            entity: entity.map(SwEntityId::new),
            channel: ChannelId::new(envelope.channel),
            sequence: envelope.sequence,
            delivery: into_sw_delivery(envelope.delivery),
            persistence: into_sw_persistence(envelope.persistence),
            coalesce_key: None,
            payload: envelope.body_json.into_bytes(),
        };

        harness.submit(SwCommand::Publish(request)).map_err(|_| {
            SignalweaveAdapterError::CommandFailed("harness full during publish".to_string())
        })?;

        let result = harness.step().ok_or(SignalweaveAdapterError::NotRunning)?;
        match result {
            Ok(SwCommandResult::Published(_)) => {
                self.next_sequence
                    .insert(envelope.channel, envelope.sequence);
                trace!(
                    channel = envelope.channel,
                    sequence = envelope.sequence,
                    "published"
                );
                Ok(())
            }
            Ok(other) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "unexpected publish result: {other:?}"
            ))),
            Err(err) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "publish failed: {err:?}"
            ))),
        }
    }

    /// Drain outbound messages and convert them into typed payloads.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running or deserialization fails.
    pub fn drain<T>(&mut self) -> Result<Vec<Payload<T>>, SignalweaveAdapterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let connection = self.connection.ok_or(SignalweaveAdapterError::NotRunning)?;
        let harness = self
            .harness
            .as_mut()
            .ok_or(SignalweaveAdapterError::NotRunning)?;
        harness
            .submit(SwCommand::DrainOutbound { connection })
            .map_err(|_| {
                SignalweaveAdapterError::CommandFailed("harness full during drain".to_string())
            })?;
        let result = harness.step().ok_or(SignalweaveAdapterError::NotRunning)?;
        match result {
            Ok(SwCommandResult::Outbound(messages)) => {
                let mut payloads = Vec::with_capacity(messages.len());
                for message in messages {
                    let envelope = PayloadEnvelope {
                        body_json: String::from_utf8_lossy(&message.payload).to_string(),
                        sequence: message.sequence,
                        revision: 0,
                        channel: message.channel.get(),
                        entity: message.entity.map(signalweave_core::EntityId::get),
                        delivery: from_sw_delivery(message.delivery),
                        persistence: from_sw_persistence(message.persistence),
                    };
                    payloads.push(envelope.to_payload()?);
                }
                Ok(payloads)
            }
            Ok(other) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "unexpected drain result: {other:?}"
            ))),
            Err(err) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "drain failed: {err:?}"
            ))),
        }
    }

    /// Return the Signalweave session snapshot for diagnostics.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running.
    pub fn snapshot(
        &mut self,
    ) -> Result<signalweave_core::SessionSnapshot, SignalweaveAdapterError> {
        let connection = self.connection.ok_or(SignalweaveAdapterError::NotRunning)?;
        let harness = self
            .harness
            .as_mut()
            .ok_or(SignalweaveAdapterError::NotRunning)?;
        harness
            .submit(SwCommand::Snapshot {
                connection,
                session: self.session_key,
            })
            .map_err(|_| {
                SignalweaveAdapterError::CommandFailed("harness full during snapshot".to_string())
            })?;
        let result = harness.step().ok_or(SignalweaveAdapterError::NotRunning)?;
        match result {
            Ok(SwCommandResult::Snapshot(snapshot)) => Ok(snapshot),
            Ok(other) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "unexpected snapshot result: {other:?}"
            ))),
            Err(err) => Err(SignalweaveAdapterError::CommandFailed(format!(
                "snapshot failed: {err:?}"
            ))),
        }
    }

    fn start_offline_embedded(&mut self) -> Result<(), SignalweaveAdapterError> {
        let mut authenticator = DevAuthenticator::new();
        let mut grants = AuthorizationGrants::new();
        grants.grant_namespace(self.namespace, AccessGrant::ReadWrite);
        grants.grant_session(self.session_key, AccessGrant::ReadWrite);
        grants.grant_space(self.space_key, AccessGrant::ReadWrite);
        let channel_scope =
            signalweave_core::ChannelScope::new(self.session_key, ChannelId::new(1));
        grants.grant_channel(channel_scope, AccessGrant::ReadWrite);
        let principal = AuthenticatedPrincipal::new(PrincipalId::new(1), grants);
        authenticator
            .insert(&self.config.dev_token, principal)
            .map_err(|_| SignalweaveAdapterError::AuthenticationFailed)?;

        let core = signalweave_core::SignalweaveCore::new(
            authenticator,
            signalweave_core::CoreConfig::default(),
        )
        .map_err(|err| {
            SignalweaveAdapterError::InitializationFailed(format!("core creation failed: {err:?}"))
        })?;

        let worker = signalweave_core::TransportIndependentWorker::new(core);
        let mut harness =
            WorkerHarness::new(worker, self.config.harness_capacity).map_err(|_| {
                SignalweaveAdapterError::InitializationFailed(
                    "harness capacity is zero".to_string(),
                )
            })?;

        let connection_result = harness
            .submit(SwCommand::TransportConnected)
            .map_err(|_| SignalweaveAdapterError::InitializationFailed("harness full".to_string()))
            .and_then(|()| {
                harness
                    .step()
                    .ok_or(SignalweaveAdapterError::InitializationFailed(
                        "no connection result".to_string(),
                    ))
            })?;
        let connection = match connection_result {
            Ok(SwCommandResult::Connected(id)) => id,
            Ok(other) => {
                return Err(SignalweaveAdapterError::InitializationFailed(format!(
                    "unexpected connection result: {other:?}"
                )));
            }
            Err(err) => {
                return Err(SignalweaveAdapterError::InitializationFailed(format!(
                    "connection failed: {err:?}"
                )));
            }
        };

        harness
            .submit(SwCommand::Authenticate {
                connection,
                credentials: Credentials::new(&self.config.dev_token),
            })
            .map_err(|_| {
                SignalweaveAdapterError::InitializationFailed("harness full".to_string())
            })?;
        let auth_result = harness
            .step()
            .ok_or(SignalweaveAdapterError::InitializationFailed(
                "no auth result".to_string(),
            ))?;
        match auth_result {
            Ok(SwCommandResult::Authenticated(_)) => {}
            Ok(other) => {
                return Err(SignalweaveAdapterError::InitializationFailed(format!(
                    "unexpected auth result: {other:?}"
                )));
            }
            Err(_) => return Err(SignalweaveAdapterError::AuthenticationFailed),
        }

        harness
            .worker_mut()
            .core_mut()
            .provision_session(self.session_key)
            .map_err(|err| {
                SignalweaveAdapterError::InitializationFailed(format!(
                    "session provisioning failed: {err:?}"
                ))
            })?;

        harness
            .worker_mut()
            .core_mut()
            .install_space(
                self.session_key,
                SpaceDescriptor {
                    id: self.space,
                    local_frame: CoordinateFrame::Cartesian3D {
                        meters_per_unit: 1.0,
                    },
                    parent: None,
                    epoch: self.epoch,
                    routing: RoutingPolicy::BroadcastAll,
                },
            )
            .map_err(|err| {
                SignalweaveAdapterError::InitializationFailed(format!(
                    "space installation failed: {err:?}"
                ))
            })?;

        harness
            .submit(SwCommand::JoinSession {
                connection,
                session: self.session_key,
            })
            .map_err(|_| {
                SignalweaveAdapterError::InitializationFailed("harness full".to_string())
            })?;
        let _ = harness.step();

        harness
            .submit(SwCommand::Subscribe {
                connection,
                space: self.space_key,
            })
            .map_err(|_| {
                SignalweaveAdapterError::InitializationFailed("harness full".to_string())
            })?;
        let _ = harness.step();

        let channel = ChannelDefinition::with_authority(
            ChannelId::new(1),
            SwDeliveryClass::ReliableOrdered,
            SwPersistenceClass::Stateful,
            64 * 1024,
            Arc::new(PermissiveAuthority),
        );
        harness
            .worker_mut()
            .core_mut()
            .register_channel(channel)
            .map_err(|err| {
                SignalweaveAdapterError::InitializationFailed(format!(
                    "channel registration failed: {err:?}"
                ))
            })?;

        self.harness = Some(harness);
        self.connection = Some(connection);
        self.status = SignalweaveStatus::OfflineEmbedded;
        debug!("Signalweave adapter started in offline embedded mode");
        Ok(())
    }
}

impl Drop for SignalweaveAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

fn into_sw_delivery(delivery: DeliveryClass) -> SwDeliveryClass {
    match delivery {
        DeliveryClass::ReliableOrdered => SwDeliveryClass::ReliableOrdered,
        DeliveryClass::ReliableUnordered => SwDeliveryClass::ReliableUnordered,
        DeliveryClass::LatestValue => SwDeliveryClass::LatestValue,
        DeliveryClass::UnreliableSequenced => SwDeliveryClass::UnreliableSequenced,
        DeliveryClass::BestEffortEvent => SwDeliveryClass::BestEffortEvent,
    }
}

fn from_sw_delivery(delivery: SwDeliveryClass) -> DeliveryClass {
    match delivery {
        SwDeliveryClass::ReliableOrdered => DeliveryClass::ReliableOrdered,
        SwDeliveryClass::ReliableUnordered => DeliveryClass::ReliableUnordered,
        SwDeliveryClass::LatestValue => DeliveryClass::LatestValue,
        SwDeliveryClass::UnreliableSequenced => DeliveryClass::UnreliableSequenced,
        SwDeliveryClass::BestEffortEvent => DeliveryClass::BestEffortEvent,
    }
}

fn into_sw_persistence(persistence: PersistenceClass) -> SwPersistenceClass {
    match persistence {
        PersistenceClass::Ephemeral => SwPersistenceClass::Ephemeral,
        PersistenceClass::Stateful => SwPersistenceClass::Stateful,
        PersistenceClass::Durable => SwPersistenceClass::Durable,
    }
}

fn from_sw_persistence(persistence: SwPersistenceClass) -> PersistenceClass {
    match persistence {
        SwPersistenceClass::Ephemeral => PersistenceClass::Ephemeral,
        SwPersistenceClass::Stateful => PersistenceClass::Stateful,
        SwPersistenceClass::Durable => PersistenceClass::Durable,
    }
}

/// A development authority that accepts all messages from session members and
/// space subscribers. This is only appropriate for offline embedded mode.
#[derive(Clone, Copy, Debug)]
struct PermissiveAuthority;

impl AuthorityPolicy for PermissiveAuthority {
    fn evaluate(
        &self,
        context: &AuthorityContext,
        _proposed: ProposedMessage<'_>,
    ) -> AuthorityOutcome {
        if !context.is_session_member {
            return AuthorityOutcome::Reject(AuthorityRejection::SessionMembershipRequired);
        }
        if !context.is_space_subscriber {
            return AuthorityOutcome::Reject(AuthorityRejection::SpaceSubscriptionRequired);
        }
        AuthorityOutcome::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct TestState {
        value: u32,
    }

    #[test]
    fn offline_embedded_startup_shutdown() {
        let mut adapter = SignalweaveAdapter::new(SignalweaveConfig::default()).unwrap();
        assert_eq!(adapter.status(), SignalweaveStatus::Stopped);
        adapter.start().unwrap();
        assert_eq!(adapter.status(), SignalweaveStatus::OfflineEmbedded);
        adapter.stop();
        assert_eq!(adapter.status(), SignalweaveStatus::Stopped);
    }

    #[test]
    fn unsupported_modes_fail_on_start() {
        for mode in [
            ConnectivityMode::LocalHost,
            ConnectivityMode::Remote,
            ConnectivityMode::Hybrid,
        ] {
            let config = SignalweaveConfig {
                mode,
                ..Default::default()
            };
            let result = SignalweaveAdapter::new(config);
            assert!(
                matches!(result, Err(SignalweaveAdapterError::UnsupportedMode(_))),
                "mode {mode:?} should be unsupported"
            );
        }
    }

    #[test]
    fn publish_and_drain_round_trip() {
        let mut adapter = SignalweaveAdapter::new(SignalweaveConfig::default()).unwrap();
        adapter.start().unwrap();
        let payload = Payload {
            body: TestState { value: 42 },
            sequence: 1,
            revision: 1,
        };
        adapter
            .publish(
                1,
                None,
                &payload,
                DeliveryClass::ReliableOrdered,
                PersistenceClass::Stateful,
            )
            .unwrap();
        let received: Vec<Payload<TestState>> = adapter.drain().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].body.value, 42);
    }

    #[test]
    fn stale_payload_rejected() {
        let mut adapter = SignalweaveAdapter::new(SignalweaveConfig::default()).unwrap();
        adapter.start().unwrap();
        let payload = Payload {
            body: TestState { value: 1 },
            sequence: 5,
            revision: 1,
        };
        adapter
            .publish(
                1,
                None,
                &payload,
                DeliveryClass::ReliableOrdered,
                PersistenceClass::Stateful,
            )
            .unwrap();
        let stale = PayloadEnvelope::from_payload(
            &Payload {
                body: TestState { value: 2 },
                sequence: 3,
                revision: 1,
            },
            1,
            None,
            DeliveryClass::ReliableOrdered,
            PersistenceClass::Stateful,
        )
        .unwrap();
        assert!(matches!(
            adapter.publish_envelope(None, stale),
            Err(SignalweaveAdapterError::StalePayload)
        ));
    }
}
