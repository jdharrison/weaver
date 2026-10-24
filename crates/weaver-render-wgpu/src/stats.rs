//! CPU encoding statistics, not GPU completion or rasterized primitive counts.

use std::time::Duration;

/// Last successfully encoded frame. Failed renders leave the previous value intact.
/// A frame may subsequently be discarded rather than presented.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    /// World instances supplied by the snapshot, including missing mesh handles.
    pub world_mesh_instances_submitted: u64,
    /// Instances retained by culling (including bypasses and missing handles).
    pub world_mesh_instances_visible: u64,
    /// Instances rejected before grouping. Submitted = visible + culled.
    pub world_mesh_instances_culled: u64,
    /// Actual indexed world mesh draw commands, including instancing/skin splits.
    pub world_mesh_draws: u64,
    /// Sum of `(index_count / 3) * instance_count` for encoded world draws.
    pub world_mesh_triangles: u64,
    /// Actual indexed foreground mesh draw commands (foreground is never culled).
    pub foreground_mesh_draws: u64,
    /// Sum of `(index_count / 3) * instance_count` for encoded foreground draws.
    pub foreground_mesh_triangles: u64,
    /// CPU wall time in render, including acquisition/encoding; excludes present/GPU work.
    pub cpu_render_time: Duration,
    /// CPU subset for world culling and world/foreground grouping/resource resolution.
    pub cpu_mesh_prepare_time: Duration,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MeshDrawStats {
    pub draws: u64,
    pub triangles: u64,
}

impl MeshDrawStats {
    pub fn record(&mut self, indices: u32, instances: usize) {
        self.draws += 1;
        self.triangles += u64::from(indices / 3) * instances as u64;
    }
}

// std::time::Instant is not supported on wasm32-unknown-unknown. Date's clock can
// move backwards; clamp that case rather than panicking or reporting a huge span.
pub(crate) struct CpuTimer {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
    #[cfg(target_arch = "wasm32")]
    start: f64,
}
impl CpuTimer {
    pub fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
            #[cfg(target_arch = "wasm32")]
            start: js_sys::Date::now(),
        }
    }
    pub fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::from_secs_f64(((js_sys::Date::now() - self.start) / 1000.0).max(0.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_commands_and_instanced_triangles_not_vertices() {
        let mut stats = MeshDrawStats::default();
        stats.record(36, 128);
        stats.record(36, 1);
        stats.record(7, 3);
        assert_eq!(stats.draws, 3);
        assert_eq!(stats.triangles, 12 * 129 + 6);
        assert_eq!(MeshDrawStats::default().draws, 0);
    }
}
