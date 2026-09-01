//! Render-snapshot extraction interface.

use crate::{EntityId, FrameId, Revision};
use glam::{Quat, Vec3};

/// A timestamped transform sample for a world-space renderable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformSample {
    /// Entity this sample describes.
    pub entity: EntityId,
    /// Coordinate frame the sample is expressed in.
    pub frame: FrameId,
    /// Translation in frame-local meters.
    pub translation: Vec3,
    /// Rotation in frame-local coordinates.
    pub rotation: Quat,
    /// Uniform scale.
    pub scale: f32,
    /// Simulation time at which the sample was committed (seconds).
    pub time_seconds: f64,
    /// World revision at which the sample was committed.
    pub revision: Revision,
}

impl TransformSample {
    /// Create a sample at the origin.
    #[must_use]
    pub const fn identity(entity: EntityId) -> Self {
        Self {
            entity,
            frame: FrameId::ROOT,
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: 1.0,
            time_seconds: 0.0,
            revision: Revision::ZERO,
        }
    }
}

/// Marker trait for renderer-neutral render snapshots.
///
/// Implementors are produced by the simulation layer and consumed by the
/// renderer. The trait is empty because snapshots are domain-specific structs
/// that the renderer crate understands.
pub trait RenderSnapshot: Send + Sync {
    /// Revision of the world this snapshot represents.
    fn revision(&self) -> Revision;
}

/// Capability implemented by simulation worlds to expose render state.
pub trait ExtractSnapshot {
    /// Type of snapshot produced.
    type Snapshot: RenderSnapshot;

    /// Extract an immutable render snapshot at the current revision.
    ///
    /// # Errors
    ///
    /// Returns an error if the world is in a state that cannot be snapshotted.
    fn extract(&self) -> Result<Self::Snapshot, crate::WeaverError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct DummySnapshot {
        rev: Revision,
    }

    impl RenderSnapshot for DummySnapshot {
        fn revision(&self) -> Revision {
            self.rev
        }
    }

    #[test]
    fn snapshot_carries_revision() {
        let snap = DummySnapshot {
            rev: Revision::new(7),
        };
        assert_eq!(snap.revision().get(), 7);
    }
}
