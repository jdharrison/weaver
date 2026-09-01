//! Renderer-neutral scene snapshot.

use crate::{
    Camera, MeshInstance, Particle, ParticleEmitter, RenderError, SpriteInstance, TextRun,
    UiElement,
};
use weaver_core::snapshot::TransformSample;
use weaver_core::{EntityId, FrameId, RenderSnapshot, Revision};

/// An extracted scene description ready for the renderer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSnapshot {
    /// World revision represented by this snapshot.
    pub revision: Revision,
    /// Timestamped transform samples for world-space renderables.
    pub transforms: Vec<TransformSample>,
    /// Coordinate frames referenced by the transforms.
    pub frames: Vec<FrameId>,
    /// Active camera.
    pub camera: Camera,
    /// Background color used to clear the frame.
    pub background_color: [f32; 4],
    /// Mesh instances.
    pub meshes: Vec<MeshInstance>,
    /// Sprite instances.
    pub sprites: Vec<SpriteInstance>,
    /// Particle emitters.
    pub particles: Vec<(EntityId, ParticleEmitter, Vec<crate::Particle>)>,
    /// Background particles rendered before other scene content (e.g. stars).
    pub background_particles: Vec<Particle>,
    /// Text runs (screen-space diagnostics and world-space labels).
    pub text: Vec<TextRun>,
    /// UI elements.
    pub ui: Vec<UiElement>,
    /// Coordinate-frame visualization enabled.
    pub show_coordinate_frames: bool,
    /// Trajectory history visualization enabled.
    pub show_trajectory_history: bool,
    /// Pause state.
    pub paused: bool,
    /// Current simulation time multiplier.
    pub time_multiplier: f64,
    /// Status strings for overlay display.
    pub status_lines: Vec<String>,
}

impl SceneSnapshot {
    /// Create an empty snapshot at the given revision.
    #[must_use]
    pub fn new(revision: Revision) -> Self {
        Self {
            revision,
            transforms: Vec::new(),
            frames: Vec::new(),
            camera: Camera {
                eye: glam::Vec3::ZERO,
                target: glam::Vec3::ZERO,
                up: glam::Vec3::Y,
                projection: crate::Projection::default(),
            },
            background_color: [0.0, 0.0, 0.0, 1.0],
            meshes: Vec::new(),
            sprites: Vec::new(),
            particles: Vec::new(),
            background_particles: Vec::new(),
            text: Vec::new(),
            ui: Vec::new(),
            show_coordinate_frames: false,
            show_trajectory_history: false,
            paused: false,
            time_multiplier: 1.0,
            status_lines: Vec::new(),
        }
    }

    /// Look up the latest transform sample for an entity.
    #[must_use]
    pub fn transform_for(&self, entity: EntityId) -> Option<&TransformSample> {
        self.transforms
            .iter()
            .filter(|s| s.entity == entity)
            .max_by_key(|s| s.revision.get())
    }

    /// Validate that the snapshot can be rendered.
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are inconsistent.
    pub fn validate(&self) -> Result<(), RenderError> {
        for sample in &self.transforms {
            if sample.scale <= 0.0 {
                return Err(RenderError::Internal(format!(
                    "entity {} has non-positive scale",
                    sample.entity
                )));
            }
        }
        Ok(())
    }
}

impl RenderSnapshot for SceneSnapshot {
    fn revision(&self) -> Revision {
        self.revision
    }
}
