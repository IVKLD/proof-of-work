#![allow(unknown_lints, clippy::collapsible_if)]

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
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

const LOGICAL_WIDTH: u32 = 130;
const LOGICAL_HEIGHT: u32 = 24;
const FONT_DATA: &[u8] = include_bytes!("../assets/font.ttf");

pub struct MonitorSurface {
    pub output: Option<wl_output::WlOutput>,
    pub output_name: Option<String>,
    pub indicator_layer: LayerSurface,
    pub scale: i32,
    pub ui_scale: f32,
    pub indicator_configured: bool,
    pub pool: SlotPool,
}

pub struct IndicatorState {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub compositor_state: CompositorState,
    pub layer_shell: LayerShell,
    pub shm: Shm,
    pub surfaces: Vec<MonitorSurface>,
    pub qh: QueueHandle<IndicatorState>,
    pub paused: Arc<AtomicBool>,
    pub hidden: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub countdown: Arc<AtomicI64>,
    pub work_remaining: Arc<AtomicI64>,
    pub scale_override: Option<f32>,
    pub target_output: Arc<std::sync::RwLock<Option<String>>>,
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
        _output: wl_output::WlOutput,
    ) {
        // Wait until update_output receives full output metadata (resolution, modes, name)
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if !self.surfaces.iter().any(|s| s.output.as_ref() == Some(&output)) {
            // First time this output is fully known! Create surface with accurate resolution and scale!
            self.create_surface_for_output(Some(&output));
            return;
        }

        let new_ui_scale = self.compute_ui_scale(Some(&output));
        let out_name = self.output_state.info(&output).and_then(|i| i.name);
        for s in &mut self.surfaces {
            if s.output.as_ref() == Some(&output) {
                if s.output_name.is_none() && out_name.is_some() {
                    s.output_name = out_name.clone();
                }
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
        self.surfaces
            .retain(|s| s.indicator_layer.wl_surface() != layer.wl_surface());
        if self.surfaces.is_empty() {
            self.running.store(false, Ordering::Relaxed);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        for s in &mut self.surfaces {
            if s.indicator_layer.wl_surface() == layer.wl_surface() {
                s.indicator_configured = true;
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

        let output_name = output
            .and_then(|o| self.output_state.info(o))
            .and_then(|i| i.name);

        let pool = SlotPool::new(1024 * 1024, &self.shm)
            .expect("Failed to create shm slot pool for monitor");

        self.surfaces.push(MonitorSurface {
            output: output.cloned(),
            output_name,
            indicator_layer,
            scale: 1,
            ui_scale,
            indicator_configured: false,
            pool,
        });
    }

    pub fn draw_all(&mut self) {
        let is_paused = self.paused.load(Ordering::Relaxed);
        let is_hidden = self.hidden.load(Ordering::Relaxed);
        let remaining = self.countdown.load(Ordering::Relaxed);
        let work_rem = self.work_remaining.load(Ordering::Relaxed);
        let current_target = self.target_output.read().ok().and_then(|g| g.clone());

        for s in &mut self.surfaces {
            // Check if this surface is on the target screen
            let is_target_screen = match (&s.output_name, &current_target) {
                (Some(name), Some(target)) => name == target,
                (None, Some(_)) => false,
                (_, None) => true,
            };

            // Draw indicator pill with countdown & dot
            if s.indicator_configured {
                let scale = s.scale.max(1);
                let total_scale = s.ui_scale * (scale as f32);
                let phys_w = (LOGICAL_WIDTH as f32 * total_scale).round() as i32;
                let phys_h = (LOGICAL_HEIGHT as f32 * total_scale).round() as i32;
                let stride = phys_w * 4;
                if let Ok((buffer, canvas)) = s.pool.create_buffer(
                    phys_w,
                    phys_h,
                    stride,
                    wl_shm::Format::Argb8888,
                ) {
                    canvas.fill(0);
                    if !is_hidden && is_target_screen {
                        render_indicator_pill(
                            canvas,
                            phys_w as u32,
                            phys_h as u32,
                            total_scale,
                            is_paused,
                            remaining,
                            work_rem,
                        );
                    }

                    let surface = s.indicator_layer.wl_surface();
                    surface.set_buffer_scale(scale);
                    surface.attach(Some(buffer.wl_buffer()), 0, 0);
                    surface.damage_buffer(0, 0, phys_w, phys_h);
                    s.indicator_layer.commit();
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
    shot_remaining: i64,
    work_remaining: i64,
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

    // 2. Countdown timer and daily remaining work time text
    let shot_text = if is_paused || shot_remaining < 0 {
        "PAUSE".to_string()
    } else {
        let m = shot_remaining / 60;
        let sec = shot_remaining % 60;
        format!("{m}:{sec:02}")
    };

    let work_text = if work_remaining <= 0 {
        let overtime = (-work_remaining) as u64;
        if overtime >= 60 {
            let oh = overtime / 3600;
            let om = (overtime % 3600) / 60;
            format!("+{oh}h {om:02}m")
        } else {
            "0h 00m".to_string()
        }
    } else {
        let rem = work_remaining as u64;
        let rh = rem / 3600;
        let rm = (rem % 3600) / 60;
        format!("{rh}h {rm:02}m")
    };

    let text = format!("{shot_text} | {work_text}");

    if let Ok(font) = FontRef::try_from_slice(FONT_DATA) {
        let font_scale = PxScale::from(12.0 * total_scale);
        let scaled = font.as_scaled(font_scale);

        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for c in text.chars() {
            let glyph = scaled.scaled_glyph(c);
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                if bounds.min.y < min_y { min_y = bounds.min.y; }
                if bounds.max.y > max_y { max_y = bounds.max.y; }
            }
        }

        let text_center_offset = if min_y < max_y {
            scaled.ascent() + (min_y + max_y) / 2.0
        } else {
            let (_tw, th) = text_size(font_scale, &font, &text);
            th as f32 / 2.0
        };

        let tx = (9.0 * total_scale).round() as i32;
        let ty = ((h_f / 2.0) - text_center_offset).round() as i32;

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

    // 3. Status indicator dot (red, amber, or green)
    let dot_cx = w_f - (12.0 * total_scale);
    let dot_cy = h_f / 2.0;
    let dot_radius = 4.2 * total_scale;
    let (base_r, base_g, base_b) = if is_paused {
        (244.0f32, 162.0f32, 97.0f32) // Amber
    } else if work_remaining <= 0 {
        (46.0f32, 196.0f32, 182.0f32)  // Teal/Green: Daily goal achieved!
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
    for (src, dst) in img.pixels().zip(canvas.as_chunks_mut::<4>().0) {
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
    work_remaining: Arc<AtomicI64>,
    scale_override: Option<f32>,
    target_output: Arc<std::sync::RwLock<Option<String>>>,
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

    let mut state = IndicatorState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        layer_shell,
        shm,
        surfaces: Vec::new(),
        qh: qh.clone(),
        paused,
        hidden,
        running: running.clone(),
        countdown,
        work_remaining,
        scale_override,
        target_output,
    };

    // 1. Initial roundtrip to bind globals
    let _ = event_queue.roundtrip(&mut state);
    // 2. Second roundtrip to receive all wl_output metadata and fire update_output
    let _ = event_queue.roundtrip(&mut state);

    if state.surfaces.is_empty() {
        let outputs: Vec<wl_output::WlOutput> = state.output_state.outputs().collect();
        if outputs.is_empty() {
            state.create_surface_for_output(None);
        } else {
            for output in outputs {
                state.create_surface_for_output(Some(&output));
            }
        }
    }

    // 3. Third roundtrip to acknowledge layer surface configuration
    let _ = event_queue.roundtrip(&mut state);

    let mut last_paused = state.paused.load(Ordering::Relaxed);
    let mut last_hidden = state.hidden.load(Ordering::Relaxed);
    let mut last_countdown = state.countdown.load(Ordering::Relaxed);
    let mut last_work_rem = state.work_remaining.load(Ordering::Relaxed);
    let mut last_target = state.target_output.read().ok().and_then(|g| g.clone());
    let raw_fd = conn.as_fd().as_raw_fd();

    while running.load(Ordering::Relaxed) {
        let _ = event_queue.dispatch_pending(&mut state);

        let current_paused = state.paused.load(Ordering::Relaxed);
        let current_hidden = state.hidden.load(Ordering::Relaxed);
        let current_countdown = state.countdown.load(Ordering::Relaxed);
        let current_work_rem = state.work_remaining.load(Ordering::Relaxed);
        let current_target = state.target_output.read().ok().and_then(|g| g.clone());

        if current_paused != last_paused
            || current_hidden != last_hidden
            || current_countdown != last_countdown
            || current_work_rem != last_work_rem
            || current_target != last_target
        {
            last_paused = current_paused;
            last_hidden = current_hidden;
            last_countdown = current_countdown;
            last_work_rem = current_work_rem;
            last_target = current_target;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_pill() {
        let w = 286;
        let h = 53;
        let mut canvas = vec![0u8; (w * h * 4) as usize];

        // 1. Test countdown "1:00 | 7h 45m"
        render_indicator_pill(&mut canvas, w, h, 2.2, false, 60, 7 * 3600 + 45 * 60);
        let mut y_min = h;
        let mut y_max = 0;
        for y in 0..h {
            for x in 15..150 {
                let idx = ((y * w + x) * 4) as usize;
                let (b, g, r) = (canvas[idx], canvas[idx + 1], canvas[idx + 2]);
                if r > 180 && g > 180 && b > 180 {
                    if y < y_min { y_min = y; }
                    if y > y_max { y_max = y; }
                }
            }
        }
        println!("1:00 | 7h 45m -> top margin: {}, bottom margin: {}", y_min, h - 1 - y_max);
        assert!((y_min as i32 - (h - 1 - y_max) as i32).abs() <= 1);

        // 2. Test "PAUSE | 7h 45m"
        canvas.fill(0);
        render_indicator_pill(&mut canvas, w, h, 2.2, true, -1, 7 * 3600 + 45 * 60);
        let mut p_min = h;
        let mut p_max = 0;
        for y in 0..h {
            for x in 15..150 {
                let idx = ((y * w + x) * 4) as usize;
                let (b, g, r) = (canvas[idx], canvas[idx + 1], canvas[idx + 2]);
                if r > 180 && g > 180 && b > 180 {
                    if y < p_min { p_min = y; }
                    if y > p_max { p_max = y; }
                }
            }
        }
        println!("PAUSE | 7h 45m -> top margin: {}, bottom margin: {}", p_min, h - 1 - p_max);
        assert!((p_min as i32 - (h - 1 - p_max) as i32).abs() <= 1);
    }
}
