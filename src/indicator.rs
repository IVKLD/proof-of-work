use std::os::fd::{AsFd, AsRawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

const LOGICAL_SIZE: u32 = 28;

pub struct MonitorSurface {
    pub layer: LayerSurface,
    pub scale: i32,
    pub configured: bool,
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
            if s.layer.wl_surface() == surface {
                s.scale = new_scale.max(1);
                s.layer.wl_surface().set_buffer_scale(s.scale);
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
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
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
            .retain(|s| s.layer.wl_surface() != layer.wl_surface());
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
            if s.layer.wl_surface() == layer.wl_surface() {
                s.configured = true;
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
    pub fn create_surface_for_output(&mut self, output: Option<&wl_output::WlOutput>) {
        let surface = self.compositor_state.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some("worktrack-indicator"),
            output,
        );

        layer.set_size(LOGICAL_SIZE, LOGICAL_SIZE);
        layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
        layer.set_margin(12, 12, 0, 0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);

        // Click-through: empty input region allows clicks to pass through
        if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor_state)
        {
            layer
                .wl_surface()
                .set_input_region(Some(region.wl_region()));
        }

        layer.commit();

        self.surfaces.push(MonitorSurface {
            layer,
            scale: 1,
            configured: false,
        });
    }

    pub fn draw_all(&mut self) {
        let is_paused = self.paused.load(Ordering::Relaxed);
        let is_hidden = self.hidden.load(Ordering::Relaxed);

        // Vibrant Red: recording (#e63946)
        // Warm Amber/Yellow: paused (#f4a261)
        let (base_r, base_g, base_b) = if is_paused {
            (244.0f32, 162.0f32, 97.0f32)
        } else {
            (230.0f32, 57.0f32, 70.0f32)
        };

        for i in 0..self.surfaces.len() {
            if !self.surfaces[i].configured {
                continue;
            }

            let scale = self.surfaces[i].scale.max(1);
            let phys_size = (LOGICAL_SIZE as i32) * scale;
            let stride = phys_size * 4;

            let (buffer, canvas) = match self.pool.create_buffer(
                phys_size,
                phys_size,
                stride,
                wl_shm::Format::Argb8888,
            ) {
                Ok(res) => res,
                Err(_) => continue,
            };

            if is_hidden {
                // Completely transparent buffer during screenshot capture (alpha = 0)
                canvas.fill(0);
            } else {
                let center = (phys_size as f32) / 2.0;
                let radius = center - (2.5 * scale as f32);
                let border_width = 1.8 * scale as f32;

                for y in 0..phys_size {
                    for x in 0..phys_size {
                        let offset = ((y * phys_size + x) * 4) as usize;
                        let dx = (x as f32) - center + 0.5;
                        let dy = (y as f32) - center + 0.5;
                        let dist = (dx * dx + dy * dy).sqrt();

                        // Anti-aliased circle edge
                        let outer_alpha = (radius + 0.5 - dist).clamp(0.0, 1.0);

                        if outer_alpha <= 0.0 {
                            canvas[offset] = 0;
                            canvas[offset + 1] = 0;
                            canvas[offset + 2] = 0;
                            canvas[offset + 3] = 0;
                            continue;
                        }

                        // Border shading
                        let border_factor =
                            ((dist - (radius - border_width)) / border_width).clamp(0.0, 1.0);
                        let shade = 1.0 - (border_factor * 0.35);

                        let r = base_r * shade;
                        let g = base_g * shade;
                        let b = base_b * shade;

                        // Premultiplied ARGB for Wayland wl_shm
                        let a_byte = (outer_alpha * 255.0).round() as u8;
                        canvas[offset] = ((b * outer_alpha).round()) as u8;
                        canvas[offset + 1] = ((g * outer_alpha).round()) as u8;
                        canvas[offset + 2] = ((r * outer_alpha).round()) as u8;
                        canvas[offset + 3] = a_byte;
                    }
                }
            }

            let surface = self.surfaces[i].layer.wl_surface();
            surface.set_buffer_scale(scale);
            surface.attach(Some(buffer.wl_buffer()), 0, 0);
            surface.damage_buffer(0, 0, phys_size, phys_size);
            self.surfaces[i].layer.commit();
        }
    }
}

pub fn run_indicator(
    paused: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
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

    // Buffer pool for multi-monitor 4K HiDPI
    let pool = match SlotPool::new(128 * 128 * 4 * 4, &shm) {
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
    };

    let _ = event_queue.roundtrip(&mut state);

    let outputs: Vec<wl_output::WlOutput> = state.output_state.outputs().collect();
    if outputs.is_empty() {
        state.create_surface_for_output(None);
    } else {
        for output in outputs {
            state.create_surface_for_output(Some(&output));
        }
    }

    let _ = event_queue.roundtrip(&mut state);

    let mut last_paused = state.paused.load(Ordering::Relaxed);
    let mut last_hidden = state.hidden.load(Ordering::Relaxed);
    let raw_fd = conn.as_fd().as_raw_fd();

    while running.load(Ordering::Relaxed) {
        let current_paused = state.paused.load(Ordering::Relaxed);
        let current_hidden = state.hidden.load(Ordering::Relaxed);

        if current_paused != last_paused || current_hidden != last_hidden {
            last_paused = current_paused;
            last_hidden = current_hidden;
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

            let poll_ret = unsafe { libc::poll(&mut poll_fd, 1, 20) };
            if poll_ret > 0 {
                let _ = guard.read();
            }
        }

        let _ = event_queue.dispatch_pending(&mut state);
    }
}
