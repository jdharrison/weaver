//! Typed payload envelope used to move application state through Woven.

use serde::{Deserialize, Serialize};

/// Delivery class mirrored from Woven semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DeliveryClass {
    /// Reliable ordered delivery.
    ReliableOrdered,
    /// Reliable unordered delivery.
    ReliableUnordered,
    /// Latest-value coalesced delivery.
    LatestValue,
    /// Unreliable sequenced delivery.
    UnreliableSequenced,
    /// Best-effort event delivery.
    BestEffortEvent,
}

/// Persistence class mirrored from Woven semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PersistenceClass {
    /// Ephemeral; not retained.
    Ephemeral,
    /// Retained as latest state.
    Stateful,
    /// Retained and journaled.
    Durable,
}

/// A strongly-typed application payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payload<T> {
    /// Application-specific body.
    pub body: T,
    /// Sequence number for ordering and staleness detection.
    pub sequence: u64,
    /// Revision of the world that produced the payload.
    pub revision: u64,
}

/// A type-erased payload envelope suitable for transit through Woven.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PayloadEnvelope {
    /// JSON-encoded payload body.
    pub body_json: String,
    /// Sequence number.
    pub sequence: u64,
    /// World revision.
    pub revision: u64,
    /// Channel identifier.
    pub channel: u64,
    /// Entity identifier, if any.
    pub entity: Option<u64>,
    /// Delivery class.
    pub delivery: DeliveryClass,
    /// Persistence class.
    pub persistence: PersistenceClass,
}

impl PayloadEnvelope {
    /// Create an envelope from a typed payload.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn from_payload<T>(
        payload: &Payload<T>,
        channel: u64,
        entity: Option<u64>,
        delivery: DeliveryClass,
        persistence: PersistenceClass,
    ) -> Result<Self, serde_json::Error>
    where
        T: Serialize,
    {
        Ok(Self {
            body_json: serde_json::to_string(&payload.body)?,
            sequence: payload.sequence,
            revision: payload.revision,
            channel,
            entity,
            delivery,
            persistence,
        })
    }

    /// Decode the envelope into a typed payload.
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails.
    pub fn to_payload<T>(&self) -> Result<Payload<T>, serde_json::Error>
    where
        T: for<'de> Deserialize<'de>,
    {
        Ok(Payload {
            body: serde_json::from_str(&self.body_json)?,
            sequence: self.sequence,
            revision: self.revision,
        })
    }
}
