//! Fixed-size, opt-in CPU wall-time summaries; no GPU timestamps.

use std::fmt;
use std::time::{Duration, Instant};
use weaver_render_wgpu::FrameStats;

#[derive(Default)]
struct Metric {
    sum: f64,
    max: f64,
    count: u64,
}

impl Metric {
    fn record(&mut self, value: f64) {
        self.sum += value;
        self.max = self.max.max(value);
        self.count = self.count.saturating_add(1);
    }

    fn average(&self) -> f64 {
        self.sum / self.count.max(1) as f64
    }
}

#[derive(Default)]
pub(super) struct Summary {
    timings: [Metric; 5],
    meshes: [Metric; 7],
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[performance] frames={} CPU wall ms avg/max (includes waits/vsync; not GPU timestamps):",
            self.timings[0].count
        )?;
        for (name, metric) in [
            "app_update",
            "render_acquire_encode",
            "mesh_prepare_subset",
            "present",
            "frame_interval",
        ]
        .into_iter()
        .zip(&self.timings)
        {
            write!(f, " {name}={:.3}/{:.3}", metric.average(), metric.max)?;
        }
        write!(f, "; mesh counts avg:")?;
        for (name, metric) in [
            "world_submitted",
            "world_visible",
            "world_culled",
            "world_draws",
            "world_triangles",
            "foreground_draws",
            "foreground_triangles",
        ]
        .into_iter()
        .zip(&self.meshes)
        {
            write!(f, " {name}={:.1}", metric.average())?;
        }
        Ok(())
    }
}

pub(super) struct PerformanceLog {
    enabled: bool,
    window_start: Option<Instant>,
    previous_redraw: Option<Instant>,
    interval: Option<Duration>,
    summary: Summary,
}

impl PerformanceLog {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            window_start: None,
            previous_redraw: None,
            interval: None,
            summary: Summary::default(),
        }
    }

    pub(super) fn begin_frame(&mut self, now: Instant) {
        if self.enabled {
            self.window_start.get_or_insert(now);
            self.interval = self
                .previous_redraw
                .replace(now)
                .map(|last| now.duration_since(last));
        }
    }

    // Called only after successful render/present: failed renders retain stale renderer stats.
    pub(super) fn record(
        &mut self,
        now: Instant,
        update: Duration,
        present: Duration,
        stats: &FrameStats,
    ) -> Option<Summary> {
        if !self.enabled {
            return None;
        }
        let start = *self.window_start.get_or_insert(now);
        for (metric, duration) in self.summary.timings.iter_mut().zip([
            Some(update),
            Some(stats.cpu_render_time),
            Some(stats.cpu_mesh_prepare_time),
            Some(present),
            self.interval,
        ]) {
            if let Some(duration) = duration {
                metric.record(duration.as_secs_f64() * 1000.0);
            }
        }
        for (metric, value) in self.summary.meshes.iter_mut().zip([
            stats.world_mesh_instances_submitted,
            stats.world_mesh_instances_visible,
            stats.world_mesh_instances_culled,
            stats.world_mesh_draws,
            stats.world_mesh_triangles,
            stats.foreground_mesh_draws,
            stats.foreground_mesh_triangles,
        ]) {
            metric.record(value as f64);
        }
        if now.duration_since(start) < Duration::from_secs(1) {
            return None;
        }
        self.window_start = Some(now);
        Some(std::mem::take(&mut self.summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_gate_keeps_state_empty() {
        let mut log = PerformanceLog::new(false);
        let now = Instant::now();
        log.begin_frame(now);
        assert!(
            log.record(
                now + Duration::from_secs(2),
                Duration::ZERO,
                Duration::ZERO,
                &FrameStats::default()
            )
            .is_none()
        );
        assert!(log.window_start.is_none());
        assert!(log.previous_redraw.is_none());
        assert_eq!(log.summary.timings[0].count, 0);
    }

    #[test]
    fn averages_maxima_cadence_and_reset() {
        let mut log = PerformanceLog::new(true);
        let now = Instant::now();
        let stats = FrameStats {
            world_mesh_instances_submitted: 10,
            world_mesh_instances_visible: 6,
            world_mesh_instances_culled: 4,
            world_mesh_draws: 2,
            world_mesh_triangles: 30,
            foreground_mesh_draws: 1,
            foreground_mesh_triangles: 5,
            cpu_render_time: Duration::from_millis(8),
            cpu_mesh_prepare_time: Duration::from_millis(2),
        };
        log.begin_frame(now);
        assert!(
            log.record(
                now,
                Duration::from_millis(2),
                Duration::from_millis(3),
                &stats
            )
            .is_none()
        );
        log.begin_frame(now + Duration::from_millis(999));
        assert!(
            log.record(
                now + Duration::from_millis(999),
                Duration::from_millis(4),
                Duration::from_millis(5),
                &stats
            )
            .is_none()
        );
        log.begin_frame(now + Duration::from_secs(1));
        let report = log
            .record(
                now + Duration::from_secs(1),
                Duration::from_millis(6),
                Duration::from_millis(7),
                &stats,
            )
            .unwrap();
        assert_eq!(report.timings[0].average(), 4.0);
        assert_eq!(report.timings[0].max, 6.0);
        assert_eq!(report.timings[1].average(), 8.0);
        assert_eq!(report.timings[2].average(), 2.0);
        assert_eq!(report.timings[3].average(), 5.0);
        assert_eq!(report.timings[4].average(), 500.0);
        assert_eq!(report.timings[4].max, 999.0); // Not the app's clamped delta.
        assert_eq!(report.timings[4].count, 2); // No invented first-frame interval.
        for (metric, expected) in report.meshes.iter().zip([10., 6., 4., 2., 30., 1., 5.]) {
            assert_eq!(metric.average(), expected);
        }
        assert_eq!(log.summary.timings[0].count, 0);
        log.begin_frame(now + Duration::from_secs(5));
        let report = log
            .record(
                now + Duration::from_secs(5),
                Duration::from_millis(1),
                Duration::ZERO,
                &FrameStats::default(),
            )
            .unwrap();
        assert_eq!(report.timings[0].average(), 1.0);
        assert_eq!(report.timings[0].max, 1.0);
        assert_eq!(report.meshes[0].average(), 0.0);
        // A long pause emits one summary, not a backlog of one-second reports.
        assert!(
            log.record(
                now + Duration::from_secs(5),
                Duration::ZERO,
                Duration::ZERO,
                &stats
            )
            .is_none()
        );
    }
}
