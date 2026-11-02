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

const WIDTH: u32 = 24;
const HEIGHT: u32 = 24;

pub struct IndicatorState {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub _compositor_state: CompositorState,
    pub shm: Shm,
    pub pool: SlotPool,
    pub layer: LayerSurface,
    pub paused: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub configured: bool,
}

impl CompositorHandler for IndicatorState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_scale: i32,
    ) {
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
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.running.store(false, Ordering::Relaxed);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.configured = true;
        let _ = configure;
        self.draw();
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
    pub fn draw(&mut self) {
        if !self.configured {
            return;
        }

        let stride = (WIDTH * 4) as i32;
        let (buffer, canvas) = match self.pool.create_buffer(
            WIDTH as i32,
            HEIGHT as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(res) => res,
            Err(_) => return,
        };

        let is_paused = self.paused.load(Ordering::Relaxed);
        // Red dot when recording: ARGB = (255, 230, 30, 30)
        // Yellow dot when paused: ARGB = (255, 240, 190, 20)
        let (r, g, b) = if is_paused {
            (240u8, 190u8, 20u8)
        } else {
            (230u8, 30u8, 30u8)
        };

        let center_x = (WIDTH as f32) / 2.0;
        let center_y = (HEIGHT as f32) / 2.0;
        let radius_sq = (center_x - 2.0).powi(2);
        let inner_radius_sq = (center_x - 3.5).powi(2);

        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let offset = ((y * WIDTH + x) * 4) as usize;
                let dx = x as f32 - center_x + 0.5;
                let dy = y as f32 - center_y + 0.5;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= radius_sq {
                    if dist_sq >= inner_radius_sq {
                        // Darker border
                        canvas[offset] = (b as f32 * 0.7) as u8; // B
                        canvas[offset + 1] = (g as f32 * 0.7) as u8; // G
                        canvas[offset + 2] = (r as f32 * 0.7) as u8; // R
                        canvas[offset + 3] = 255; // A
                    } else {
                        canvas[offset] = b;
                        canvas[offset + 1] = g;
                        canvas[offset + 2] = r;
                        canvas[offset + 3] = 255;
                    }
                } else {
                    // Transparent
                    canvas[offset] = 0;
                    canvas[offset + 1] = 0;
                    canvas[offset + 2] = 0;
                    canvas[offset + 3] = 0;
                }
            }
        }

        self.layer
            .wl_surface()
            .attach(Some(buffer.wl_buffer()), 0, 0);
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, WIDTH as i32, HEIGHT as i32);
        self.layer.commit();
    }
}

pub fn run_indicator(paused: Arc<AtomicBool>, running: Arc<AtomicBool>) {
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

    let surface = compositor_state.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("worktrack-indicator"),
        None,
    );

    layer.set_anchor(Anchor::TOP | Anchor::RIGHT);
    layer.set_size(WIDTH, HEIGHT);
    layer.set_margin(8, 8, 0, 0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);

    // Click-through: empty input region
    if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&compositor_state) {
        layer
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
    }

    layer.commit();

    let pool = match SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm) {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut state = IndicatorState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        _compositor_state: compositor_state,
        shm,
        pool,
        layer,
        paused,
        running: running.clone(),
        configured: false,
    };

    let mut last_paused = state.paused.load(Ordering::Relaxed);
    let raw_fd = conn.as_fd().as_raw_fd();

    while running.load(Ordering::Relaxed) {
        let current_paused = state.paused.load(Ordering::Relaxed);
        if current_paused != last_paused {
            last_paused = current_paused;
            state.draw();
        }

        let _ = conn.flush();

        // Standard Wayland event loop integration:
        // Prepare read guard, poll socket with 200ms timeout, then read and dispatch
        if let Some(guard) = conn.prepare_read() {
            let mut poll_fd = libc::pollfd {
                fd: raw_fd,
                events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
                revents: 0,
            };

            let poll_ret = unsafe { libc::poll(&mut poll_fd, 1, 200) };
            if poll_ret > 0 {
                let _ = guard.read();
            }
        }

        let _ = event_queue.dispatch_pending(&mut state);
    }
}
