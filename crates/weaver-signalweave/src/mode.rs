//! Connectivity modes for the Signalweave adapter.

/// How Weaver connects to Signalweave.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectivityMode {
    /// Signalweave runs in-process with no network sockets.
    OfflineEmbedded,
    /// A local Signalweave node is hosted by Weaver (unsupported in v0.1).
    LocalHost,
    /// Connect to a remote managed Signalweave node (unsupported in v0.1).
    Remote,
    /// Local embedded node with optional remote bridging (unsupported in v0.1).
    Hybrid,
}

impl ConnectivityMode {
    /// Human-readable description.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::OfflineEmbedded => "offline embedded Signalweave worker",
            Self::LocalHost => "local Signalweave host",
            Self::Remote => "remote Signalweave node",
            Self::Hybrid => "hybrid embedded + remote",
        }
    }
}
