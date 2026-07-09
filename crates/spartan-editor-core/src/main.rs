mod cursor;
mod fixture;
mod gpu;
mod input;
mod latency;
mod text;

use spartan_editor_core::viewport::{self, Viewport};
use spartan_editor_core::{build, dap_session, editor_view, highlight, language, lsp, lsp_session};

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};
use winit::event::{ElementState, Event, KeyEvent, MouseButton, WindowEvent};
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
///
/// `highlighter`, when present, runs a real tree-sitter parse + highlight
/// pass over the windowed text before it's shaped (§75.11) -- deliberately
/// windowed, not whole-document, since `tree_sitter_highlight::Highlighter`'s
/// public API has no cheap way to restrict itself to a sub-range (see
/// `highlight.rs`'s doc comment for the real, verified finding that forced
/// this scope).
fn reshape_window(
    text_state: &mut text::TextState,
    editor: &editor_view::EditorView,
    viewport: &Viewport,
    highlighter: Option<&mut highlight::Highlighter>,
) {
    let windowed = viewport::windowed_text(&editor.document, viewport);
    match highlighter {
        Some(hl) => {
            let spans = hl.highlight(&windowed);
            text_state.set_text_highlighted(&windowed, &spans);
        }
        None => text_state.set_text(&windowed),
    }
}

/// Routes a completed edit to the cheapest correct `TextState` update,
/// viewport-aware. This is the crate's real extension of `render-spike`'s
/// own `apply_edit_effect`: a `Line(doc_line_i)` edit outside the current
/// viewport needs no redraw at all (a real virtualization win render-spike
/// never had the opportunity to take, since it always rendered the whole
/// document); one inside the viewport is translated to a window-local index
/// first. Structural edits and any edge case cosmic-text's windowed buffer
/// doesn't yet know about fall back to a full (but cheap, windowed) reshape.
///
/// A file with an active `highlighter` always takes that full windowed
/// reshape path, even for an in-window same-line edit -- a single edited
/// line can't be correctly re-highlighted in isolation (is this token
/// inside a string that started on a previous line within the window?).
/// This is a real, measured latency trade-off (§75.11), not a cost-free
/// addition.
fn apply_edit_effect(
    text_state: &mut text::TextState,
    editor: &editor_view::EditorView,
    viewport: &Viewport,
    effect: editor_view::EditEffect,
    highlighter: Option<&mut highlight::Highlighter>,
) -> bool {
    match effect {
        editor_view::EditEffect::Line(doc_line_i) => {
            let doc_len_lines = editor.document.len_lines();
            match viewport::to_local_line(doc_line_i, viewport, doc_len_lines) {
                None => false, // off-screen: genuinely nothing to redraw
                Some(local_i) if highlighter.is_none() && local_i < text_state.line_count() => {
                    match editor.document.line(doc_line_i) {
                        Ok(line_text) => {
                            text_state.set_line_text(local_i, strip_line_ending(&line_text));
                            true
                        }
                        Err(_) => {
                            reshape_window(text_state, editor, viewport, highlighter);
                            true
                        }
                    }
                }
                Some(_) => {
                    reshape_window(text_state, editor, viewport, highlighter);
                    true
                }
            }
        }
        editor_view::EditEffect::Structural => {
            reshape_window(text_state, editor, viewport, highlighter);
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
    // Optional 6th arg: a pre-built debug binary to launch under the
    // detected language's DAP adapter (F9/F5/F10/F11 below). Deliberately
    // NOT "build the project automatically" -- see §75.8 for why that's a
    // separate, real piece of work this pass doesn't attempt.
    let debug_binary_path: Option<PathBuf> = std::env::args()
        .nth(5)
        .and_then(|s| s.strip_prefix("--debug-binary:").map(PathBuf::from));

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
    // (see language.rs's doc comment) -- tree-sitter stays unwired. LSP
    // (§75.6) and DAP (§75.8) are both wired for real below.
    let mut lsp_session: Option<lsp_session::LspSession> = None;
    // (adapter command, pre-built binary, cwd, source path) -- captured here,
    // used on every F5 press below (cloned, not consumed, so the same
    // pre-built binary can be relaunched repeatedly), not launched
    // immediately (unlike LSP, a debug session should only start when the
    // user asks for one).
    let mut dap_launch_info: Option<(spartan_languages::CommandSpec, PathBuf, PathBuf, PathBuf)> =
        None;
    // (adapter command, project root, source path) -- the real build-system
    // integration §75.8 named as out of scope for that pass (§75.10):
    // captured only when no explicit `--debug-binary:` was given but a real
    // Cargo project is discoverable, so F5 can run a real `cargo build`
    // first instead of requiring a pre-built binary.
    let mut dap_build_info: Option<(spartan_languages::CommandSpec, PathBuf, PathBuf)> = None;
    // Real tree-sitter syntax highlighting (§75.11) -- only Rust is wired,
    // matching every prior pass's own precedent (LSP started with
    // rust-analyzer, DAP with lldb-dap). Any other grammar name is named
    // and left unhighlighted rather than silently guessed at.
    let mut highlighter: Option<highlight::Highlighter> = None;
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
                let file_path = Path::new(&fixture_path);
                let project_root = language::find_project_root(file_path, &profile.marker_files)
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
                if let Some(command) = &profile.lsp_command {
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
                if let Some(command) = &profile.dap_command {
                    if let Some(binary_path) = &debug_binary_path {
                        println!(
                            "DAP ready: {} on {} -- F9 toggles a breakpoint at the cursor, F5 launches",
                            command.program,
                            binary_path.display()
                        );
                        dap_launch_info = Some((
                            command.clone(),
                            binary_path.clone(),
                            project_root.clone(),
                            file_path.to_path_buf(),
                        ));
                    } else if profile.build_systems.iter().any(|s| s == "cargo")
                        && project_root.join("Cargo.toml").is_file()
                    {
                        println!(
                            "DAP ready: {} via a real `cargo build` (project root: {}) -- F9 \
                             toggles a breakpoint at the cursor, F5 builds and launches",
                            command.program,
                            project_root.display()
                        );
                        dap_build_info = Some((
                            command.clone(),
                            project_root.clone(),
                            file_path.to_path_buf(),
                        ));
                    } else {
                        println!(
                            "DAP available for {} but no --debug-binary:<path> was given and no \
                             cargo project was detected -- pass one to enable F5/F9/F10/F11 \
                             debugging",
                            command.program
                        );
                    }
                }
                if profile.tree_sitter_grammar == "tree-sitter-rust" {
                    println!("Syntax highlighting: real tree-sitter-rust (windowed, see §75.11)");
                    highlighter = Some(highlight::Highlighter::rust());
                } else {
                    println!(
                        "No real tree-sitter wiring for grammar {:?} yet -- only tree-sitter-rust \
                         is wired (§75.11)",
                        profile.tree_sitter_grammar
                    );
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

    // Real breakdown of where cold-open time actually goes, not a guess --
    // §75.5-§75.8 all named the ~575-620ms cold-open number as ~6x over
    // §39.1's <100ms target without ever measuring which step causes it.
    let t_setup_done = Instant::now();

    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    let t_event_loop = Instant::now();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Spartan editor-core")
            .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 700.0))
            .build(&event_loop)
            .expect("failed to create window"),
    );
    let t_window = Instant::now();

    // `FontSystem::new()` scans and parses every font on the system -- a
    // real, measured ~93-97ms cost (§75.9) with no dependency on the GPU
    // device/queue `GpuState::new()` is about to spend ~220-330ms creating.
    // Building it on its own thread lets that cost overlap with the async
    // GPU setup instead of paying both back-to-back on the same thread.
    let font_system_handle = thread::spawn(glyphon::FontSystem::new);

    let mut gpu_state = pollster::block_on(gpu::GpuState::new(window.clone()));
    let t_gpu = Instant::now();
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

    let font_system = font_system_handle
        .join()
        .expect("font system background thread panicked");
    let mut text_state = text::TextState::new(
        font_system,
        &gpu_state.device,
        &gpu_state.queue,
        gpu_state.config.format,
        gpu_state.size.width as f32,
        gpu_state.size.height as f32,
    );
    let t_text_state = Instant::now();
    // The key difference from render-spike: seeded with only the visible
    // window's text, not the whole document.
    reshape_window(&mut text_state, &editor, &viewport, highlighter.as_mut());
    let t_reshape = Instant::now();

    let cursor_renderer = cursor::CursorRenderer::new(&gpu_state.device, gpu_state.config.format);
    let t_cursor_renderer = Instant::now();

    // 150ms idle default per spec §2.3. This timer is polled from
    // `AboutToWait`, which only fires continuously because this crate
    // already runs `ControlFlow::Poll` unconditionally (for the benchmark
    // harness below) -- if a future pass switches to `ControlFlow::Wait`
    // for idle-CPU reasons, this debounce would silently stop firing once
    // the user stops generating other events.
    let mut lsp_debouncer = lsp::DidChangeDebouncer::new(Duration::from_millis(150));

    // Line-number breakpoints (1-indexed, DAP convention), toggled by F9 at
    // the cursor's current line -- the §39.2-sanctioned fallback for a v1
    // that doesn't yet have rope-anchored breakpoint persistence (see
    // §75.8). No live changes once a session is running, a named limitation.
    let mut breakpoints: Vec<i64> = Vec::new();
    let mut dap_session: Option<dap_session::DapSession> = None;
    // A real `cargo build` running on its own thread (§75.10) -- the
    // receiver half of a one-shot channel, polled non-blockingly each
    // frame, matching the `LspSession`/`DapSession` "never block the
    // render thread" pattern even though this itself isn't an ongoing
    // session.
    let mut pending_build: Option<
        mpsc::Receiver<(
            spartan_languages::CommandSpec,
            PathBuf,
            PathBuf,
            build::BuildResult,
        )>,
    > = None;

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
    // `WindowEvent::MouseInput` carries no position of its own (winit
    // reports cursor position and button state as separate events), so the
    // most recent `CursorMoved` position is tracked here for a click to
    // read back.
    let mut last_cursor_pos: (f32, f32) = (0.0, 0.0);

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
                            if let Some(session) = dap_session.take() {
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
                                reshape_window(
                                    &mut text_state,
                                    &editor,
                                    &viewport,
                                    highlighter.as_mut(),
                                );
                                window.request_redraw();
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            last_cursor_pos = (position.x as f32, position.y as f32);
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } => {
                            // `hit_test` expects coordinates relative to the
                            // text buffer's own origin, the same convention
                            // `cursor_pixel_pos` uses in reverse (see its own
                            // doc comment) -- so the window-space position
                            // needs the same offset subtracted first.
                            let local_x = last_cursor_pos.0 - text::TEXT_ORIGIN_X;
                            let local_y = last_cursor_pos.1 - text::TEXT_ORIGIN_Y;
                            if let Some((local_line, col_chars)) =
                                text_state.hit_test(local_x, local_y)
                            {
                                let doc_line = viewport::to_doc_line(local_line, &viewport);
                                editor.set_cursor_to_line_col(doc_line, col_chars);
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
                                    reshape_window(
                                        &mut text_state,
                                        &editor,
                                        &viewport,
                                        highlighter.as_mut(),
                                    );
                                    window.request_redraw();
                                }
                            }
                            Key::Named(NamedKey::PageUp) => {
                                let page = -(viewport.visible_lines as isize);
                                let doc_len_lines = editor.document.len_lines();
                                if viewport.scroll_by(page, doc_len_lines) {
                                    reshape_window(
                                        &mut text_state,
                                        &editor,
                                        &viewport,
                                        highlighter.as_mut(),
                                    );
                                    window.request_redraw();
                                }
                            }
                            Key::Named(NamedKey::F9) => {
                                let (cursor_line, _) = editor.cursor_line_col();
                                let line_1indexed = (cursor_line + 1) as i64;
                                if let Some(pos) =
                                    breakpoints.iter().position(|&l| l == line_1indexed)
                                {
                                    breakpoints.remove(pos);
                                    println!("Breakpoint removed at line {line_1indexed}");
                                } else {
                                    breakpoints.push(line_1indexed);
                                    println!("Breakpoint set at line {line_1indexed}");
                                }
                            }
                            Key::Named(NamedKey::F5) => {
                                // A session whose background thread has already exited
                                // (the debuggee ran to completion, or the launch sequence
                                // failed) is genuinely over -- treat it as gone so this
                                // press rebuilds/relaunches instead of silently trying to
                                // `Continue` a session nothing is listening on anymore.
                                if dap_session.as_ref().is_some_and(|s| s.is_finished()) {
                                    dap_session = None;
                                }

                                if let Some(session) = &dap_session {
                                    session.send_command(dap_session::DapCommand::Continue);
                                } else if pending_build.is_some() {
                                    println!("A build is already in progress -- please wait");
                                } else if let Some((command, binary_path, cwd, source_path)) =
                                    dap_launch_info.clone()
                                {
                                    println!(
                                        "Launching debug session: {} on {}",
                                        command.program,
                                        binary_path.display()
                                    );
                                    dap_session = Some(dap_session::DapSession::launch(
                                        &command,
                                        &binary_path,
                                        &cwd,
                                        &source_path,
                                        &breakpoints,
                                    ));
                                } else if let Some((command, project_root, source_path)) =
                                    dap_build_info.clone()
                                {
                                    println!(
                                        "Building {} with a real `cargo build`...",
                                        project_root.display()
                                    );
                                    let (build_tx, build_rx) = mpsc::channel();
                                    let build_project_root = project_root.clone();
                                    thread::spawn(move || {
                                        let result = build::build_debug_binary(&build_project_root);
                                        let _ =
                                            build_tx.send((command, project_root, source_path, result));
                                    });
                                    pending_build = Some(build_rx);
                                } else {
                                    println!(
                                        "No debug session available (no --debug-binary:<path> \
                                         given and no cargo project detected, or the detected \
                                         language has no dap_command)"
                                    );
                                }
                            }
                            Key::Named(NamedKey::F10) => {
                                if let Some(session) = &dap_session {
                                    session.send_command(dap_session::DapCommand::StepOver);
                                }
                            }
                            Key::Named(NamedKey::F11) => {
                                if let Some(session) = &dap_session {
                                    session.send_command(dap_session::DapCommand::StepInto);
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
                                        reshape_window(
                                            &mut text_state,
                                            &editor,
                                            &viewport,
                                            highlighter.as_mut(),
                                        );
                                    } else {
                                        apply_edit_effect(
                                            &mut text_state,
                                            &editor,
                                            &viewport,
                                            effect,
                                            highlighter.as_mut(),
                                        );
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
                                let ms = |from: Instant, to: Instant| {
                                    to.saturating_duration_since(from).as_secs_f64() * 1000.0
                                };
                                println!(
                                    "Cold-open: process start -> first presented frame = {:.2}ms",
                                    program_start.elapsed().as_secs_f64() * 1000.0
                                );
                                println!("Cold-open breakdown (real, measured, not estimated):");
                                println!(
                                    "  arg parsing / fixture load / language detect / LSP spawn = {:.2}ms",
                                    ms(program_start, t_setup_done)
                                );
                                println!(
                                    "  winit EventLoop::new()                                   = {:.2}ms",
                                    ms(t_setup_done, t_event_loop)
                                );
                                println!(
                                    "  window creation                                          = {:.2}ms",
                                    ms(t_event_loop, t_window)
                                );
                                println!(
                                    "  GpuState::new() (wgpu instance/adapter/device/surface)   = {:.2}ms",
                                    ms(t_window, t_gpu)
                                );
                                println!(
                                    "  TextState::new() (cosmic-text FontSystem + glyphon atlas) = {:.2}ms",
                                    ms(t_gpu, t_text_state)
                                );
                                println!(
                                    "  initial reshape_window() (windowed text shaping)          = {:.2}ms",
                                    ms(t_text_state, t_reshape)
                                );
                                println!(
                                    "  CursorRenderer::new()                                    = {:.2}ms",
                                    ms(t_reshape, t_cursor_renderer)
                                );
                                println!(
                                    "  first RedrawRequested (surface acquire -> present)        = {:.2}ms",
                                    ms(t_cursor_renderer, Instant::now())
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

                            // No debug UI exists yet -- real DAP stop/exit/error
                            // updates are surfaced to stdout here, matching the LSP
                            // diagnostics pattern above.
                            if let Some(session) = &dap_session {
                                for update in session.poll_updates() {
                                    match update {
                                        dap_session::DapUpdate::Stopped(lines) => {
                                            println!("DAP stopped:");
                                            for line in lines {
                                                println!("  {line}");
                                            }
                                        }
                                        dap_session::DapUpdate::Exited => {
                                            println!("DAP: program exited")
                                        }
                                        dap_session::DapUpdate::Error(e) => {
                                            println!("DAP error: {e}")
                                        }
                                    }
                                }
                            }

                            // A real `cargo build` triggered by F5 (§75.10) -- non-blocking
                            // poll, matching the LspSession/DapSession update pattern, even
                            // though this itself is a one-shot rather than an ongoing session.
                            if let Some(rx) = &pending_build {
                                if let Ok((command, cwd, source_path, result)) = rx.try_recv() {
                                    pending_build = None;
                                    match result {
                                        build::BuildResult::Success(binary_path) => {
                                            println!(
                                                "Build succeeded: {}",
                                                binary_path.display()
                                            );
                                            println!(
                                                "Launching debug session: {} on {}",
                                                command.program,
                                                binary_path.display()
                                            );
                                            dap_session = Some(dap_session::DapSession::launch(
                                                &command,
                                                &binary_path,
                                                &cwd,
                                                &source_path,
                                                &breakpoints,
                                            ));
                                        }
                                        build::BuildResult::Failure(diagnostics) => {
                                            println!("Build failed:");
                                            for d in diagnostics {
                                                println!("{d}");
                                            }
                                        }
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
                            let redrew =
                                apply_edit_effect(&mut text_state, &editor, &viewport, effect, None);
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
                            apply_edit_effect(&mut text_state, &editor, &viewport, effect, None);
                        }
                        cursor_bench_remaining -= 1;
                    } else if scroll_bench_remaining > 0 {
                        let doc_len_lines = editor.document.len_lines();
                        let direction = if viewport.scroll_line == 0 { 1isize } else { -1isize };
                        let page = direction * viewport.visible_lines as isize;
                        scroll_latency.note_key_event();
                        if viewport.scroll_by(page, doc_len_lines) {
                            reshape_window(&mut text_state, &editor, &viewport, None);
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
