//! Debug overlay and command menu.

use crate::world::WeaverWorld;
use glam::Vec2;
use std::time::Instant;
use sysinfo::{MemoryRefreshKind, System};
use weaver_render::{SceneSnapshot, TextAnchor, TextRun, UiElement, UiRect};

/// Action triggered by a side-menu button.
#[derive(Clone, Copy, Debug)]
pub enum MenuAction {
    /// Focus the camera on the given entity.
    FocusEntity(weaver_core::EntityId),
}

/// A single item in the right-side focus menu.
#[derive(Clone, Debug)]
pub struct MenuItem {
    /// Label shown on the button.
    pub label: String,
    /// Action performed when the button is clicked.
    pub action: MenuAction,
}

/// Rolling-window performance metrics for the debug overlay.
pub struct DebugOverlay {
    frame_times: Vec<f64>,
    last_frame: Option<Instant>,
    sys: System,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugOverlay {
    /// Create a new overlay.
    pub fn new() -> Self {
        Self {
            frame_times: Vec::with_capacity(128),
            last_frame: None,
            sys: System::new(),
        }
    }

    /// Record the start of a frame and update metrics from the previous frame.
    pub fn mark_frame(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            let elapsed = now.duration_since(last).as_secs_f64();
            if self.frame_times.len() == self.frame_times.capacity() {
                self.frame_times.remove(0);
            }
            self.frame_times.push(elapsed);
        }
        self.last_frame = Some(now);
    }

    /// Average frame time over the last few frames.
    #[must_use]
    pub fn average_frame_time(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        self.frame_times.iter().sum::<f64>() / self.frame_times.len() as f64
    }

    /// Frames per second from the average frame time.
    #[must_use]
    pub fn fps(&self) -> f64 {
        let avg = self.average_frame_time();
        if avg <= 0.0 {
            return 0.0;
        }
        1.0 / avg
    }

    /// Current system memory usage in megabytes.
    #[must_use]
    pub fn memory_mb(&mut self) -> u64 {
        self.sys
            .refresh_memory_specifics(MemoryRefreshKind::nothing());
        self.sys.used_memory() / 1024
    }

    /// Estimated end-to-end latency for the last frame, in milliseconds.
    #[must_use]
    pub fn latency_ms(&self) -> f64 {
        self.average_frame_time() * 1000.0
    }

    /// Format a compact status string.
    #[must_use]
    pub fn status_text(&mut self) -> String {
        format!(
            "{:.0} FPS | {:.1} ms | {:.0} MB",
            self.fps(),
            self.latency_ms(),
            self.memory_mb()
        )
    }
}

/// Toggleable debug command/logger menu.
pub struct DebugMenu {
    overlay: DebugOverlay,
    open: bool,
}

impl Default for DebugMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugMenu {
    /// Create a new closed debug menu.
    pub fn new() -> Self {
        Self {
            overlay: DebugOverlay::new(),
            open: false,
        }
    }

    /// Toggle the menu open/closed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Whether the menu is currently open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Record the start of a frame.
    pub fn mark_frame(&mut self) {
        self.overlay.mark_frame();
    }

    /// Always-visible top-right overlay showing time coordinate and performance.
    pub fn push_overlay(
        &mut self,
        snapshot: &mut SceneSnapshot,
        screen_width: u32,
        simulation_time: f64,
    ) {
        snapshot.text.push(TextRun {
            text: format!("T+{:.2}s | {}", simulation_time, self.overlay.status_text()),
            position: glam::Vec2::new(screen_width as f32 - 10.0, 10.0),
            size: 14.0,
            color: [0.2, 1.0, 0.4, 1.0],
            anchor: TextAnchor::TopRight,
            bounds: None,
            world_space: false,
            world_height: 1.0,
        });
    }

    /// Full-width top bar shown when the menu is open. Displays the
    /// `debuginfo` command output: system services and performance.
    pub fn push_top_bar(
        &mut self,
        snapshot: &mut SceneSnapshot,
        world: &WeaverWorld,
        screen_width: u32,
        format_time: Option<&dyn Fn(f64) -> String>,
    ) {
        if !self.open {
            return;
        }

        let height = 28.0f32;
        snapshot.ui.push(UiElement::Rect(UiRect {
            position: glam::Vec2::new(0.0, 0.0),
            size: glam::Vec2::new(screen_width as f32, height),
            background: [0.08, 0.08, 0.12, 0.92],
            border_color: [0.25, 0.25, 0.35, 1.0],
            border_width: 0.0,
            corner_radius: 0.0,
            anchor: TextAnchor::TopLeft,
            layer: 100,
            interactive: false,
        }));

        let services = format!(
            "Worldline: {} | Signalweave: {} | Entities: {} | Revision: {}",
            worldline_status(world),
            signalweave_status(world),
            world.entity_count(),
            world.revision().get()
        );
        let time_text = format_time.map_or_else(
            || format!("{:.2}s", world.simulation_time()),
            |f| f(world.simulation_time()),
        );
        let performance = format!(
            "Time: {time_text} | {:.0} FPS | {:.1} ms | {:.0} MB",
            self.overlay.fps(),
            self.overlay.latency_ms(),
            self.overlay.memory_mb()
        );

        snapshot.text.push(TextRun {
            text: format!("debuginfo> {services}"),
            position: glam::Vec2::new(10.0, 6.0),
            size: 13.0,
            color: [0.9, 0.9, 0.9, 1.0],
            anchor: TextAnchor::TopLeft,
            bounds: None,
            world_space: false,
            world_height: 1.0,
        });
        snapshot.text.push(TextRun {
            text: performance,
            position: glam::Vec2::new(screen_width as f32 - 10.0, 6.0),
            size: 13.0,
            color: [0.2, 1.0, 0.4, 1.0],
            anchor: TextAnchor::TopRight,
            bounds: None,
            world_space: false,
            world_height: 1.0,
        });
    }

    /// Render a right-anchored focus menu and return the screen-space hit
    /// regions for each interactive button.
    pub fn push_side_menu(
        &self,
        snapshot: &mut SceneSnapshot,
        screen_width: u32,
        items: &[MenuItem],
    ) -> Vec<(MenuAction, Vec2, Vec2)> {
        if items.is_empty() {
            return Vec::new();
        }

        let panel_width = 140.0f32;
        let button_height = 26.0f32;
        let padding = 8.0f32;
        let gap = 4.0f32;
        let margin = 10.0f32;
        let panel_x = screen_width as f32 - panel_width - margin;
        let panel_y = 40.0f32;
        let panel_height = padding * 2.0
            + items.len() as f32 * button_height
            + (items.len().saturating_sub(1)) as f32 * gap;

        snapshot.ui.push(UiElement::Rect(UiRect {
            position: Vec2::new(panel_x, panel_y),
            size: Vec2::new(panel_width, panel_height),
            background: [0.06, 0.06, 0.09, 0.92],
            border_color: [0.25, 0.25, 0.35, 1.0],
            border_width: 1.0,
            corner_radius: 6.0,
            anchor: TextAnchor::TopLeft,
            layer: 100,
            interactive: false,
        }));

        let mut buttons = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let bx = panel_x + padding;
            let by = panel_y + padding + i as f32 * (button_height + gap);
            let bw = panel_width - padding * 2.0;
            snapshot.ui.push(UiElement::Rect(UiRect {
                position: Vec2::new(bx, by),
                size: Vec2::new(bw, button_height),
                background: [0.12, 0.12, 0.18, 0.95],
                border_color: [0.35, 0.35, 0.45, 1.0],
                border_width: 1.0,
                corner_radius: 4.0,
                anchor: TextAnchor::TopLeft,
                layer: 101,
                interactive: true,
            }));
            snapshot.text.push(TextRun {
                text: item.label.clone(),
                position: Vec2::new(bx + bw / 2.0, by + button_height / 2.0 - 4.0),
                size: 12.0,
                color: [0.9, 0.9, 0.9, 1.0],
                anchor: TextAnchor::Center,
                bounds: None,
                world_space: false,
                world_height: 1.0,
            });
            buttons.push((
                item.action,
                Vec2::new(bx, by),
                Vec2::new(bx + bw, by + button_height),
            ));
        }
        buttons
    }

    /// Optional hover tooltip rendered near the mouse cursor.
    ///
    /// `size` is the inner content size in pixels; padding is added around the
    /// text and the background rect is sized to fit.
    pub fn push_tooltip(
        &self,
        snapshot: &mut SceneSnapshot,
        mouse: (f32, f32),
        text: &str,
        size: glam::Vec2,
    ) {
        if text.is_empty() {
            return;
        }
        let padding = glam::Vec2::new(12.0, 8.0);
        let rect_size = size + padding * 2.0;
        let text_pos = glam::Vec2::new(mouse.0 + 12.0, mouse.1 + 12.0);
        snapshot.ui.push(UiElement::Rect(UiRect {
            position: text_pos - padding,
            size: rect_size,
            background: [0.05, 0.05, 0.08, 0.95],
            border_color: [0.3, 0.3, 0.4, 1.0],
            border_width: 1.0,
            corner_radius: 4.0,
            anchor: TextAnchor::TopLeft,
            layer: 101,
            interactive: false,
        }));
        snapshot.text.push(TextRun {
            text: text.to_string(),
            position: text_pos,
            size: 12.0,
            color: [0.9, 0.9, 0.9, 1.0],
            anchor: TextAnchor::TopLeft,
            bounds: None,
            world_space: false,
            world_height: 1.0,
        });
    }
}

fn worldline_status(world: &WeaverWorld) -> &'static str {
    use weaver_worldline::clock::ClockState;
    match world.runtime_state() {
        ClockState::Running => "running",
        ClockState::Paused => "paused",
        ClockState::Stopped => "stopped",
        ClockState::Error => "error",
    }
}

fn signalweave_status(world: &WeaverWorld) -> String {
    world
        .signalweave()
        .map_or_else(|| "disabled".to_string(), |a| format!("{:?}", a.status()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_computed_from_average_frame_time() {
        let mut menu = DebugMenu::new();
        menu.overlay.frame_times = vec![0.016, 0.017, 0.015];
        menu.overlay.last_frame = Some(Instant::now());
        assert!((menu.overlay.fps() - 62.5).abs() < 1.0);
    }

    #[test]
    fn empty_overlay_returns_zero() {
        let menu = DebugMenu::new();
        assert_eq!(menu.overlay.fps(), 0.0);
        assert_eq!(menu.overlay.latency_ms(), 0.0);
        assert_eq!(menu.overlay.average_frame_time(), 0.0);
    }

    #[test]
    fn menu_toggles() {
        let mut menu = DebugMenu::new();
        assert!(!menu.is_open());
        menu.toggle();
        assert!(menu.is_open());
        menu.toggle();
        assert!(!menu.is_open());
    }
}
