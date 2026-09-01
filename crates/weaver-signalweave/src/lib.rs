//! Weaver-facing adapter for Signalweave.
//!
//! This crate provides a narrow seam over `signalweave-core`. Weaver code
//! must not depend on Signalweave internals; all external state changes enter
//! through typed commands defined in [`weaver_core`].

#![warn(missing_docs)]

pub mod adapter;
pub mod config;
pub mod error;
pub mod mode;
pub mod payload;

pub use adapter::{SignalweaveAdapter, SignalweaveStatus};
pub use config::SignalweaveConfig;
pub use error::SignalweaveAdapterError;
pub use mode::ConnectivityMode;
pub use payload::{DeliveryClass, Payload, PayloadEnvelope, PersistenceClass};
