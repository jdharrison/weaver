//! Weaver-facing adapter for Woven.
//!
//! The adapter starts Woven's development node on loopback and uses the public
//! `WVN1` client protocol; Weaver never accesses `woven-core` directly.

#![warn(missing_docs)]

pub mod adapter;
pub mod config;
pub mod error;
pub mod mode;
pub mod payload;

pub use adapter::{WovenAdapter, WovenStatus};
pub use config::WovenConfig;
pub use error::WovenAdapterError;
pub use mode::ConnectivityMode;
pub use payload::{DeliveryClass, Payload, PayloadEnvelope, PersistenceClass};
