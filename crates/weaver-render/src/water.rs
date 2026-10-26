//! Stylized water material settings.

use glam::Vec3;

/// Per-frame water appearance for `MeshRenderMode::Water`.
///
/// The renderer uses these colors and wave parameters in a fixed GPU water
/// shader. This is a presentation approximation, not a fluid simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterSettings {
    /// Deep-water color.
    pub deep_color: Vec3,
    /// Shallow/crest color.
    pub shallow_color: Vec3,
    /// Sky color near the horizon, used for Fresnel reflection.
    pub sky_horizon: Vec3,
    /// Sky color overhead, used for Fresnel reflection.
    pub sky_zenith: Vec3,
    /// Direction from the surface toward the sun. The renderer normalizes it.
    pub sun_direction: Vec3,
    /// Sun/specular tint.
    pub sun_color: Vec3,
    /// Specular highlight strength.
    pub specular_strength: f32,
    /// Horizontal wave frequency multiplier.
    pub wave_scale: f32,
    /// Vertical wave displacement in metres.
    pub wave_height: f32,
    /// Whitecap/crest foam strength.
    pub foam_strength: f32,
    /// Scrolling detail-texture contrast.
    pub detail_strength: f32,
    /// Normal steepness multiplier for specular/fresnel shading.
    pub normal_strength: f32,
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            deep_color: Vec3::new(0.035, 0.122, 0.161),
            shallow_color: Vec3::new(0.090, 0.243, 0.282),
            sky_horizon: Vec3::new(0.65, 0.80, 0.88),
            sky_zenith: Vec3::new(0.12, 0.36, 0.66),
            sun_direction: Vec3::new(-0.45, 0.78, -0.44),
            sun_color: Vec3::new(1.0, 0.97, 0.90),
            specular_strength: 1.2,
            wave_scale: 1.0,
            wave_height: 0.55,
            foam_strength: 0.35,
            detail_strength: 0.35,
            normal_strength: 2.5,
        }
    }
}

impl WaterSettings {
    /// Whether every numeric input is finite and the sun direction is usable.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [
            self.deep_color,
            self.shallow_color,
            self.sky_horizon,
            self.sky_zenith,
            self.sun_direction,
            self.sun_color,
        ]
        .iter()
        .all(|color| color.is_finite())
            && self.sun_direction.length_squared() > 1.0e-6
            && [
                self.specular_strength,
                self.wave_scale,
                self.wave_height,
                self.foam_strength,
                self.detail_strength,
                self.normal_strength,
            ]
            .iter()
            .all(|value| value.is_finite())
    }
}
