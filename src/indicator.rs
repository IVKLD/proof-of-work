#![allow(unknown_lints, clippy::collapsible_if)]

use ab_glyph::{FontRef, PxScale};
use imageproc::drawing::{draw_text_mut, text_size};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

const LOGICAL_WIDTH: u32 = 78;
const LOGICAL_HEIGHT: u32 = 24;
const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

pub struct MonitorSurface {
    pub output: Option<wl_output::WlOutput>,
    pub indicator_layer: LayerSurface,
    pub bg_cover_layer: LayerSurface,
    pub scale: i32,
    pub ui_scale: f32,
    pub indicator_configured: bool,
    pub bg_configured: bool,
    pub bg_width: u32,
    pub bg_height: u32,
}

pub struct IndicatorState {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub compositor_state: CompositorState,
    pub layer_shell: LayerShell,
    pub shm: Shm,
    pub pool: SlotPool,
    pub surfaces: Vec<MonitorSurface>,
    pub qh: QueueHandle<IndicatorState>,
    pub paused: Arc<AtomicBool>,
    pub hidden: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub countdown: Arc<AtomicI64>,
    pub scale_override: Option<f32>,
}

impl CompositorHandler for IndicatorState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_scale: i32,
    ) {
        for s in &mut self.surfaces {
            if s.indicator_layer.wl_surface() == surface {
                s.scale = new_scale.max(1);
                s.indicator_layer.wl_surface().set_buffer_scale(s.scale);
                break;
            }
        }
        self.draw_all();
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }
}

impl OutputHandler for IndicatorState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.create_surface_for_output(Some(&output));
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let new_ui_scale = self.compute_ui_scale(Some(&output));
        for s in &mut self.surfaces {
            if s.output.as_ref() == Some(&output) {
                if (s.ui_scale - new_ui_scale).abs() > 0.01 {
                    s.ui_scale = new_ui_scale;
                    let logical_w = (LOGICAL_WIDTH as f32 * new_ui_scale).round() as u32;
                    let logical_h = (LOGICAL_HEIGHT as f32 * new_ui_scale).round() as u32;
                    let logical_margin = (12.0 * new_ui_scale).round() as i32;
                    s.indicator_layer.set_size(logical_w, logical_h);
                    s.indicator_layer.set_margin(logical_margin, logical_margin, 0, 0);
                    s.indicator_layer.commit();
                }
                break;
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.surfaces.retain(|s| s.output.as_ref() != Some(&output));
    }
}

impl ShmHandler for IndicatorState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for IndicatorState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        self.surfaces.retain(|s| {
            s.indicator_layer.wl_surface() != layer.wl_surface()
                && s.bg_cover_layer.wl_surface() != layer.wl_surface()
        });
        if self.surfaces.is_empty() {
            self.running.store(false, Ordering::Relaxed);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        for s in &mut self.surfaces {
            if s.indicator_layer.wl_surface() == layer.wl_surface() {
                s.indicator_configured = true;
                break;
            } else if s.bg_cover_layer.wl_surface() == layer.wl_surface() {
                s.bg_configured = true;
                s.bg_width = configure.new_size.0;
                s.bg_height = configure.new_size.1;
                break;
            }
        }
        self.draw_all();
    }
}

impl ProvidesRegistryState for IndicatorState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(IndicatorState);
delegate_output!(IndicatorState);
delegate_shm!(IndicatorState);
delegate_layer!(IndicatorState);
delegate_registry!(IndicatorState);

impl IndicatorState {
    pub fn compute_ui_scale(&self, output: Option<&wl_output::WlOutput>) -> f32 {
        if let Some(override_scale) = self.scale_override {
            override_scale.max(0.5)
        } else {
            let (mon_w, mon_h) = if let Some(info) = output.and_then(|o| self.output_state.info(o)) {
                info.logical_size.unwrap_or_else(|| {
                    info.modes
                        .iter()
                        .find(|m| m.current)
                        .map(|m| (m.dimensions.0, m.dimensions.1))
                        .unwrap_or((1920, 1080))
                })
            } else {
                (1920, 1080)
            };
            if mon_w >= 3840 || mon_h >= 2160 {
                2.2f32 // 4K HiDPI
            } else if mon_w >= 2560 || mon_h >= 1440 {
                1.4f32 // QHD
            } else {
                1.0f32 // 1080p
            }
        }
    }

    pub fn create_surface_for_output(&mut self, output: Option<&wl_output::WlOutput>) {
        if let Some(out) = output {
            if self.surfaces.iter().any(|s| s.output.as_ref() == Some(out)) {
                return;
            }
        }

        let ui_scale = self.compute_ui_scale(output);
        let logical_w = (LOGICAL_WIDTH as f32 * ui_scale).round() as u32;
        let logical_h = (LOGICAL_HEIGHT as f32 * ui_scale).round() as u32;
        let logical_margin = (12.0 * ui_scale).round() as i32;

        // 1. Overlay indicator surface
        let ind_surface = self.compositor_state.create_surface(&self.qh);
        let indicator_layer = self.layer_shell.create_layer_surface(
            &self.qh,
            ind_surface,
            Layer::Overlay,
            Some("worktrack-indicator"),
            output,
        );

        indicator_layer.set_size(logical_w, logical_h);
        indicator_layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
        indicator_layer.set_margin(logical_margin, logical_margin, 0, 0);
        indicator_layer.set_keyboard_interactivity(KeyboardInteractivity::None);

        if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor_state)
        {
            indicator_layer
                .wl_surface()
                .set_input_region(Some(region.wl_region()));
        }

        indicator_layer.commit();

        // 2. Wallpaper barrier surface on Layer::Bottom (sits ABOVE mpvpaper and BELOW windows)
        let bg_surface = self.compositor_state.create_surface(&self.qh);
        let bg_cover_layer = self.layer_shell.create_layer_surface(
            &self.qh,
            bg_surface,
            Layer::Bottom,
            Some("worktrack-bg-barrier"),
            output,
        );

        bg_cover_layer.set_size(0, 0);
        bg_cover_layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        bg_cover_layer.set_keyboard_interactivity(KeyboardInteractivity::None);

        if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor_state)
        {
            bg_cover_layer
                .wl_surface()
                .set_input_region(Some(region.wl_region()));
        }

        bg_cover_layer.commit();

        self.surfaces.push(MonitorSurface {
            output: output.cloned(),
            indicator_layer,
            bg_cover_layer,
            scale: 1,
            ui_scale,
            indicator_configured: false,
            bg_configured: false,
            bg_width: 0,
            bg_height: 0,
        });
    }

    pub fn draw_all(&mut self) {
        let is_paused = self.paused.load(Ordering::Relaxed);
        let hide_file = std::env::var("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
            .join("worktrack.hide");
        let is_hidden = self.hidden.load(Ordering::Relaxed) || hide_file.exists();
        let remaining = self.countdown.load(Ordering::Relaxed);

        for s in &mut self.surfaces {
            // 1. Draw indicator pill with countdown & dot
            if s.indicator_configured {
                let scale = s.scale.max(1);
                let total_scale = s.ui_scale * (scale as f32);
                let phys_w = (LOGICAL_WIDTH as f32 * total_scale).round() as i32;
                let phys_h = (LOGICAL_HEIGHT as f32 * total_scale).round() as i32;
                let stride = phys_w * 4;

                if let Ok((buffer, canvas)) = self.pool.create_buffer(
                    phys_w,
                    phys_h,
                    stride,
                    wl_shm::Format::Argb8888,
                ) {
                    canvas.fill(0);
                    if !is_hidden {
                        render_indicator_pill(
                            canvas,
                            phys_w as u32,
                            phys_h as u32,
                            total_scale,
                            is_paused,
                            remaining,
                        );
                    }

                    let surface = s.indicator_layer.wl_surface();
                    surface.set_buffer_scale(scale);
                    surface.attach(Some(buffer.wl_buffer()), 0, 0);
                    surface.damage_buffer(0, 0, phys_w, phys_h);
                    s.indicator_layer.commit();
                }
            }

            // 2. Manage Layer::Bottom wallpaper barrier
            if s.bg_configured && s.bg_width > 0 && s.bg_height > 0 {
                if is_hidden {
                    // SCREENSHOT MODE: Solid dark #0f0f0f buffer covers mpvpaper natively
                    let w = s.bg_width;
                    let h = s.bg_height;
                    let stride = (w * 4) as i32;

                    if let Ok((buffer, canvas)) = self.pool.create_buffer(
                        w as i32,
                        h as i32,
                        stride,
                        wl_shm::Format::Argb8888,
                    ) {
                        for p in canvas.chunks_exact_mut(4) {
                            p[0] = 15;  // B (#0f)
                            p[1] = 15;  // G (#0f)
                            p[2] = 15;  // R (#0f)
                            p[3] = 255; // A (opaque)
                        }
                        let surface = s.bg_cover_layer.wl_surface();
                        surface.attach(Some(buffer.wl_buffer()), 0, 0);
                        surface.damage_buffer(0, 0, w as i32, h as i32);
                        s.bg_cover_layer.commit();
                    }
                } else {
                    // NORMAL RUNNING MODE: 1x1 transparent buffer (alpha = 0)
                    if let Ok((buffer, canvas)) = self.pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
                        canvas.fill(0);
                        let surface = s.bg_cover_layer.wl_surface();
                        surface.attach(Some(buffer.wl_buffer()), 0, 0);
                        surface.damage_buffer(0, 0, 1, 1);
                        s.bg_cover_layer.commit();
                    }
                }
            }
        }
    }
}

pub fn render_indicator_pill(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    total_scale: f32,
    is_paused: bool,
    remaining: i64,
) {
    let mut img = image::RgbaImage::new(w, h);
    let w_f = w as f32;
    let h_f = h as f32;
    let r_f = h_f / 2.0;

    // 1. Rounded pill background with subtle dark glass appearance
    for y in 0..h {
        for x in 0..w {
            let cx = (x as f32 + 0.5).clamp(r_f, w_f - r_f);
            let cy = r_f;
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let alpha = (r_f + 0.5 - dist).clamp(0.0, 1.0);
            if alpha > 0.0 {
                let border_width = 1.3 * total_scale;
                let is_border = dist >= (r_f - border_width);
                let (bg_r, bg_g, bg_b, bg_a) = if is_border {
                    (70u8, 70u8, 80u8, (220.0 * alpha) as u8)
                } else {
                    (18u8, 18u8, 22u8, (200.0 * alpha) as u8)
                };
                img.put_pixel(x, y, image::Rgba([bg_r, bg_g, bg_b, bg_a]));
            }
        }
    }

    // 2. Countdown timer text
    let text = if is_paused || remaining < 0 {
        "PAUSE".to_string()
    } else {
        let m = remaining / 60;
        let sec = remaining % 60;
        format!("{m}:{sec:02}")
    };

    if let Ok(font) = FontRef::try_from_slice(FONT_DATA) {
        let font_scale = PxScale::from(12.0 * total_scale);
        let (_tw, th) = text_size(font_scale, &font, &text);
        let tx = (9.0 * total_scale).round() as i32;
        let ty = ((h_f - th as f32) / 2.0).round() as i32;
        draw_text_mut(
            &mut img,
            image::Rgba([240, 240, 245, 255]),
            tx,
            ty,
            font_scale,
            &font,
            &text,
        );
    }

    // 3. Status indicator dot (red or amber)
    let dot_cx = w_f - (12.0 * total_scale);
    let dot_cy = h_f / 2.0;
    let dot_radius = 4.2 * total_scale;
    let (base_r, base_g, base_b) = if is_paused {
        (244.0f32, 162.0f32, 97.0f32) // Amber
    } else {
        (230.0f32, 57.0f32, 70.0f32)  // Red
    };

    let min_x = (dot_cx - dot_radius - 2.0).max(0.0) as u32;
    let max_x = (dot_cx + dot_radius + 2.0).min(w_f) as u32;
    let min_y = (dot_cy - dot_radius - 2.0).max(0.0) as u32;
    let max_y = (dot_cy + dot_radius + 2.0).min(h_f) as u32;

    for y in min_y..max_y {
        for x in min_x..max_x {
            let dx = (x as f32 + 0.5) - dot_cx;
            let dy = (y as f32 + 0.5) - dot_cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let dot_alpha = (dot_radius + 0.5 - dist).clamp(0.0, 1.0);
            if dot_alpha > 0.0 {
                let shade = 1.0 - (dist / dot_radius).clamp(0.0, 1.0) * 0.25;
                let r = (base_r * shade) as u8;
                let g = (base_g * shade) as u8;
                let b = (base_b * shade) as u8;

                // Alpha blend over pill
                let existing = img.get_pixel(x, y).0;
                let ex_a = existing[3] as f32 / 255.0;
                let in_a = dot_alpha;
                let out_a = in_a + ex_a * (1.0 - in_a);
                if out_a > 0.0 {
                    let out_r = ((r as f32 * in_a + existing[0] as f32 * ex_a * (1.0 - in_a)) / out_a) as u8;
                    let out_g = ((g as f32 * in_a + existing[1] as f32 * ex_a * (1.0 - in_a)) / out_a) as u8;
                    let out_b = ((b as f32 * in_a + existing[2] as f32 * ex_a * (1.0 - in_a)) / out_a) as u8;
                    img.put_pixel(x, y, image::Rgba([out_r, out_g, out_b, (out_a * 255.0) as u8]));
                }
            }
        }
    }

    // 4. Pre-multiplied ARGB8888 for Wayland SHM canvas
    for (src, dst) in img.pixels().zip(canvas.chunks_exact_mut(4)) {
        let [r, g, b, a] = src.0;
        let alpha = a as f32 / 255.0;
        dst[0] = ((b as f32) * alpha).round() as u8;
        dst[1] = ((g as f32) * alpha).round() as u8;
        dst[2] = ((r as f32) * alpha).round() as u8;
        dst[3] = a;
    }
}

pub fn run_indicator(
    paused: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    countdown: Arc<AtomicI64>,
    scale_override: Option<f32>,
) {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(_) => return,
    };

    let (globals, mut event_queue) = match registry_queue_init(&conn) {
        Ok(q) => q,
        Err(_) => return,
    };
    let qh = event_queue.handle();

    let compositor_state = match CompositorState::bind(&globals, &qh) {
        Ok(s) => s,
        Err(_) => return,
    };
    let layer_shell = match LayerShell::bind(&globals, &qh) {
        Ok(s) => s,
        Err(_) => return,
    };
    let shm = match Shm::bind(&globals, &qh) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Buffer pool for multi-monitor 4K HiDPI and Layer::Bottom background cover
    let pool = match SlotPool::new(64 * 1024 * 1024, &shm) {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut state = IndicatorState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        layer_shell,
        shm,
        pool,
        surfaces: Vec::new(),
        qh: qh.clone(),
        paused,
        hidden,
        running: running.clone(),
        countdown,
        scale_override,
    };

    let _ = event_queue.roundtrip(&mut state);

    if state.surfaces.is_empty() {
        state.create_surface_for_output(None);
    }

    let _ = event_queue.roundtrip(&mut state);

    let hide_file = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        .join("worktrack.hide");

    let mut last_paused = state.paused.load(Ordering::Relaxed);
    let mut last_hidden = state.hidden.load(Ordering::Relaxed) || hide_file.exists();
    let mut last_countdown = state.countdown.load(Ordering::Relaxed);
    let raw_fd = conn.as_fd().as_raw_fd();

    while running.load(Ordering::Relaxed) {
        let _ = event_queue.dispatch_pending(&mut state);

        let current_paused = state.paused.load(Ordering::Relaxed);
        let current_hidden = state.hidden.load(Ordering::Relaxed) || hide_file.exists();
        let current_countdown = state.countdown.load(Ordering::Relaxed);

        if current_paused != last_paused || current_hidden != last_hidden || current_countdown != last_countdown {
            last_paused = current_paused;
            last_hidden = current_hidden;
            last_countdown = current_countdown;
            state.draw_all();
            let _ = conn.flush();
        }

        let _ = conn.flush();

        if let Some(guard) = conn.prepare_read() {
            let mut poll_fd = libc::pollfd {
                fd: raw_fd,
                events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&mut poll_fd, 1, 100) };
            if poll_ret > 0 {
                let _ = guard.read();
            }
        } else {
            // Prevent busy spinning if prepare_read is None
            std::thread::sleep(Duration::from_millis(50));
        }

        let _ = event_queue.dispatch_pending(&mut state);
    }
}

struct BarrierState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    surface: Option<LayerSurface>,
    configured: bool,
    width: u32,
    height: u32,
}

impl CompositorHandler for BarrierState {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wayland_client::protocol::wl_output::Transform) {}
}

impl OutputHandler for BarrierState {
    fn output_state(&mut self) -> &mut OutputState { &mut self.output_state }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for BarrierState {
    fn shm_state(&mut self) -> &mut Shm { &mut self.shm }
}

impl LayerShellHandler for BarrierState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {}
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        self.configured = true;
        self.width = configure.new_size.0;
        self.height = configure.new_size.1;
    }
}

impl ProvidesRegistryState for BarrierState {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry_state }
    registry_handlers![OutputState];
}

delegate_compositor!(BarrierState);
delegate_output!(BarrierState);
delegate_shm!(BarrierState);
delegate_layer!(BarrierState);
delegate_registry!(BarrierState);

pub fn capture_with_standalone_barrier(
    path: &str,
    target_output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor_state = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new(3840 * 2160 * 4, &shm)?;

    let mut state = BarrierState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        layer_shell,
        shm,
        pool,
        surface: None,
        configured: false,
        width: 0,
        height: 0,
    };

    let _ = event_queue.roundtrip(&mut state);

    let outputs: Vec<wl_output::WlOutput> = state.output_state.outputs().collect();
    let mut chosen_out = None;
    for out in &outputs {
        if let Some(info) = state.output_state.info(out) {
            if info.name.as_deref() == Some(target_output) {
                chosen_out = Some(out.clone());
                break;
            }
        }
    }

    let wl_surf = state.compositor_state.create_surface(&qh);
    let layer = state.layer_shell.create_layer_surface(
        &qh,
        wl_surf,
        Layer::Bottom,
        Some("worktrack-shot-barrier"),
        chosen_out.as_ref(),
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();

    state.surface = Some(layer);

    while !state.configured {
        let _ = event_queue.blocking_dispatch(&mut state);
    }

    let (w, h) = (state.width, state.height);
    if w > 0 && h > 0 {
        let (buffer, canvas) = state.pool.create_buffer(w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888)?;
        for p in canvas.chunks_exact_mut(4) {
            p[0] = 15;  // B (#0f)
            p[1] = 15;  // G (#0f)
            p[2] = 15;  // R (#0f)
            p[3] = 255; // A (opaque)
        }
        if let Some(ref s) = state.surface {
            s.wl_surface().attach(Some(buffer.wl_buffer()), 0, 0);
            s.wl_surface().damage_buffer(0, 0, w as i32, h as i32);
            s.commit();
        }
        let _ = conn.flush();
        let _ = event_queue.roundtrip(&mut state);
        std::thread::sleep(Duration::from_millis(50));
    }

    let grim = crate::niri::grim_bin();
    let status = std::process::Command::new(grim)
        .args(["-c", "-o", target_output, path])
        .status()?;

    if !status.success() {
        return Err(format!("grim failed to capture output {target_output}").into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_pill() {
        let w = 172;
        let h = 53;
        let mut canvas = vec![0u8; (w * h * 4) as usize];
        render_indicator_pill(&mut canvas, w, h, 2.2, false, 45);
        let non_zero = canvas.iter().any(|&b| b > 0);
        assert!(non_zero);

        // Test paused
        render_indicator_pill(&mut canvas, w, h, 2.2, true, -1);
        let non_zero_paused = canvas.iter().any(|&b| b > 0);
        assert!(non_zero_paused);
    }
}
