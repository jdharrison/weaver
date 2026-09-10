//! Woven network adapter using local development or verified remote QUIC.

use crate::config::WovenConfig;
use crate::error::WovenAdapterError;
use crate::mode::ConnectivityMode;
use crate::payload::{DeliveryClass, Payload, PayloadEnvelope, PersistenceClass};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use tokio::runtime::Runtime;

use woven_client::{Client, ClientConfig};
use woven_protocol::{ControlPayload, MessagePayload};

const MAX_PENDING_ENTITY_LEAVES: usize = 1_024;

/// Current status of the Woven adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WovenStatus {
    /// The adapter is not running.
    Stopped,
    /// A local Woven development node is running and connected over loopback QUIC.
    EmbeddedLocalNode,
    /// Connected to an externally started local Woven node over loopback QUIC.
    Loopback,
    /// Connected using explicit CA trust and a file-supplied credential over QUIC.
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
    entity_id: Option<u64>,
    next_sequence: HashMap<u64, u64>,
    entity_leaves: VecDeque<u64>,
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
            entity_id: None,
            next_sequence: HashMap::new(),
            entity_leaves: VecDeque::with_capacity(MAX_PENDING_ENTITY_LEAVES),
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

    /// Connect through the public QUIC protocol, starting a node only in embedded mode.
    ///
    /// # Errors
    ///
    /// Returns an error when the local node, client handshake, or subscription cannot start.
    pub fn start(&mut self) -> Result<(), WovenAdapterError> {
        if self.client.is_some() {
            return Ok(());
        }
        let remote = if self.config.mode == ConnectivityMode::RemoteQuic {
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
            ConnectivityMode::Loopback | ConnectivityMode::RemoteQuic => (
                self.config.endpoint.clone().ok_or_else(|| {
                    WovenAdapterError::InitializationFailed(
                        "explicit mode requires an endpoint".to_owned(),
                    )
                })?,
                if self.config.mode == ConnectivityMode::RemoteQuic {
                    WovenStatus::RemoteQuic
                } else {
                    WovenStatus::Loopback
                },
            ),
        };
        let (tls, token) = match remote {
            Some((tls, token)) => (Some(tls), token),
            None => (None, self.config.dev_token.clone()),
        };
        let client_config = ClientConfig {
            url,
            token,
            max_frame_bytes: self.config.max_frame_bytes,
            max_payload_bytes: self.config.max_payload_bytes,
        };
        let (client, entity_id) = runtime.block_on(remote_operation(&self.config, async {
            let mut client = match tls {
                Some(tls) => Client::connect_with_tls(client_config, tls).await,
                None => Client::connect(client_config).await,
            }
            .map_err(client_error)?;
            client
                .join_session(self.config.namespace_id, self.config.session_id)
                .await
                .map_err(client_error)?;
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
            let entity_id = expect_entity_entered(&mut client).await?;
            Ok::<_, WovenAdapterError>((client, entity_id))
        }))?;

        self.runtime = Some(runtime);
        self.client = Some(client);
        self.entity_id = Some(entity_id);
        self.status = status;

        Ok(())
    }

    /// Stop the protocol client and its local Woven runtime.
    pub fn stop(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.close();
        }
        self.runtime = None;
        self.entity_id = None;
        self.next_sequence.clear();
        self.entity_leaves.clear();
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
    /// The local development node defines channel `1` as reliable/ephemeral and
    /// channel `2` as latest-value/stateful. Other client-side policy combinations
    /// are rejected rather than silently rewritten.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running, the sequence is stale, or Woven rejects it.
    pub fn publish_envelope(
        &mut self,
        entity: Option<u64>,
        envelope: PayloadEnvelope,
    ) -> Result<(), WovenAdapterError> {
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
                _ => return Err(WovenAdapterError::UnsupportedChannelPolicy),
            };
            result.map_err(client_error)
        }));
        if let Err(error) = &result
            && self.config.mode == ConnectivityMode::RemoteQuic
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

    /// Drain currently available Woven application envelopes.
    ///
    /// # Errors
    ///
    /// Returns an error when Woven returns an invalid UTF-8 payload or transport error.
    pub fn drain_envelopes(&mut self) -> Result<Vec<PayloadEnvelope>, WovenAdapterError> {
        let client = self.client.as_mut().ok_or(WovenAdapterError::NotRunning)?;
        let runtime = self.runtime.as_ref().ok_or(WovenAdapterError::NotRunning)?;
        let mut payloads = Vec::new();
        let mut entity_leaves = Vec::new();
        let max_messages = if self.config.mode == ConnectivityMode::RemoteQuic {
            128
        } else {
            usize::MAX
        };
        for _ in 0..max_messages {
            let Some(envelope) = runtime.block_on(remote_operation(&self.config, async {
                client
                    .recv_timeout(Duration::from_millis(1))
                    .await
                    .map_err(client_error)
            }))?
            else {
                break;
            };
            if matches!(
                &envelope.message,
                MessagePayload::Control(ControlPayload::EntityLeft(_))
            ) {
                if let Some(entity) = envelope.entity_id {
                    entity_leaves.push(entity);
                }
                continue;
            }
            let (body, delivery) = match envelope.message {
                MessagePayload::ReliableEvent(payload) => {
                    (payload.bytes, DeliveryClass::ReliableOrdered)
                }
                MessagePayload::EntityState(payload) => (payload.bytes, DeliveryClass::LatestValue),
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
        for entity in entity_leaves {
            if self.entity_leaves.len() == MAX_PENDING_ENTITY_LEAVES {
                self.entity_leaves.pop_front();
            }
            self.entity_leaves.push_back(entity);
        }
        Ok(payloads)
    }

    /// Drain entity IDs that Woven reported as having left the subscribed space.
    #[must_use]
    pub fn drain_entity_leaves(&mut self) -> Vec<u64> {
        self.entity_leaves.drain(..).collect()
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

async fn remote_operation<T>(
    config: &WovenConfig,
    operation: impl std::future::Future<Output = Result<T, WovenAdapterError>>,
) -> Result<T, WovenAdapterError> {
    if config.mode != ConnectivityMode::RemoteQuic {
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
