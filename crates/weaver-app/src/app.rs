//! Windowed application runner using winit.

use crate::debug::{DebugMenu, MenuAction, MenuItem};
use crate::error::AppError;
use crate::event::InputEvent;
use crate::world::{WeaverWorld, WorldConfig};
use glam::{Vec3, Vec4, Vec4Swizzles};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent as WinitWindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// Callback signature for populating the world and uploading GPU resources.
pub type SetupFn = Box<dyn FnOnce(&mut WeaverWorld, &mut weaver_render_wgpu::WgpuRenderer)>;

/// Callback signature for per-frame simulation updates.
pub type UpdateFn = Box<dyn FnMut(&mut WeaverWorld, f64)>;

/// Callback signature for formatting a hover tooltip for an entity.
pub type TooltipFn = Box<dyn Fn(&WeaverWorld, weaver_core::EntityId, f64) -> String>;

/// Callback signature for formatting the simulation time shown in the top bar.
pub type TimeFormatterFn = Box<dyn Fn(f64) -> String>;

/// Callback for an application-specific, low-frequency window-title status.
pub type TitleStatusFn = Box<dyn Fn() -> String>;

/// Callback signature for building the right-side focus menu after setup.
pub type SideMenuFn = Box<dyn FnOnce(&WeaverWorld) -> Vec<MenuItem>>;

/// Prevent an unexpectedly long stall from monopolizing the event loop while
/// fixed-rate simulation time catches up to elapsed wall time.
const MAX_SIMULATION_CATCH_UP_STEPS: u32 = 8;

/// Configuration for the windowed application.
pub struct ApplicationConfig {
    /// Window title.
    pub title: String,
    /// Initial window width.
    pub width: u32,
    /// Initial window height.
    pub height: u32,
    /// World configuration.
    pub world: WorldConfig,
    /// Optional scene setup callback invoked after world/renderer creation.
    pub setup: Option<SetupFn>,
    /// Optional per-frame update callback invoked after the simulation step.
    pub update: Option<UpdateFn>,
    /// Optional hover tooltip formatter. Receives the world, hovered entity id,
    /// and current simulation time. Returns the tooltip text (may be empty).
    pub tooltip: Option<TooltipFn>,
    /// Optional formatter for the simulation time displayed in the debug top bar.
    pub format_time: Option<TimeFormatterFn>,
    /// Optional application-specific status appended to the window title at a
    /// low (once per second) cadence.
    pub title_status: Option<TitleStatusFn>,
    /// Optional right-side focus menu builder, invoked after setup so it can
    /// reference spawned entity ids.
    pub side_menu: Option<SideMenuFn>,
    /// Surface present mode. Use `AutoNoVsync` or `Immediate` to uncap FPS.
    pub present_mode: wgpu::PresentMode,
}

impl std::fmt::Debug for ApplicationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationConfig")
            .field("title", &self.title)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("world", &self.world)
            .field("setup", &self.setup.is_some())
            .field("update", &self.update.is_some())
            .field("tooltip", &self.tooltip.is_some())
            .field("format_time", &self.format_time.is_some())
            .field("title_status", &self.title_status.is_some())
            .field("side_menu", &self.side_menu.is_some())
            .field("present_mode", &self.present_mode)
            .finish()
    }
}

impl Default for ApplicationConfig {
    fn default() -> Self {
        Self {
            title: "Weaver".to_string(),
            width: 1280,
            height: 720,
            world: WorldConfig::default(),
            setup: None,
            update: None,
            tooltip: None,
            format_time: None,
            title_status: None,
            side_menu: None,
            present_mode: wgpu::PresentMode::AutoVsync,
        }
    }
}

/// A windowed Weaver application.
pub struct Application {
    config: ApplicationConfig,
    window: Option<Arc<Window>>,
    renderer: Option<weaver_render_wgpu::WgpuRenderer>,
    world: Option<WeaverWorld>,
    debug_menu: DebugMenu,
    update: Option<UpdateFn>,
    title_status: Option<TitleStatusFn>,
    side_menu: Vec<MenuItem>,
    side_menu_builder: Option<SideMenuFn>,
    side_menu_buttons: Vec<(MenuAction, glam::Vec2, glam::Vec2)>,
    mouse_position: Option<(f32, f32)>,
    last_mouse: Option<(f32, f32)>,
    last_step: Option<Instant>,
    last_title_update: Option<Instant>,
    step_interval: Duration,
    running: bool,
    origin_entity: Option<weaver_core::EntityId>,
    camera_distance: f32,
    camera_azimuth: f32,
    camera_elevation: f32,
    rotating: bool,
    drag_start: Option<(f32, f32)>,
}

impl Application {
    /// Create a new application from configuration.
    #[must_use]
    pub fn new(mut config: ApplicationConfig) -> Self {
        let step_interval =
            Duration::from_secs_f64(1.0 / f64::from(config.world.clock.steps_per_second.max(1)));
        Self {
            update: config.update.take(),
            title_status: config.title_status.take(),
            // Performance-focused native applications use the title for
            // diagnostics; tooltip callbacks are deliberately not installed.
            side_menu: Vec::new(),
            side_menu_builder: config.side_menu.take(),
            side_menu_buttons: Vec::new(),
            config,
            window: None,
            renderer: None,
            world: None,
            debug_menu: DebugMenu::new(),
            mouse_position: None,
            last_mouse: None,
            last_step: None,
            last_title_update: None,
            step_interval,
            running: true,
            origin_entity: None,
            camera_distance: 50.0,
            camera_azimuth: 0.0,
            camera_elevation: 0.5,
            rotating: false,
            drag_start: None,
        }
    }

    /// Run the application until the window closes.
    ///
    /// # Errors
    ///
    /// Returns an error if the event loop or renderer fails.
    pub fn run(mut self) -> Result<(), AppError> {
        let event_loop = EventLoop::new().map_err(|err| AppError::Window(err.to_string()))?;
        event_loop
            .run_app(&mut self)
            .map_err(|err| AppError::Window(err.to_string()))?;
        Ok(())
    }
}

impl ApplicationHandler for Application {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.width,
                self.config.height,
            ));
        let window = match event_loop.create_window(window_attributes) {
            Ok(w) => Arc::new(w),
            Err(err) => {
                tracing::error!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };
        center_window_on_primary_monitor(&window);

        let mut renderer = match pollster::block_on(weaver_render_wgpu::WgpuRenderer::from_window(
            window.clone(),
            weaver_render_wgpu::WgpuContextConfig {
                present_mode: self.config.present_mode,
                ..weaver_render_wgpu::WgpuContextConfig::default()
            },
        )) {
            Ok(r) => r,
            Err(err) => {
                tracing::error!("failed to create renderer: {err}");
                event_loop.exit();
                return;
            }
        };

        let mut world = match WeaverWorld::new(self.config.world.clone()) {
            Ok(w) => w,
            Err(err) => {
                tracing::error!("failed to create world: {err}");
                event_loop.exit();
                return;
            }
        };

        if let Some(setup) = self.config.setup.take() {
            setup(&mut world, &mut renderer);
        }
        world.start();

        if let Some(builder) = self.side_menu_builder.take() {
            self.side_menu = builder(&world);
        }

        let (distance, azimuth, elevation) = spherical_from_camera(world.camera());
        self.camera_distance = distance;
        self.camera_azimuth = azimuth;
        self.camera_elevation = elevation.clamp(-1.55, 1.55);

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.world = Some(world);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WinitWindowEvent,
    ) {
        let Some(world) = self.world.as_mut() else {
            return;
        };

        match event {
            WinitWindowEvent::CloseRequested
            | WinitWindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                self.running = false;
                event_loop.exit();
            }
            WinitWindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                world.handle_input(InputEvent::Resized {
                    width: size.width,
                    height: size.height,
                });
            }
            WinitWindowEvent::CursorMoved { position, .. } => {
                self.mouse_position = Some((position.x as f32, position.y as f32));
                if self.rotating {
                    if let Some(last) = self.last_mouse {
                        let dx = position.x as f32 - last.0;
                        let dy = position.y as f32 - last.1;
                        self.camera_azimuth -= dx * 0.005;
                        self.camera_elevation -= dy * 0.005;
                        self.camera_elevation = self.camera_elevation.clamp(-1.55, 1.55);
                        update_camera_target(
                            world,
                            self.origin_entity,
                            self.camera_distance,
                            self.camera_azimuth,
                            self.camera_elevation,
                        );
                    }
                    self.last_mouse = self.mouse_position;
                }
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                let zoom = match delta {
                    MouseScrollDelta::LineDelta(_, y) => 1.0 - y * 0.1,
                    MouseScrollDelta::PixelDelta(pos) => 1.0 - pos.y as f32 * 0.002,
                };
                self.camera_distance *= zoom.clamp(0.5, 2.0);
                self.camera_distance = self.camera_distance.clamp(1.0, 10_000.0);
                update_camera_target(
                    world,
                    self.origin_entity,
                    self.camera_distance,
                    self.camera_azimuth,
                    self.camera_elevation,
                );
            }
            WinitWindowEvent::MouseInput { button, state, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    self.rotating = true;
                    self.last_mouse = self.mouse_position;
                    self.drag_start = self.mouse_position;
                }
                (MouseButton::Left, ElementState::Released) => {
                    self.rotating = false;
                    self.last_mouse = None;
                    if let (Some(start), Some(mouse)) = (self.drag_start, self.mouse_position) {
                        let dx = mouse.0 - start.0;
                        let dy = mouse.1 - start.1;
                        if dx * dx + dy * dy < 9.0 {
                            // Treat as a click rather than a drag.
                            if let Some((action, _min, _max)) =
                                self.side_menu_buttons.iter().find(|(_, min, max)| {
                                    mouse.0 >= min.x
                                        && mouse.0 <= max.x
                                        && mouse.1 >= min.y
                                        && mouse.1 <= max.y
                                })
                            {
                                let MenuAction::FocusEntity(id) = action;
                                self.origin_entity = Some(*id);
                                update_camera_target(
                                    world,
                                    self.origin_entity,
                                    self.camera_distance,
                                    self.camera_azimuth,
                                    self.camera_elevation,
                                );
                            } else {
                                let (width, height) = self
                                    .renderer
                                    .as_ref()
                                    .map_or((1280, 720), |r| r.context().size);
                                if let Some(id) = hovered_entity(world, mouse, (width, height)) {
                                    self.origin_entity = Some(id);
                                    update_camera_target(
                                        world,
                                        self.origin_entity,
                                        self.camera_distance,
                                        self.camera_azimuth,
                                        self.camera_elevation,
                                    );
                                }
                            }
                        }
                    }
                    self.drag_start = None;
                }
                _ => {}
            },
            WinitWindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                let input = match logical_key.as_ref() {
                    Key::Named(NamedKey::Space) => Some(InputEvent::TogglePause),
                    Key::Named(NamedKey::F1) => Some(InputEvent::ToggleCoordinateFrames),
                    Key::Named(NamedKey::F2) => Some(InputEvent::ToggleTrajectoryHistory),
                    Key::Character("1") => Some(InputEvent::SetTimeMultiplier(0.5)),
                    Key::Character("2") => Some(InputEvent::SetTimeMultiplier(1.0)),
                    Key::Character("3") => Some(InputEvent::SetTimeMultiplier(2.0)),
                    _ => None,
                };
                if let Some(input) = input {
                    world.handle_input(input);
                }

                match logical_key.as_ref() {
                    Key::Character("+" | "=") => {
                        self.camera_distance *= 0.9;
                        update_camera_target(
                            world,
                            self.origin_entity,
                            self.camera_distance,
                            self.camera_azimuth,
                            self.camera_elevation,
                        );
                    }
                    Key::Character("-") => {
                        self.camera_distance *= 1.1;
                        update_camera_target(
                            world,
                            self.origin_entity,
                            self.camera_distance,
                            self.camera_azimuth,
                            self.camera_elevation,
                        );
                    }
                    _ => {}
                }
            }
            WinitWindowEvent::RedrawRequested => {
                let now = Instant::now();
                let mut steps_this_frame = 0;
                loop {
                    let due = self
                        .last_step
                        .is_none_or(|last| now.duration_since(last) >= self.step_interval);
                    if !due || steps_this_frame >= MAX_SIMULATION_CATCH_UP_STEPS {
                        break;
                    }
                    if let Err(err) = world.step() {
                        tracing::error!("simulation step failed: {err}");
                        break;
                    }
                    self.last_step =
                        Some(self.last_step.map_or(now, |last| last + self.step_interval));
                    steps_this_frame += 1;
                }
                if steps_this_frame == MAX_SIMULATION_CATCH_UP_STEPS
                    && self
                        .last_step
                        .is_some_and(|last| now.duration_since(last) >= self.step_interval)
                {
                    tracing::warn!(
                        "simulation is behind its bounded catch-up limit; rendering cannot keep pace"
                    );
                }
                let time = world.simulation_time();
                if let Some(update) = self.update.as_mut() {
                    update(world, time);
                }
                update_camera_target(
                    world,
                    self.origin_entity,
                    self.camera_distance,
                    self.camera_azimuth,
                    self.camera_elevation,
                );
                if let Some(renderer) = self.renderer.as_mut() {
                    match world.extract_snapshot() {
                        Ok(snapshot) => {
                            // Native performance mode intentionally submits no
                            // debug/menu/tooltip UI. Window-title metrics are
                            // rate-limited and kept outside the render scene.
                            self.side_menu_buttons.clear();
                            match renderer.render(&snapshot) {
                                Ok(frame) => renderer.present(frame),
                                Err(err) => tracing::error!("render failed: {err}"),
                            }
                        }
                        Err(err) => tracing::error!("snapshot extraction failed: {err}"),
                    }
                }
                self.debug_menu.mark_frame();
                if self
                    .last_title_update
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
                {
                    if let Some(window) = self.window.as_ref() {
                        let title = match self.title_status.as_ref() {
                            Some(status) => format!(
                                "{} | {} | {}",
                                self.config.title,
                                status(),
                                self.debug_menu.title_status()
                            ),
                            None => format!(
                                "{} | {}",
                                self.config.title,
                                self.debug_menu.title_status()
                            ),
                        };
                        window.set_title(&title);
                    }
                    self.last_title_update = Some(now);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn hovered_entity(
    world: &WeaverWorld,
    mouse: (f32, f32),
    screen: (u32, u32),
) -> Option<weaver_core::EntityId> {
    let camera = world.camera();
    let aspect = screen.0 as f32 / screen.1.max(1) as f32;
    let (origin, dir) = ray_from_mouse(camera, aspect, mouse, (screen.0 as f32, screen.1 as f32));

    let mut closest: Option<(weaver_core::EntityId, f32)> = None;
    for (id, renderable) in world.iter() {
        let transform = renderable
            .mesh
            .as_ref()
            .map(|m| m.transform)
            .or_else(|| renderable.sprite.as_ref().map(|s| s.transform))
            .unwrap_or_default();
        let radius = renderable
            .mesh
            .as_ref()
            .map(|m| m.transform.scale)
            .or_else(|| renderable.sprite.as_ref().map(|s| s.transform.scale))
            .unwrap_or(0.5);
        if let Some(t) = ray_sphere_intersect(origin, dir, transform.translation, radius)
            && closest.is_none_or(|(_, best)| t < best)
        {
            closest = Some((id, t));
        }
    }
    closest.map(|(id, _)| id)
}

fn center_window_on_primary_monitor(window: &Window) {
    let Some(monitor) = window.primary_monitor() else {
        return;
    };
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();
    let window_size = window.outer_size();
    let x = monitor_pos.x + (monitor_size.width as i32 - window_size.width as i32) / 2;
    let y = monitor_pos.y + (monitor_size.height as i32 - window_size.height as i32) / 2;
    window.set_outer_position(PhysicalPosition::new(x, y));
}

fn update_camera_target(
    world: &mut WeaverWorld,
    origin_entity: Option<weaver_core::EntityId>,
    distance: f32,
    azimuth: f32,
    elevation: f32,
) {
    let target = origin_entity
        .and_then(|id| world.get(id))
        .and_then(|r| r.mesh.as_ref().map(|m| m.transform.translation))
        .unwrap_or_else(|| world.camera().target);
    camera_from_spherical(world.camera_mut(), target, distance, azimuth, elevation);
}

fn spherical_from_camera(camera: &weaver_render::Camera) -> (f32, f32, f32) {
    let offset = camera.eye - camera.target;
    let distance = offset.length();
    let elevation = (offset.y / distance).asin();
    let azimuth = offset.x.atan2(offset.z);
    (distance, azimuth, elevation)
}

fn camera_from_spherical(
    camera: &mut weaver_render::Camera,
    target: Vec3,
    distance: f32,
    azimuth: f32,
    elevation: f32,
) {
    let x = distance * elevation.cos() * azimuth.sin();
    let y = distance * elevation.sin();
    let z = distance * elevation.cos() * azimuth.cos();
    camera.eye = target + Vec3::new(x, y, z);
    camera.target = target;
}

fn ray_from_mouse(
    camera: &weaver_render::Camera,
    aspect: f32,
    mouse: (f32, f32),
    screen: (f32, f32),
) -> (Vec3, Vec3) {
    let ndc_x = (mouse.0 / screen.0) * 2.0 - 1.0;
    let ndc_y = 1.0 - (mouse.1 / screen.1) * 2.0;
    let clip = Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
    let inv_vp = camera.view_projection(aspect).inverse();
    let world = inv_vp * clip;
    let world_pos = world.xyz() / world.w;
    let dir = (world_pos - camera.eye).normalize();
    (camera.eye, dir)
}

fn ray_sphere_intersect(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let a = dir.dot(dir);
    let b = 2.0 * oc.dot(dir);
    let c = oc.dot(oc) - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / (2.0 * a);
    if t >= 0.0 { Some(t) } else { None }
}
