mod cursor;
mod fixture;
mod gpu;
mod input;
mod latency;
mod text;

use render_spike::editor_view;
use std::sync::Arc;
use std::time::Instant;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;

/// `wgpu::Color` clear values are specified in linear space, but the chosen
/// surface format is sRGB (see `gpu.rs`'s format selection) -- passing a
/// perceptual/sRGB value straight through renders noticeably lighter than
/// intended (an intended dark 0.08 sRGB gray displayed as a medium-light
/// gray, found by comparing against an unmistakable pure-red clear color
/// during manual verification, not assumed from wgpu's docs alone).
fn srgb_to_linear(s: f64) -> f64 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn main() {
    let program_start = Instant::now();
    println!("=== render-spike -- Spike 0.1 GPU-half, first increment ===");

    let fixture_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "..\\..\\crates\\spartan-buffer\\src\\lib.rs".to_string());
    // Optional second arg: run N internally-scripted random-position inserts
    // (see `EditorView::insert_random`) instead of waiting for real keyboard
    // input, then print a final latency report and exit. This is explicitly
    // NOT real OS-level synthetic input (unlike the `enigo`-based tooling
    // used to verify Steps 4/5 by hand) -- it drives `Document`/`TextState`
    // directly on the same thread, named honestly as "scripted" throughout
    // this spike's output and its exit report, not oversold as real input.
    let bench_iters: Option<usize> = std::env::args().nth(2).and_then(|s| s.parse().ok());

    // `--synthetic:<lines>` generates rope-spike's exact corpus shape (§47.1,
    // `fixture::synthetic_file`) in-memory instead of reading a path from
    // disk, so both spikes' exit reports can measure the identical content.
    let initial_text = if let Some(spec) = fixture_path.strip_prefix("--synthetic:") {
        let lines: usize = spec
            .parse()
            .unwrap_or_else(|_| panic!("invalid --synthetic:<lines> value: {spec}"));
        fixture::synthetic_file(lines)
    } else {
        std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("failed to read {fixture_path}: {e}"))
    };
    println!(
        "Loaded {fixture_path} -- {} chars, {} lines",
        initial_text.chars().count(),
        initial_text.lines().count()
    );
    if let Some(n) = bench_iters {
        println!("Scripted latency benchmark: {n} internally-driven random-position inserts");
    }
    let mut editor = editor_view::EditorView::new(&initial_text);
    let mut bench_rng = rand::thread_rng();
    let mut bench_remaining = bench_iters.unwrap_or(0);

    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Spartan render-spike")
            .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 700.0))
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

    let mut text_state = text::TextState::new(
        &gpu_state.device,
        &gpu_state.queue,
        gpu_state.config.format,
        gpu_state.size.width as f32,
        gpu_state.size.height as f32,
    );
    text_state.set_text(&editor.text());

    let cursor_renderer = cursor::CursorRenderer::new(&gpu_state.device, gpu_state.config.format);

    // Capacity of 2000 mirrors rope-spike's typing-benchmark iteration count
    // (§47.1), so a scripted run of comparable size produces a comparable
    // sample count between the two spikes' reports.
    let mut latency_tracker = latency::LatencyTracker::new(2000);
    // Print a running report every N presented-with-an-edit frames rather
    // than every frame -- an earlier per-frame `eprintln!` used during Step 5
    // debugging flooded a redirected log to 379MB in seconds and starved the
    // process; this stays informative without repeating that mistake.
    const REPORT_EVERY: usize = 200;
    let mut samples_at_last_report = 0usize;
    let mut cold_open_reported = false;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);

            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    match event {
                        WindowEvent::CloseRequested => {
                            latency_tracker.report("input-to-photon (final)");
                            elwt.exit();
                        }
                        WindowEvent::Resized(new_size) => {
                            gpu_state.resize(new_size);
                            text_state.resize(new_size.width as f32, new_size.height as f32);
                        }
                        WindowEvent::KeyboardInput {
                            event: key_event, ..
                        } => {
                            if input::handle_key_event(&mut editor, &key_event) {
                                latency_tracker.note_key_event();
                                text_state.set_text(&editor.text());
                                window.request_redraw();
                            }
                        }
                        WindowEvent::RedrawRequested => {
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

                            text_state
                                .prepare(
                                    &gpu_state.device,
                                    &gpu_state.queue,
                                    gpu_state.size.width,
                                    gpu_state.size.height,
                                )
                                .expect("glyphon text prepare failed");

                            let (cursor_line, cursor_col) = editor.cursor_line_col();
                            let cursor_pixel_pos =
                                text_state.cursor_pixel_pos(cursor_line, cursor_col);
                            if let Some((rel_x, rel_y)) = cursor_pixel_pos {
                                cursor_renderer.update(
                                    &gpu_state.queue,
                                    cursor::CursorRect {
                                        x: text::TEXT_ORIGIN_X + rel_x,
                                        y: text::TEXT_ORIGIN_Y + rel_y,
                                        width: cursor::CURSOR_WIDTH_PX,
                                        height: text::LINE_HEIGHT,
                                    },
                                    cursor::ScreenSize {
                                        width: gpu_state.size.width as f32,
                                        height: gpu_state.size.height as f32,
                                    },
                                );
                            }

                            let mut encoder = gpu_state.device.create_command_encoder(
                                &wgpu::CommandEncoderDescriptor {
                                    label: Some("render-spike encoder"),
                                },
                            );
                            {
                                let mut pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("clear + text pass"),
                                        color_attachments: &[Some(
                                            wgpu::RenderPassColorAttachment {
                                                view: &view,
                                                resolve_target: None,
                                                ops: wgpu::Operations {
                                                    // Dark editor-background gray, gamma-corrected for the
                                                    // sRGB surface -- see srgb_to_linear's doc comment. Not
                                                    // pure black, so a black window vs. a rendering failure
                                                    // aren't visually indistinguishable during verification.
                                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                                        r: srgb_to_linear(0.08),
                                                        g: srgb_to_linear(0.08),
                                                        b: srgb_to_linear(0.09),
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
                                text_state.render(&mut pass).expect("glyphon render failed");
                                if cursor_pixel_pos.is_some() {
                                    cursor_renderer.render(&mut pass);
                                }
                            }
                            gpu_state.queue.submit(std::iter::once(encoder.finish()));
                            frame.present();
                            text_state.atlas.trim();

                            if !cold_open_reported {
                                println!(
                                    "Cold-open: process start -> first presented frame = {:.2}ms",
                                    program_start.elapsed().as_secs_f64() * 1000.0
                                );
                                cold_open_reported = true;
                            }

                            latency_tracker.note_frame_presented();
                            if latency_tracker.total_recorded() - samples_at_last_report
                                >= REPORT_EVERY
                            {
                                latency_tracker.report("input-to-photon");
                                samples_at_last_report = latency_tracker.total_recorded();
                            }

                            if let Some(total) = bench_iters {
                                if latency_tracker.total_recorded() >= total {
                                    println!(
                                        "\n=== Scripted benchmark complete ({total} inserts) ==="
                                    );
                                    latency_tracker.report("input-to-photon (scripted, final)");
                                    elwt.exit();
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::AboutToWait => {
                    if bench_remaining > 0 {
                        editor.insert_random(&mut bench_rng, "x");
                        latency_tracker.note_key_event();
                        text_state.set_text(&editor.text());
                        bench_remaining -= 1;
                    }
                    window.request_redraw();
                }
                _ => {}
            }
        })
        .expect("event loop exited with an error");
}
