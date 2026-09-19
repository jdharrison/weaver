//! Connectivity modes for the Woven adapter.

/// How Weaver connects to Woven.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectivityMode {
    /// Start a local Woven development node and connect through loopback QUIC.
    EmbeddedLocalNode,
    /// Connect to an explicitly configured loopback Woven node.
    Loopback,
    /// Connect to a Host-provisioned managed QUIC scope using verified TLS and Bearer admission.
    ManagedQuic,
    /// Connect to an explicit static QUIC endpoint using verified TLS and a token file.
    RemoteQuic,
}

impl ConnectivityMode {
    /// Human-readable description.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::EmbeddedLocalNode => "embedded local Woven node",
            Self::Loopback => "loopback Woven node",
            Self::ManagedQuic => "managed Woven QUIC scope",
            Self::RemoteQuic => "verified remote Woven QUIC node",
        }
    }

    pub(crate) const fn uses_verified_tls(self) -> bool {
        matches!(self, Self::ManagedQuic | Self::RemoteQuic)
    }
}
