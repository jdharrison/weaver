//! Application world that composes simulation, Woven, and render state.

use crate::error::AppError;
use crate::event::InputEvent;
use glam::Vec3;
use std::collections::HashMap;
use std::time::Duration;
use weaver_core::snapshot::TransformSample;
use weaver_core::{EntityId, Revision};
use weaver_render::{
    Camera, MeshInstance, Particle, ParticleEmitter, SceneSnapshot, SpriteInstance,
};
use weaver_worldline::FrameId as WlFrameId;
use weaver_worldline::{
    FrameRegistry, SimulationClockConfig, SimulationControl, SimulationRuntime, TrajectorySample,
};
use weaver_woven::{
    DeliveryClass, Payload, PayloadEnvelope, PersistenceClass, WovenAdapter, WovenConfig,
};

/// Application world configuration.
#[derive(Clone, Debug, Default)]
pub struct WorldConfig {
    /// Simulation clock configuration.
    pub clock: SimulationClockConfig,
    /// Woven connection configuration.
    pub woven: WovenConfig,
    /// Whether rendering coordinate frames is enabled.
    pub show_coordinate_frames: bool,
    /// Whether trajectory history is enabled.
    pub show_trajectory_history: bool,
    /// Number of background star particles to generate.
    pub starfield_count: usize,
    /// Background clear color.
    pub background_color: [f32; 4],
}

/// Per-entity renderable state.
#[derive(Clone, Debug, Default)]
pub struct Renderable {
    /// Associated mesh instance, if any.
    pub mesh: Option<MeshInstance>,
    /// Associated sprite instance, if any.
    pub sprite: Option<SpriteInstance>,
    /// Associated particle emitter, if any.
    pub emitter: Option<ParticleEmitter>,
    /// World-space label.
    pub label: Option<String>,
    /// Parent frame.
    pub frame: WlFrameId,
}

/// The runtime world that owns simulation state, entity registry, and Woven connectivity.
pub struct WeaverWorld {
    config: WorldConfig,
    runtime: SimulationRuntime,
    woven: Option<WovenAdapter>,
    entities: HashMap<EntityId, Renderable>,
    revision: Revision,
    paused: bool,
    time_multiplier: f64,
    camera: Camera,
    replicated_payloads: Vec<PayloadEnvelope>,
    departed_woven_entities: Vec<u64>,
}

impl WeaverWorld {
    /// Create a new world from configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the Woven adapter cannot be created.
    pub fn new(config: WorldConfig) -> Result<Self, AppError> {
        let runtime = SimulationRuntime::new(config.clock);
        let mut adapter = WovenAdapter::new(config.woven.clone())?;
        adapter.start()?;
        let woven = Some(adapter);
        Ok(Self {
            config,
            runtime,
            woven,
            entities: HashMap::new(),
            revision: Revision::ZERO,
            paused: false,
            time_multiplier: 1.0,
            camera: Camera::default(),
            replicated_payloads: Vec::new(),
            departed_woven_entities: Vec::new(),
        })
    }

    /// Spawn an entity with the given renderable description.
    pub fn spawn(&mut self, renderable: Renderable) -> EntityId {
        let id = EntityId::new();
        self.entities.insert(id, renderable);
        id
    }

    /// Remove an entity.
    pub fn remove(&mut self, id: EntityId) -> Option<Renderable> {
        self.entities.remove(&id)
    }

    /// Access an entity's renderable state.
    #[must_use]
    pub fn get(&self, id: EntityId) -> Option<&Renderable> {
        self.entities.get(&id)
    }

    /// Mutable access to an entity's renderable state.
    #[must_use]
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Renderable> {
        self.entities.get_mut(&id)
    }

    /// Process an input event.
    pub fn handle_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::TogglePause => {
                if self.paused {
                    self.paused = false;
                    self.runtime.resume();
                } else {
                    self.paused = true;
                    self.runtime.pause();
                }
            }
            InputEvent::SetTimeMultiplier(m) => {
                self.time_multiplier = m;
                self.runtime.set_time_multiplier(m);
            }
            InputEvent::ToggleCoordinateFrames => {
                self.config.show_coordinate_frames = !self.config.show_coordinate_frames;
            }
            InputEvent::ToggleTrajectoryHistory => {
                self.config.show_trajectory_history = !self.config.show_trajectory_history;
            }
            InputEvent::Resized { .. } => {}
        }
    }

    /// Advance the simulation by one fixed step.
    ///
    /// # Errors
    ///
    /// Returns an error if the simulation step fails.
    pub fn step(&mut self) -> Result<Option<Duration>, AppError> {
        if self.paused {
            return Ok(None);
        }
        let delta = self.runtime.step();
        self.revision = self.revision.next();
        self.record_samples();
        self.process_woven()?;
        Ok(delta)
    }

    /// Start the world simulation.
    pub fn start(&mut self) {
        self.paused = false;
        self.runtime.start();
    }

    /// Pause the world simulation.
    pub fn pause(&mut self) {
        self.paused = true;
        self.runtime.pause();
    }

    /// Resume the world simulation.
    pub fn resume(&mut self) {
        self.paused = false;
        self.runtime.resume();
    }

    /// Reset the world.
    pub fn reset(&mut self) {
        self.runtime.reset();
        self.revision = Revision::ZERO;
        self.replicated_payloads.clear();
        self.departed_woven_entities.clear();
    }

    /// Current world revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Current simulation time in seconds.
    #[must_use]
    pub fn simulation_time(&self) -> f64 {
        self.runtime.time_seconds()
    }

    /// Current runtime clock state.
    #[must_use]
    pub fn runtime_state(&self) -> weaver_worldline::clock::ClockState {
        self.runtime.state()
    }

    /// Number of live entities.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Iterate over all entities and their renderable state.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &Renderable)> {
        self.entities.iter().map(|(id, r)| (*id, r))
    }

    /// Number of trajectory samples recorded for an entity.
    #[must_use]
    pub fn trajectory_history_len(&self, entity: EntityId) -> usize {
        self.runtime
            .history(entity)
            .map_or(0, weaver_worldline::history::TrajectoryHistory::len)
    }

    /// Interpolated trajectory sample for an entity at the given time.
    #[must_use]
    pub fn interpolated_trajectory(&self, entity: EntityId, time: f64) -> Option<TrajectorySample> {
        self.runtime.interpolate(entity, time).ok()
    }

    /// Most recent recorded trajectory sample for an entity.
    #[must_use]
    pub fn latest_trajectory_sample(&self, entity: EntityId) -> Option<&TrajectorySample> {
        self.runtime
            .history(entity)
            .and_then(|h| h.samples().last())
    }

    /// Access the camera.
    #[must_use]
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// Mutable access to the camera.
    #[must_use]
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Access the frame registry.
    #[must_use]
    pub fn frames(&self) -> &FrameRegistry {
        self.runtime.frames()
    }

    /// Mutable access to the frame registry.
    #[must_use]
    pub fn frames_mut(&mut self) -> &mut FrameRegistry {
        self.runtime.frames_mut()
    }

    /// Access the Woven adapter, if any.
    #[must_use]
    pub fn woven(&self) -> Option<&WovenAdapter> {
        self.woven.as_ref()
    }

    /// Mutable access to the Woven adapter, if any.
    #[must_use]
    pub fn woven_mut(&mut self) -> Option<&mut WovenAdapter> {
        self.woven.as_mut()
    }

    /// Last application payload received for each Woven channel.
    #[must_use]
    pub fn replicated_payloads(&self) -> &[PayloadEnvelope] {
        &self.replicated_payloads
    }

    /// Drain Woven entity IDs that left since the previous call.
    pub fn drain_departed_woven_entities(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.departed_woven_entities)
    }

    /// Publish a typed payload to Woven.
    ///
    /// # Errors
    ///
    /// Returns an error if the adapter is not running or publish fails.
    pub fn publish<T>(
        &mut self,
        channel: u64,
        entity: Option<u64>,
        payload: &Payload<T>,
        delivery: DeliveryClass,
        persistence: PersistenceClass,
    ) -> Result<(), AppError>
    where
        T: serde::Serialize,
    {
        if let Some(adapter) = self.woven.as_mut() {
            adapter.publish(channel, entity, payload, delivery, persistence)?;
        }
        Ok(())
    }

    fn record_samples(&mut self) {
        let time = self.runtime.time_seconds();
        for (id, renderable) in &self.entities {
            let transform = renderable
                .mesh
                .as_ref()
                .map(|m| m.transform)
                .or_else(|| renderable.sprite.as_ref().map(|s| s.transform))
                .unwrap_or_default();
            self.runtime.record_sample(
                *id,
                TrajectorySample {
                    time_seconds: time,
                    frame: renderable.frame,
                    translation: transform.translation,
                    rotation: transform.rotation,
                    scale: transform.scale,
                },
            );
        }
    }

    fn process_woven(&mut self) -> Result<(), AppError> {
        let Some(adapter) = self.woven.as_mut() else {
            return Ok(());
        };
        let envelopes = adapter.drain_envelopes()?;
        let departed_entities = adapter.drain_entity_leaves();
        for entity in departed_entities {
            self.replicated_payloads
                .retain(|payload| payload.entity != Some(entity));
            self.departed_woven_entities.push(entity);
        }
        for envelope in envelopes {
            // Keep only newer sequences per channel and Woven entity.
            if let Some(existing) = self
                .replicated_payloads
                .iter()
                .find(|p| p.channel == envelope.channel && p.entity == envelope.entity)
                && envelope.sequence <= existing.sequence
            {
                continue;
            }
            self.replicated_payloads
                .retain(|p| p.channel != envelope.channel || p.entity != envelope.entity);
            self.replicated_payloads.push(envelope);
        }
        Ok(())
    }

    /// Extract an immutable render snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be built.
    pub fn extract_snapshot(&self) -> Result<SceneSnapshot, AppError> {
        let mut snapshot = SceneSnapshot::new(self.revision);
        snapshot.camera = self.camera.clone();
        snapshot.paused = self.paused;
        snapshot.time_multiplier = self.time_multiplier;
        snapshot.show_coordinate_frames = self.config.show_coordinate_frames;
        snapshot.show_trajectory_history = self.config.show_trajectory_history;
        snapshot.background_color = self.config.background_color;
        snapshot.background_particles = starfield_particles(self.config.starfield_count);

        let time = self.runtime.time_seconds();
        for (id, renderable) in &self.entities {
            let transform = renderable
                .mesh
                .as_ref()
                .map(|m| m.transform)
                .or_else(|| renderable.sprite.as_ref().map(|s| s.transform))
                .unwrap_or_default();
            snapshot.transforms.push(TransformSample {
                entity: *id,
                frame: weaver_core::FrameId::new(renderable.frame.get()),
                translation: transform.translation,
                rotation: transform.rotation,
                scale: transform.scale,
                time_seconds: time,
                revision: self.revision,
            });
            if let Some(mesh) = &renderable.mesh {
                snapshot.meshes.push(mesh.clone());
            }
            if let Some(sprite) = &renderable.sprite {
                snapshot.sprites.push(sprite.clone());
            }
            if let Some(emitter) = &renderable.emitter {
                let particles = simulate_emitter(emitter, time);
                snapshot.particles.push((*id, emitter.clone(), particles));
            }
        }

        Ok(snapshot)
    }
}

fn simulate_emitter(emitter: &ParticleEmitter, _time: f64) -> Vec<Particle> {
    // Deterministic pseudo-random particles for visual validation.
    let count = emitter.capacity.min(64);
    let mut particles = Vec::with_capacity(count);
    for i in 0..count {
        let t = (i as f32 + 0.5) / count as f32;
        let age = t * emitter.max_lifetime;
        let life = t;
        let velocity = Vec3::new(
            (i as f32 * 0.1).sin(),
            emitter.min_velocity.y + t * (emitter.max_velocity.y - emitter.min_velocity.y),
            (i as f32 * 0.13).cos(),
        );
        let position = emitter.origin + velocity * age;
        let start: [f32; 4] = emitter.start_color;
        let end: [f32; 4] = emitter.end_color;
        let color = [
            start[0] + (end[0] - start[0]) * life,
            start[1] + (end[1] - start[1]) * life,
            start[2] + (end[2] - start[2]) * life,
            start[3] + (end[3] - start[3]) * life,
        ];
        let size = emitter.start_size + (emitter.end_size - emitter.start_size) * life;
        particles.push(Particle {
            position,
            velocity,
            age,
            lifetime: emitter.max_lifetime,
            size,
            color,
        });
    }
    particles
}

fn starfield_particles(count: usize) -> Vec<weaver_render::Particle> {
    const RADIUS: f32 = 400.0;
    let mut particles = Vec::with_capacity(count);
    for i in 0..count {
        // Fibonacci sphere distribution for even star placement.
        let y = 1.0 - (i as f32 + 0.5) * 2.0 / count.max(1) as f32;
        let theta = (i as f32) * 137.5_f32.to_radians();
        let r = (1.0 - y * y).sqrt();
        let position = glam::Vec3::new(r * theta.cos(), y, r * theta.sin()) * RADIUS;
        let brightness = 0.5 + 0.5 * ((i as f32 * 0.618).fract());
        let size = 0.4 + 1.2 * ((i as f32 * 0.37).fract());
        particles.push(weaver_render::Particle {
            position,
            velocity: glam::Vec3::ZERO,
            age: 0.0,
            lifetime: 1.0,
            size,
            color: [brightness, brightness, brightness, 1.0],
        });
    }
    particles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_extraction_without_gpu() {
        let mut world = WeaverWorld::new(WorldConfig::default()).unwrap();
        world.start();
        world.step().unwrap();
        let snapshot = world.extract_snapshot().unwrap();
        assert_eq!(snapshot.revision, world.revision());
        assert_eq!(world.entity_count(), snapshot.transforms.len());
    }

    #[test]
    fn world_revision_monotonicity() {
        let mut world = WeaverWorld::new(WorldConfig::default()).unwrap();
        world.start();
        for _ in 0..10 {
            world.step().unwrap();
        }
        assert_eq!(world.revision().get(), 10);
    }
}
