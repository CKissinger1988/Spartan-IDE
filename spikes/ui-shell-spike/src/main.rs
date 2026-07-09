mod gpu;
mod latency;
mod panel;
mod webview_bridge;

#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowBuilder;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::Rect;

/// A real bug found only by testing keyboard input after clicking the
/// WebView, not predicted in advance: once wry's child `WebView2` control
/// takes keyboard focus (on creation or on click), clicking elsewhere in
/// the *same top-level window's native (non-WebView) area* does not return
/// keyboard focus to winit -- confirmed with an isolated test crate: the
/// top-level window remains the OS foreground window
/// (`GetForegroundWindow` matches) and `WindowEvent::Focused(true)` never
/// fires again, so no `KeyboardInput` events reach the native event loop at
/// all. `winit::window::Window::focus_window()` alone does not fix this
/// either (tested). A direct Win32 `SetFocus` call on the window's own HWND
/// does. This is a concrete instance of the exact "does this feel like one
/// app" risk §39.4/§35.9 name as Spike 0.4's central uncertainty -- not a
/// cosmetic seam, but keyboard input ownership silently getting stuck on
/// the WebView.
///
/// Windows/WebView2-only, matching this whole spike's scope (`wry` uses
/// WebView2 on Windows, a different backend -- WebKitGTK -- on Linux, where
/// this exact focus-stealing bug, if it exists at all, hasn't been
/// investigated). Gated with `#[cfg(windows)]` rather than left unguarded --
/// found by actually running `cargo build --workspace` on Linux, where the
/// unconditional `windows`-crate FFI call failed to link (`undefined symbol:
/// SetFocus`), not by inspection.
#[cfg(windows)]
fn win32_hwnd(window: &winit::window::Window) -> HWND {
    match window
        .window_handle()
        .expect("window should have a valid handle")
        .as_raw()
    {
        RawWindowHandle::Win32(h) => HWND(h.hwnd.get()),
        other => panic!("unexpected window handle type: {other:?}"),
    }
}

const LEFT_RAIL_WIDTH: f64 = 200.0;
const AUX_PANE_WIDTH: f64 = 250.0;
const FADE_DURATION_MS: f64 = 180.0; // §8.4's stated cross-fade duration

/// sRGB -> linear, same EOTF as `render-spike`'s `srgb_to_linear` -- the
/// panel colors below are sRGB hex values and must be converted before
/// being written to an sRGB-format surface, or they render visibly lighter
/// than intended (the exact bug `render-spike` found and documented first).
fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_hex_to_linear_rgba(r: u8, g: u8, b: u8) -> [f32; 4] {
    [
        srgb_to_linear(r as f32 / 255.0),
        srgb_to_linear(g as f32 / 255.0),
        srgb_to_linear(b as f32 / 255.0),
        1.0,
    ]
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
        1.0,
    ]
}

/// Three placeholder "modes" (Agent/Editor/Design, per §8.1) represented
/// only as the auxiliary pane's color -- real per-mode content is out of
/// scope for this increment (§39.4 explicitly excludes it).
const MODE_COLORS: [(&str, (u8, u8, u8)); 3] = [
    ("Agent", (0xC4, 0x43, 0x2B)),  // accent rust
    ("Editor", (0x3D, 0x9C, 0x93)), // teal
    ("Design", (0x4E, 0x9E, 0x72)), // green
];

fn main() {
    println!("=== ui-shell-spike -- Spike 0.4 first increment ===");

    // Optional CLI arg: after startup, dispatch N synthetic clicks on the
    // WebView's button via script injection (`element.click()`), spaced
    // 50ms apart, to gather repeatable round-trip samples without needing
    // fragile OS-level mouse-coordinate clicking. Stated plainly, matching
    // render-spike's own precedent: this is a real click dispatched through
    // the DOM's own event system (exercising the exact IPC code path a real
    // click would), but it is script-triggered, not real OS input.
    let auto_click_count: Option<usize> = std::env::args().nth(1).and_then(|s| s.parse().ok());
    if let Some(n) = auto_click_count {
        println!("Scripted round-trip benchmark: {n} synthetic button clicks");
    }

    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Spartan ui-shell-spike")
            .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 700.0))
            .build(&event_loop)
            .expect("failed to create window"),
    );

    let mut gpu_state = pollster::block_on(gpu::GpuState::new(window.clone()));
    println!(
        "Adapter: {} | backend={:?} | device_type={:?}",
        gpu_state.adapter_info.name,
        gpu_state.adapter_info.backend,
        gpu_state.adapter_info.device_type
    );

    let panel_renderer = panel::PanelRenderer::new(&gpu_state.device, gpu_state.config.format, 4);

    // Owns its own clone of the `Arc<Window>` rather than borrowing the
    // outer `window` binding, since the latter is moved wholesale into the
    // `event_loop.run(move |...| ...)` closure below -- a shared borrow and
    // a move of the same binding can't coexist.
    let center_stage_bounds = {
        let window = window.clone();
        move |size: winit::dpi::PhysicalSize<u32>| -> Rect {
            let scale = window.scale_factor();
            Rect {
                position: LogicalPosition::new(LEFT_RAIL_WIDTH, 0.0).into(),
                size: LogicalSize::new(
                    (size.width as f64 / scale - LEFT_RAIL_WIDTH - AUX_PANE_WIDTH).max(0.0),
                    size.height as f64 / scale,
                )
                .into(),
            }
        }
    };

    let bridge = webview_bridge::WebviewBridge::new(&window, center_stage_bounds(gpu_state.size));

    let mut native_counter: i64 = 0;
    let mut mode_index: usize = 0;
    let mut fade_from = srgb_hex_to_linear_rgba(
        MODE_COLORS[0].1 .0,
        MODE_COLORS[0].1 .1,
        MODE_COLORS[0].1 .2,
    );
    let mut fade_to = fade_from;
    let mut fade_start: Option<Instant> = None;
    let mut mode_switch_samples: Vec<f64> = Vec::new();

    let left_rail_color = srgb_hex_to_linear_rgba(0x18, 0x18, 0x1B); // s2 token

    let mut samples_reported = 0usize;
    let mut auto_click_remaining = auto_click_count.unwrap_or(0);
    let mut last_auto_click = Instant::now();
    const AUTO_CLICK_INTERVAL: Duration = Duration::from_millis(50);

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);

            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    match event {
                        WindowEvent::CloseRequested => {
                            println!("\n=== Final reports ===");
                            println!(
                            "Final IPC-side counter value: {} (confirms JS click -> Rust IPC -> \
                             counter increment actually happened, not just that messages arrived)",
                            *bridge.counter.borrow()
                        );
                            latency::percentiles(
                                bridge.round_trip_samples.borrow().clone(),
                                "ipc round-trip",
                            );
                            latency::percentiles(
                                mode_switch_samples.clone(),
                                "mode-switch fade duration",
                            );
                            elwt.exit();
                        }
                        WindowEvent::Resized(new_size) => {
                            gpu_state.resize(new_size);
                            bridge.set_bounds(center_stage_bounds(new_size));
                            window.request_redraw();
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            ..
                        } => {
                            // Only fires for clicks in the native (non-WebView)
                            // area -- see `win32_hwnd`'s doc comment for why this
                            // is necessary on Windows. Windows/WebView2-only;
                            // a no-op on other platforms (unverified there).
                            #[cfg(windows)]
                            unsafe {
                                let _ = SetFocus(win32_hwnd(&window));
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    state: ElementState::Pressed,
                                    logical_key,
                                    ..
                                },
                            ..
                        } => match logical_key {
                            Key::Named(NamedKey::ArrowUp) => {
                                native_counter += 1;
                                bridge.push_counter_from_native(native_counter);
                            }
                            Key::Named(NamedKey::Tab) => {
                                let next_index = (mode_index + 1) % MODE_COLORS.len();
                                let (name, (r, g, b)) = MODE_COLORS[next_index];
                                fade_from =
                                    lerp_color(fade_from, fade_to, fade_progress(fade_start));
                                fade_to = srgb_hex_to_linear_rgba(r, g, b);
                                fade_start = Some(Instant::now());
                                mode_index = next_index;
                                println!("Mode switch -> {name}");
                            }
                            _ => {}
                        },
                        WindowEvent::RedrawRequested => {
                            let t = fade_progress(fade_start);
                            if t >= 1.0 {
                                if let Some(start) = fade_start.take() {
                                    let real_elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                                    mode_switch_samples.push(real_elapsed_ms);
                                }
                                fade_from = fade_to;
                            }
                            let aux_color = lerp_color(fade_from, fade_to, t);

                            let frame = match gpu_state.surface.get_current_texture() {
                                Ok(frame) => frame,
                                Err(_) => {
                                    gpu_state.resize(gpu_state.size);
                                    return;
                                }
                            };
                            let view = frame
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            let screen_w = gpu_state.size.width as f32;
                            let screen_h = gpu_state.size.height as f32;
                            let scale = window.scale_factor() as f32;
                            let vertex_count = panel_renderer.update(
                                &gpu_state.queue,
                                &[
                                    panel::PanelRect {
                                        x: 0.0,
                                        y: 0.0,
                                        width: LEFT_RAIL_WIDTH as f32 * scale,
                                        height: screen_h,
                                        color: left_rail_color,
                                    },
                                    panel::PanelRect {
                                        x: screen_w - AUX_PANE_WIDTH as f32 * scale,
                                        y: 0.0,
                                        width: AUX_PANE_WIDTH as f32 * scale,
                                        height: screen_h,
                                        color: aux_color,
                                    },
                                ],
                                screen_w,
                                screen_h,
                            );

                            let mut encoder = gpu_state.device.create_command_encoder(
                                &wgpu::CommandEncoderDescriptor {
                                    label: Some("ui-shell-spike encoder"),
                                },
                            );
                            {
                                let mut pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("clear + panels pass"),
                                        color_attachments: &[Some(
                                            wgpu::RenderPassColorAttachment {
                                                view: &view,
                                                resolve_target: None,
                                                ops: wgpu::Operations {
                                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                                        r: srgb_to_linear(0.035) as f64,
                                                        g: srgb_to_linear(0.035) as f64,
                                                        b: srgb_to_linear(0.04) as f64,
                                                        a: 1.0,
                                                    }),
                                                    store: wgpu::StoreOp::Store,
                                                },
                                            },
                                        )],
                                        depth_stencil_attachment: None,
                                        timestamp_writes: None,
                                        occlusion_query_set: None,
                                    });
                                panel_renderer.render(&mut pass, vertex_count);
                            }
                            gpu_state.queue.submit(std::iter::once(encoder.finish()));
                            frame.present();

                            let sample_count = bridge.round_trip_samples.borrow().len();
                            if sample_count > 0 && sample_count - samples_reported >= 5 {
                                latency::percentiles(
                                    bridge.round_trip_samples.borrow().clone(),
                                    "ipc round-trip",
                                );
                                samples_reported = sample_count;
                            }

                            if let Some(target) = auto_click_count {
                                if target > 0 && bridge.round_trip_samples.borrow().len() >= target
                                {
                                    println!("\n=== Scripted round-trip benchmark complete ===");
                                    latency::percentiles(
                                        bridge.round_trip_samples.borrow().clone(),
                                        "ipc round-trip (scripted, final)",
                                    );
                                    elwt.exit();
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::AboutToWait => {
                    if auto_click_remaining > 0 && last_auto_click.elapsed() >= AUTO_CLICK_INTERVAL
                    {
                        bridge.trigger_synthetic_click();
                        auto_click_remaining -= 1;
                        last_auto_click = Instant::now();
                    }
                    window.request_redraw();
                }
                _ => {}
            }
        })
        .expect("event loop exited with an error");
}

fn fade_progress(fade_start: Option<Instant>) -> f32 {
    match fade_start {
        None => 1.0,
        Some(start) => {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            (elapsed_ms / FADE_DURATION_MS).min(1.0) as f32
        }
    }
}
