mod cursor;
mod fixture;
mod gpu;
mod input;
mod latency;
mod text;

use spartan_editor_core::viewport::{self, Viewport};
use spartan_editor_core::{editor_view, language, lsp, lsp_session};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowBuilder;

/// `wgpu::Color` clear values are specified in linear space, but the chosen
/// surface format is sRGB -- passing a perceptual/sRGB value straight
/// through renders noticeably lighter than intended. Promoted from
/// `spikes/render-spike/src/main.rs` (§39.1, §47.9) verbatim.
fn srgb_to_linear(s: f64) -> f64 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// `Document::line()` (ropey) includes the line's trailing terminator, but
/// cosmic-text's `BufferLine`s never do. Promoted verbatim.
fn strip_line_ending(s: &str) -> &str {
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

/// Re-slices the document to the viewport's current visible range and fully
/// reshapes it -- cheap regardless of document size, since the slice is
/// bounded by `viewport.visible_lines` (~40-60), not `document.len_lines()`
/// (up to 50,000 in the benchmark fixture). This is the operation that
/// replaces `render-spike`'s `text_state.set_text(&editor.text())` (which
/// always passed the *entire* document) at every call site.
fn reshape_window(
    text_state: &mut text::TextState,
    editor: &editor_view::EditorView,
    viewport: &Viewport,
) {
    let windowed = viewport::windowed_text(&editor.document, viewport);
    text_state.set_text(&windowed);
}

/// Routes a completed edit to the cheapest correct `TextState` update,
/// viewport-aware. This is the crate's real extension of `render-spike`'s
/// own `apply_edit_effect`: a `Line(doc_line_i)` edit outside the current
/// viewport needs no redraw at all (a real virtualization win render-spike
/// never had the opportunity to take, since it always rendered the whole
/// document); one inside the viewport is translated to a window-local index
/// first. Structural edits and any edge case cosmic-text's windowed buffer
/// doesn't yet know about fall back to a full (but cheap, windowed) reshape.
fn apply_edit_effect(
    text_state: &mut text::TextState,
    editor: &editor_view::EditorView,
    viewport: &Viewport,
    effect: editor_view::EditEffect,
) -> bool {
    match effect {
        editor_view::EditEffect::Line(doc_line_i) => {
            let doc_len_lines = editor.document.len_lines();
            match viewport::to_local_line(doc_line_i, viewport, doc_len_lines) {
                None => false, // off-screen: genuinely nothing to redraw
                Some(local_i) if local_i < text_state.line_count() => {
                    match editor.document.line(doc_line_i) {
                        Ok(line_text) => {
                            text_state.set_line_text(local_i, strip_line_ending(&line_text));
                            true
                        }
                        Err(_) => {
                            reshape_window(text_state, editor, viewport);
                            true
                        }
                    }
                }
                Some(_) => {
                    reshape_window(text_state, editor, viewport);
                    true
                }
            }
        }
        editor_view::EditEffect::Structural => {
            reshape_window(text_state, editor, viewport);
            true
        }
        editor_view::EditEffect::None => false,
    }
}

fn main() {
    let program_start = Instant::now();
    println!("=== spartan-editor-core -- real Tier 1 core-engine increment ===");

    let fixture_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "..\\..\\crates\\spartan-buffer\\src\\lib.rs".to_string());
    // Optional 2nd/3rd/4th args: N internally-scripted random-position edit
    // iterations, then M sequential cursor-adjacent typing iterations, then
    // K scroll iterations, run sequentially, printing a final report and
    // exiting -- same "scripted, not real OS input" honesty as
    // `render-spike`'s own bench mode. The random-position phase and the
    // cursor-adjacent phase measure genuinely different things at 50k-line
    // scale: random positions land inside the ~34-60 line visible window
    // only ~0.1% of the time (a real, near-zero-cost no-op the rest of the
    // time -- see `in_window_edits`/`off_window_edits` below), so it alone
    // can't characterize "how fast is editing where the user can actually
    // see," which is what virtualization is meant to help with. The cursor
    // phase measures exactly that: the cursor starts at line 0 and this
    // phase never scrolls, so every one of its edits is a real in-window
    // reshape, however large the surrounding document is.
    let bench_edit_iters: Option<usize> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    let bench_cursor_iters: Option<usize> = std::env::args().nth(3).and_then(|s| s.parse().ok());
    let bench_scroll_iters: Option<usize> = std::env::args().nth(4).and_then(|s| s.parse().ok());

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

    // First real combination of spartan-buffer + rendering + spartan-languages
    // (see language.rs's doc comment) -- tree-sitter stays unwired; LSP is
    // wired for real for the first time below (§75.6). DAP stays unwired:
    // no debugging UI (breakpoints, stepping, variable inspection) exists
    // anywhere in this crate yet, so it's a deliberately separate, later pass.
    let mut lsp_session: Option<lsp_session::LspSession> = None;
    if !fixture_path.starts_with("--synthetic:") {
        match language::detect_language_for_file(Path::new(&fixture_path)) {
            Some(profile) => {
                println!(
                    "Detected language: {} (tree-sitter grammar: {}, LSP: {}, DAP: {})",
                    profile.id,
                    profile.tree_sitter_grammar,
                    profile.lsp_command.is_some(),
                    profile.dap_command.is_some()
                );
                if let Some(command) = &profile.lsp_command {
                    let file_path = Path::new(&fixture_path);
                    let project_root =
                        language::find_project_root(file_path, &profile.marker_files)
                            .unwrap_or_else(|| {
                                // Named, real limitation (see the crate README):
                                // no marker file found in any ancestor means no
                                // coherent project root, so this falls back to
                                // single-file mode -- rust-analyzer still runs,
                                // but with meaningfully worse diagnostics.
                                file_path
                                    .parent()
                                    .map(Path::to_path_buf)
                                    .unwrap_or_else(|| PathBuf::from("."))
                            });
                    println!(
                        "Starting real LSP session: {} (project root: {})",
                        command.program,
                        project_root.display()
                    );
                    match lsp_session::LspSession::spawn(
                        command,
                        &project_root,
                        file_path,
                        &initial_text,
                    ) {
                        Ok(session) => lsp_session = Some(session),
                        Err(e) => {
                            println!("Failed to start LSP session ({}): {e}", command.program)
                        }
                    }
                }
            }
            None => println!("No language profile detected for {fixture_path}"),
        }
    }

    let mut editor = editor_view::EditorView::new(&initial_text);
    let mut bench_rng = rand::thread_rng();
    let mut edit_bench_remaining = bench_edit_iters.unwrap_or(0);
    let mut cursor_bench_remaining = bench_cursor_iters.unwrap_or(0);
    let mut scroll_bench_remaining = bench_scroll_iters.unwrap_or(0);
    if let Some(n) = bench_edit_iters {
        println!("Scripted random-position edit benchmark: {n} internally-driven inserts");
    }
    if let Some(n) = bench_cursor_iters {
        println!("Scripted cursor-adjacent typing benchmark: {n} internally-driven inserts");
    }
    if let Some(n) = bench_scroll_iters {
        println!("Scripted scroll benchmark: {n} internally-driven PageDown scrolls");
    }

    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Spartan editor-core")
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

    // Computed once from initial window size -- not recomputed on resize,
    // a named simplification (see the crate README).
    let visible_lines = ((gpu_state.size.height as f32 - 2.0 * text::TEXT_ORIGIN_Y)
        / text::LINE_HEIGHT)
        .floor()
        .max(1.0) as usize;
    let mut viewport = Viewport::new(visible_lines);
    println!(
        "Viewport: {visible_lines} visible lines (vs. {} total in the document)",
        editor.document.len_lines()
    );

    let mut text_state = text::TextState::new(
        &gpu_state.device,
        &gpu_state.queue,
        gpu_state.config.format,
        gpu_state.size.width as f32,
        gpu_state.size.height as f32,
    );
    // The key difference from render-spike: seeded with only the visible
    // window's text, not the whole document.
    reshape_window(&mut text_state, &editor, &viewport);

    let cursor_renderer = cursor::CursorRenderer::new(&gpu_state.device, gpu_state.config.format);

    // 150ms idle default per spec §2.3. This timer is polled from
    // `AboutToWait`, which only fires continuously because this crate
    // already runs `ControlFlow::Poll` unconditionally (for the benchmark
    // harness below) -- if a future pass switches to `ControlFlow::Wait`
    // for idle-CPU reasons, this debounce would silently stop firing once
    // the user stops generating other events.
    let mut lsp_debouncer = lsp::DidChangeDebouncer::new(Duration::from_millis(150));

    let mut edit_latency = latency::LatencyTracker::new(2000);
    let mut cursor_latency = latency::LatencyTracker::new(2000);
    let mut scroll_latency = latency::LatencyTracker::new(200);
    let mut in_window_edits = 0usize;
    let mut off_window_edits = 0usize;
    const REPORT_EVERY: usize = 200;
    let mut samples_at_last_report = 0usize;
    let mut cold_open_reported = false;
    let mut edit_bench_reported = false;
    let mut cursor_bench_reported = false;
    let mut scroll_bench_reported = false;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);

            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    match event {
                        WindowEvent::CloseRequested => {
                            println!("\n=== Final reports ===");
                            println!(
                                "In-window edits: {in_window_edits} (real reshape) | \
                                 off-window edits: {off_window_edits} (no redraw needed)"
                            );
                            edit_latency.report("input-to-photon (edits, random-position)");
                            cursor_latency.report("input-to-photon (edits, cursor-adjacent)");
                            scroll_latency.report("scroll re-shape");
                            if let Some(session) = lsp_session.take() {
                                session.shutdown();
                            }
                            elwt.exit();
                        }
                        WindowEvent::Resized(new_size) => {
                            gpu_state.resize(new_size);
                            text_state.resize(new_size.width as f32, new_size.height as f32);

                            // Recomputes how many lines are visible from the new window
                            // height -- previously fixed at startup only (a named
                            // limitation, §75.5), meaning a resized window's viewport size
                            // silently drifted from what was actually on screen.
                            let new_visible_lines = ((new_size.height as f32
                                - 2.0 * text::TEXT_ORIGIN_Y)
                                / text::LINE_HEIGHT)
                                .floor()
                                .max(1.0) as usize;
                            if new_visible_lines != viewport.visible_lines {
                                viewport.visible_lines = new_visible_lines;
                                let (cursor_line, _) = editor.cursor_line_col();
                                let doc_len_lines = editor.document.len_lines();
                                viewport.ensure_visible(cursor_line, doc_len_lines);
                                reshape_window(&mut text_state, &editor, &viewport);
                                window.request_redraw();
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } => match &key_event.logical_key {
                            Key::Named(NamedKey::PageDown) => {
                                let page = viewport.visible_lines as isize;
                                let doc_len_lines = editor.document.len_lines();
                                if viewport.scroll_by(page, doc_len_lines) {
                                    reshape_window(&mut text_state, &editor, &viewport);
                                    window.request_redraw();
                                }
                            }
                            Key::Named(NamedKey::PageUp) => {
                                let page = -(viewport.visible_lines as isize);
                                let doc_len_lines = editor.document.len_lines();
                                if viewport.scroll_by(page, doc_len_lines) {
                                    reshape_window(&mut text_state, &editor, &viewport);
                                    window.request_redraw();
                                }
                            }
                            _ => {
                                let effect = input::handle_key_event(&mut editor, &key_event);
                                if effect != editor_view::EditEffect::None {
                                    edit_latency.note_key_event();
                                    lsp_debouncer.on_edit();
                                    let (cursor_line, _) = editor.cursor_line_col();
                                    let doc_len_lines = editor.document.len_lines();
                                    if viewport.ensure_visible(cursor_line, doc_len_lines) {
                                        // The cursor moved outside the current window (e.g.
                                        // Enter near the bottom edge) -- the viewport itself
                                        // scrolled, so one full reshape against the new window
                                        // covers both the scroll and the edit's own visual
                                        // change, rather than reshaping twice.
                                        reshape_window(&mut text_state, &editor, &viewport);
                                    } else {
                                        apply_edit_effect(&mut text_state, &editor, &viewport, effect);
                                    }
                                    window.request_redraw();
                                }
                            }
                        },
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

                            let (cursor_doc_line, cursor_col) = editor.cursor_line_col();
                            let doc_len_lines = editor.document.len_lines();
                            let cursor_pixel_pos =
                                viewport::to_local_line(cursor_doc_line, &viewport, doc_len_lines)
                                    .and_then(|local_line| {
                                        text_state.cursor_pixel_pos(local_line, cursor_col)
                                    });
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
                                    label: Some("spartan-editor-core encoder"),
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

                            // No diagnostics UI exists yet (matching how detected
                            // language is also just printed) -- real LSP diagnostics
                            // are surfaced to stdout here.
                            if let Some(session) = &lsp_session {
                                for update in session.poll_updates() {
                                    let lsp_session::LspUpdate::Diagnostics(lines) = update;
                                    println!("LSP diagnostics ({} item(s)):", lines.len());
                                    for line in lines {
                                        println!("  {line}");
                                    }
                                }
                            }

                            edit_latency.note_frame_presented();
                            cursor_latency.note_frame_presented();
                            scroll_latency.note_frame_presented();
                            if edit_latency.total_recorded() - samples_at_last_report >= REPORT_EVERY
                            {
                                edit_latency.report("input-to-photon (edits)");
                                samples_at_last_report = edit_latency.total_recorded();
                            }

                            // One-shot guards: without `*_bench_reported`, this block would
                            // re-fire on every subsequent `RedrawRequested` once the target
                            // count is reached (a real bug caught only by actually running
                            // the scripted benchmark, not by inspection -- the exit report
                            // printed dozens of times before this fix). Phases chain
                            // edit -> cursor -> scroll; only the last one actually
                            // configured for this run calls `elwt.exit()`.
                            let has_cursor_phase = bench_cursor_iters.is_some();
                            let has_scroll_phase = bench_scroll_iters.is_some();
                            if !edit_bench_reported {
                                if let Some(total) = bench_edit_iters {
                                    if edit_latency.total_recorded() >= total {
                                        println!("\n=== Scripted random-position edit benchmark complete ({total} inserts) ===");
                                        println!(
                                            "In-window edits: {in_window_edits} (real reshape) | \
                                             off-window edits: {off_window_edits} (no redraw needed)"
                                        );
                                        edit_latency.report("input-to-photon (edits, random-position, final)");
                                        edit_bench_reported = true;
                                        if !has_cursor_phase && !has_scroll_phase {
                                            elwt.exit();
                                        }
                                    }
                                }
                            }
                            if !cursor_bench_reported {
                                if let Some(total) = bench_cursor_iters {
                                    if cursor_latency.total_recorded() >= total {
                                        println!("\n=== Scripted cursor-adjacent typing benchmark complete ({total} inserts) ===");
                                        cursor_latency.report("input-to-photon (edits, cursor-adjacent, final)");
                                        cursor_bench_reported = true;
                                        if !has_scroll_phase {
                                            elwt.exit();
                                        }
                                    }
                                }
                            }
                            if !scroll_bench_reported {
                                if let Some(total) = bench_scroll_iters {
                                    if scroll_latency.total_recorded() >= total {
                                        println!("\n=== Scripted scroll benchmark complete ({total} scrolls) ===");
                                        scroll_latency.report("scroll re-shape (scripted, final)");
                                        scroll_bench_reported = true;
                                        elwt.exit();
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::AboutToWait => {
                    if edit_bench_remaining > 0 {
                        let effect = editor.insert_random(&mut bench_rng, "x");
                        if effect != editor_view::EditEffect::None {
                            edit_latency.note_key_event();
                            let redrew = apply_edit_effect(&mut text_state, &editor, &viewport, effect);
                            if redrew {
                                in_window_edits += 1;
                            } else {
                                off_window_edits += 1;
                            }
                        }
                        edit_bench_remaining -= 1;
                    } else if cursor_bench_remaining > 0 {
                        // Deliberately never inserts "\n": the cursor stays on line 0 for
                        // this whole phase (growing it), which keeps every single edit
                        // in-window regardless of iteration count -- a clean, uncontended
                        // measurement of the `EditEffect::Line` fast path at 50k-line
                        // document scale, free of the "did the cursor scroll out of the
                        // (not-yet-implemented) auto-follow window" question.
                        let effect = editor.insert_at_cursor("x");
                        if effect != editor_view::EditEffect::None {
                            cursor_latency.note_key_event();
                            apply_edit_effect(&mut text_state, &editor, &viewport, effect);
                        }
                        cursor_bench_remaining -= 1;
                    } else if scroll_bench_remaining > 0 {
                        let doc_len_lines = editor.document.len_lines();
                        let direction = if viewport.scroll_line == 0 { 1isize } else { -1isize };
                        let page = direction * viewport.visible_lines as isize;
                        scroll_latency.note_key_event();
                        if viewport.scroll_by(page, doc_len_lines) {
                            reshape_window(&mut text_state, &editor, &viewport);
                        }
                        scroll_bench_remaining -= 1;
                    }
                    // Polled every tick because this crate runs `ControlFlow::Poll`
                    // unconditionally (see `lsp_debouncer`'s declaration comment).
                    if lsp_debouncer.should_dispatch_now() {
                        if let Some(session) = &lsp_session {
                            session.notify_edit(editor.text());
                        }
                    }
                    window.request_redraw();
                }
                _ => {}
            }
        })
        .expect("event loop exited with an error");
}
