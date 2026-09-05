//! Connectivity modes for the Woven adapter.

/// How Weaver connects to Woven.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectivityMode {
    /// Start a local Woven development node and connect through loopback QUIC.
    EmbeddedLocalNode,
    /// Connect to an explicitly configured loopback Woven node.
    Loopback,
}

impl ConnectivityMode {
    /// Human-readable description.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::EmbeddedLocalNode => "embedded local Woven node",
            Self::Loopback => "loopback Woven node",
        }
    }
}
