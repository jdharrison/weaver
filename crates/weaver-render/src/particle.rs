//! CPU-updated particle emitters.

use glam::Vec3;

/// A single particle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    /// World-space position.
    pub position: Vec3,
    /// Velocity in world units per second.
    pub velocity: Vec3,
    /// Current age in seconds.
    pub age: f32,
    /// Maximum lifetime in seconds.
    pub lifetime: f32,
    /// Size in world units.
    pub size: f32,
    /// RGBA color.
    pub color: [f32; 4],
}

/// A CPU-driven particle emitter rendered via instancing.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEmitter {
    /// Local-space origin of the emitter.
    pub origin: Vec3,
    /// Maximum number of live particles.
    pub capacity: usize,
    /// Emission rate in particles per second.
    pub emission_rate: f32,
    /// Minimum particle lifetime.
    pub min_lifetime: f32,
    /// Maximum particle lifetime.
    pub max_lifetime: f32,
    /// Minimum initial velocity.
    pub min_velocity: Vec3,
    /// Maximum initial velocity.
    pub max_velocity: Vec3,
    /// Start color.
    pub start_color: [f32; 4],
    /// End color.
    pub end_color: [f32; 4],
    /// Start size.
    pub start_size: f32,
    /// End size.
    pub end_size: f32,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            capacity: 256,
            emission_rate: 64.0,
            min_lifetime: 0.5,
            max_lifetime: 2.0,
            min_velocity: Vec3::new(-0.5, 0.5, -0.5),
            max_velocity: Vec3::new(0.5, 1.5, 0.5),
            start_color: [1.0, 0.8, 0.2, 1.0],
            end_color: [1.0, 0.2, 0.0, 0.0],
            start_size: 0.15,
            end_size: 0.05,
        }
    }
}
