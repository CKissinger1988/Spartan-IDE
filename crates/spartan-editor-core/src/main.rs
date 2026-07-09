mod cursor;
mod fixture;
mod gpu;
mod input;
mod latency;
mod selection;
mod text;
mod webview_bridge;

use mode_toggle::AppMode;
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use spartan_editor_core::viewport::{self, Viewport};
use spartan_editor_core::{
    accessibility, build, command_palette, dap_session, editor_view, file_tree, git_panel,
    gui_bridge, highlight, language, lsp, lsp_session, mode_toggle, tab_bar,
};

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use winit::event::{ElementState, Event, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::EventLoopBuilder;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::WindowBuilder;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::Rect;

/// Promoted verbatim from `spikes/ui-shell-spike/src/main.rs` (§47.11): a
/// real, documented fix for a real bug that spike found only by testing
/// keyboard input after clicking the embedded WebView, not predicted in
/// advance -- once wry's child `WebView2` control takes keyboard focus,
/// clicking elsewhere in this *same top-level window's* native area does
/// not return keyboard focus to winit on Windows. `winit::window::Window::
/// focus_window()` alone does not fix this (tested in that spike); a
/// direct Win32 `SetFocus` call does. Windows/WebView2-only, matching this
/// whole bridge's scope (`wry` uses a different backend -- WebKitGTK -- on
/// Linux, where this exact focus-stealing bug, if it exists at all, hasn't
/// been investigated in this project).
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

/// The real screen-space region Design mode's embedded WebView occupies
/// (§6.1, task #12) -- exactly the main editor's own content area (right
/// of the sidebar, below the tab bar/mode-toggle row), so switching modes
/// swaps *what* fills that region without moving or resizing it.
fn design_content_bounds(
    window: &winit::window::Window,
    size: winit::dpi::PhysicalSize<u32>,
) -> Rect {
    let scale = window.scale_factor();
    Rect {
        position: LogicalPosition::new(text::TEXT_ORIGIN_X as f64, text::TAB_BAR_HEIGHT as f64)
            .into(),
        size: LogicalSize::new(
            (size.width as f64 / scale - text::TEXT_ORIGIN_X as f64).max(0.0),
            (size.height as f64 / scale - text::TAB_BAR_HEIGHT as f64).max(0.0),
        )
        .into(),
    }
}

/// A zero-size `Rect` at the same position `design_content_bounds` uses --
/// used to effectively hide the WebView while leaving Design mode, rather
/// than destroying and recreating it on every mode switch. A real,
/// deliberate cost/complexity tradeoff: a hidden, zero-size WebView still
/// exists as a real child control, but occupies no screen space and can't
/// be clicked or receive real input.
fn design_hidden_bounds() -> Rect {
    Rect {
        position: LogicalPosition::new(text::TEXT_ORIGIN_X as f64, text::TAB_BAR_HEIGHT as f64)
            .into(),
        size: LogicalSize::new(0.0, 0.0).into(),
    }
}

/// Ensures a WebView exists and is shown at the real content-area bounds
/// (creating it lazily on first use), then refreshes its content, if
/// `mode` is `Design`; otherwise hides an already-existing one. Called
/// after every real mode assignment (Ctrl+1/2/3, a mode-toggle click, a
/// command-palette mode-switch command) -- not just ones that specifically
/// target Design -- so leaving Design mode reliably hides the WebView
/// again via this one shared code path, not a separate hide call at every
/// site that could possibly change `mode` away from it.
fn sync_webview_for_mode(
    bridge: &mut Option<webview_bridge::WebviewBridge>,
    component_tree_request: &mut Option<gui_bridge::ComponentTreeRequest>,
    mode: AppMode,
    window: &winit::window::Window,
    size: winit::dpi::PhysicalSize<u32>,
    file: &OpenFile,
) {
    if mode == AppMode::Design {
        let b = bridge.get_or_insert_with(|| {
            webview_bridge::WebviewBridge::new(window, design_content_bounds(window, size))
        });
        b.set_bounds(design_content_bounds(window, size));
        sync_webview_content(b, component_tree_request, file);
    } else if let Some(b) = bridge {
        b.set_bounds(design_hidden_bounds());
    }
}

/// Pushes the real active file's path/component-file check into the
/// WebView and, for a real component file, spawns a real §75.41
/// `gui_bridge` request for its component tree (showing a real "parsing"
/// state immediately, resolved later by `AboutToWait`'s own poll of
/// `component_tree_request`). Called whenever a WebView already exists
/// and Design mode is (or is becoming) the current mode -- a real, named
/// limitation: this must be called explicitly at every "active file
/// changed while potentially still in Design mode" site (tab bar click,
/// sidebar click, closing a file bringing a different one to the front),
/// since there's no generic "on active file changed" hook in this crate
/// to hang it off of instead.
fn sync_webview_content(
    bridge: &webview_bridge::WebviewBridge,
    component_tree_request: &mut Option<gui_bridge::ComponentTreeRequest>,
    file: &OpenFile,
) {
    let is_component_file = gui_bridge::is_component_file(&file.label);
    bridge.push_file_info(&file.label, is_component_file);
    if is_component_file {
        bridge.push_component_tree_loading();
        *component_tree_request = Some(gui_bridge::spawn_component_tree_request(Path::new(
            &file.label,
        )));
    } else {
        // Real, live bug fixed here, found only by testing a real
        // file-switch away from a component file, not by inspection: an
        // earlier version just cleared `component_tree_request` (correctly
        // canceling any in-flight fetch) but never told the WebView to
        // stop showing the *previous* file's stale tree underneath the
        // now-updated file-info line.
        *component_tree_request = None;
        bridge.push_component_tree_not_applicable();
    }
}

/// Thin wrapper for call sites that only have `Option<&WebviewBridge>` (no
/// mode-transition logic needed) -- an "active file changed" site where
/// Design mode may already be showing.
fn sync_webview_file_info(
    bridge: &Option<webview_bridge::WebviewBridge>,
    component_tree_request: &mut Option<gui_bridge::ComponentTreeRequest>,
    mode: AppMode,
    file: &OpenFile,
) {
    if mode != AppMode::Design {
        return;
    }
    if let Some(bridge) = bridge {
        sync_webview_content(bridge, component_tree_request, file);
    }
}

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

/// Real navigation-and-selection interaction rule (§75.18, extended by
/// §75.22 for Home/End/Ctrl+Arrow), matching conventional editor behavior:
/// Shift+<key> extends the active selection (arming one at the current
/// cursor first if none exists yet); a plain (non-Shift, non-Ctrl) Left/
/// Right press while a selection exists collapses it to the selection's
/// start/end instead of moving further from wherever the cursor last was.
/// Every other case (Up/Down, Home/End, Ctrl+Left/Right, Ctrl+Home/End)
/// clears the selection and still moves from the (now-collapsed) cursor,
/// since there's no single "correct" boundary-collapse target for those the
/// way there is for a plain horizontal arrow. Returns whether anything
/// visually changed (cursor position or selection state), so the caller
/// knows whether a redraw is warranted.
fn handle_navigation_key(
    editor: &mut editor_view::EditorView,
    key: NamedKey,
    shift: bool,
    ctrl: bool,
) -> bool {
    let do_move = |editor: &mut editor_view::EditorView| match key {
        NamedKey::ArrowLeft if ctrl => editor.move_word_left(),
        NamedKey::ArrowLeft => editor.move_left(),
        NamedKey::ArrowRight if ctrl => editor.move_word_right(),
        NamedKey::ArrowRight => editor.move_right(),
        NamedKey::ArrowUp => editor.move_up(),
        NamedKey::ArrowDown => editor.move_down(),
        NamedKey::Home if ctrl => editor.move_to_document_start(),
        NamedKey::Home => editor.move_to_line_start(),
        NamedKey::End if ctrl => editor.move_to_document_end(),
        NamedKey::End => editor.move_to_line_end(),
        _ => unreachable!("handle_navigation_key called with a non-navigation key"),
    };

    if shift {
        editor.start_selection_if_needed();
        return do_move(editor);
    }

    if let Some((start, end)) = editor.selection_range() {
        editor.selection_anchor = None;
        return match key {
            NamedKey::ArrowLeft if !ctrl => {
                editor.cursor = start;
                true
            }
            NamedKey::ArrowRight if !ctrl => {
                editor.cursor = end;
                true
            }
            _ => do_move(editor),
        };
    }

    do_move(editor)
}

/// One open file's full editing state -- everything that used to be flat
/// `main()` locals before multi-file support (§75.15), now indexed through
/// `files[active]` instead. `TextState` (the actual GPU-backed cosmic-text
/// buffer/atlas) deliberately stays a single shared instance in `main()`,
/// not duplicated per file -- switching the active file just reshapes the
/// same `TextState` against the newly active file's own `editor`/
/// `viewport`/`highlighter`, avoiding N times the GPU atlas memory for N
/// open files, which a naive "one TextState per file" design would cost.
struct OpenFile {
    /// Display label -- the original CLI argument (a real path, or
    /// `--synthetic:<n>`), not necessarily canonicalized.
    label: String,
    editor: editor_view::EditorView,
    /// Constructed with a placeholder `visible_lines` of 0 by `open_file`
    /// (file loading happens before `GpuState::new()`, to preserve the
    /// existing cold-open breakdown's timing semantics -- see `main`'s own
    /// comment) and patched to the real value once the window size is
    /// known, right after `GpuState::new()` returns.
    viewport: Viewport,
    highlighter: Option<highlight::Highlighter>,
    lsp_session: Option<lsp_session::LspSession>,
    lsp_debouncer: lsp::DidChangeDebouncer,
    /// Line-number breakpoints (1-indexed, DAP convention) for *this* file
    /// specifically -- moved from a single flat `Vec<i64>` (§75.8) into
    /// per-file storage here, fixing a real, previously-latent bug: a bare
    /// line number has no file association, so it was silently ambiguous
    /// the moment more than one file could be open at once.
    breakpoints: Vec<i64>,
    dap_launch_info: Option<(spartan_languages::CommandSpec, PathBuf, PathBuf, PathBuf)>,
    dap_build_info: Option<(spartan_languages::CommandSpec, PathBuf, PathBuf)>,
    /// Real, per-file unsaved-changes tracking (§75.16) -- set on any
    /// non-`EditEffect::None` edit, cleared on a successful `Ctrl+S` save.
    /// `label.starts_with("--synthetic:")` files are never saveable (they
    /// have no real path), so `dirty` on one is display-only.
    dirty: bool,
}

/// Loads one file (or `--synthetic:<n>`), detects its language, and spawns
/// its own real LSP session / captures its own DAP info / builds its own
/// highlighter -- exactly the setup `main()` used to do once, inline, for
/// the single file it opened (§75.5-§75.11). Every open file gets this
/// treatment now, including a real, named cost this pass doesn't try to
/// hide: each file with an LSP-capable profile spawns its *own*
/// language-server process, even if two open files share a project root
/// and could in principle share one session -- `LspSession` (§75.6) is
/// hardcoded to exactly one file's `didOpen`/`didChange` lifecycle, and
/// multiplexing it across files is real, separate future work, not
/// attempted here (see the crate README).
fn open_file(label: &str, debug_binary_path: Option<&PathBuf>) -> OpenFile {
    let initial_text = if let Some(spec) = label.strip_prefix("--synthetic:") {
        let lines: usize = spec
            .parse()
            .unwrap_or_else(|_| panic!("invalid --synthetic:<lines> value: {spec}"));
        fixture::synthetic_file(lines)
    } else {
        std::fs::read_to_string(label).unwrap_or_else(|e| panic!("failed to read {label}: {e}"))
    };
    println!(
        "Loaded {label} -- {} chars, {} lines",
        initial_text.chars().count(),
        initial_text.lines().count()
    );

    // Real secrets-detection warning (§9, §36, task #6) -- the "Secrets
    // scanning pass" §9 calls for, applied at the one real, honest call
    // site that exists today (no Leo/`ModelProvider` tool-execution layer
    // is built yet, so there's no "before any diff is shown to Leo"
    // moment to hook into -- see `spartan-security`'s own doc comment).
    // A whole-file, whole-document scan is correct here (unlike this
    // crate's windowed-only highlighting/tree-sitter passes): a secret
    // anywhere in the file is worth warning about, not just one currently
    // scrolled into view.
    let secret_findings = spartan_security::scan(&initial_text);
    if !secret_findings.is_empty() {
        println!(
            "WARNING: {label} appears to contain {} possible secret(s): {}",
            secret_findings.len(),
            secret_findings
                .iter()
                .map(|f| format!("{:?}", f.kind))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let mut lsp_session = None;
    let mut dap_launch_info = None;
    let mut dap_build_info = None;
    let mut highlighter = None;
    if !label.starts_with("--synthetic:") {
        match language::detect_language_for_file(Path::new(label)) {
            Some(profile) => {
                println!(
                    "Detected language: {} (tree-sitter grammar: {}, LSP: {}, DAP: {})",
                    profile.id,
                    profile.tree_sitter_grammar,
                    profile.lsp_command.is_some(),
                    profile.dap_command.is_some()
                );
                let file_path = Path::new(label);
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
                    if let Some(binary_path) = debug_binary_path {
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
                match profile.tree_sitter_grammar.as_str() {
                    "tree-sitter-rust" => {
                        println!(
                            "Syntax highlighting: real tree-sitter-rust (windowed, see §75.11)"
                        );
                        highlighter = Some(highlight::Highlighter::rust());
                    }
                    "tree-sitter-typescript" => {
                        println!(
                            "Syntax highlighting: real tree-sitter-typescript (windowed, see §75.29)"
                        );
                        highlighter = Some(highlight::Highlighter::typescript());
                    }
                    "tree-sitter-python" => {
                        println!(
                            "Syntax highlighting: real tree-sitter-python (windowed, see §75.29)"
                        );
                        highlighter = Some(highlight::Highlighter::python());
                    }
                    "tree-sitter-java" => {
                        println!(
                            "Syntax highlighting: real tree-sitter-java (windowed, see §75.29)"
                        );
                        highlighter = Some(highlight::Highlighter::java());
                    }
                    "tree-sitter-go" => {
                        println!("Syntax highlighting: real tree-sitter-go (windowed, see §75.29)");
                        highlighter = Some(highlight::Highlighter::go());
                    }
                    other => {
                        println!(
                            "No real tree-sitter wiring for grammar {other:?} yet -- Kotlin is a \
                             real, blocked gap (see §75.29): no available tree-sitter-0.25-compatible \
                             Kotlin grammar crate ships a bundled highlights query"
                        );
                    }
                }
            }
            None => println!("No language profile detected for {label}"),
        }
    }

    OpenFile {
        label: label.to_string(),
        editor: editor_view::EditorView::new(&initial_text),
        viewport: Viewport::new(0),
        highlighter,
        lsp_session,
        lsp_debouncer: lsp::DidChangeDebouncer::new(Duration::from_millis(150)),
        breakpoints: Vec::new(),
        dap_launch_info,
        dap_build_info,
        dirty: false,
    }
}

/// Real file tree sidebar root (§75.26, task #24): reuses exactly the same
/// project-root detection an LSP session already needs (`open_file`'s own
/// `language::find_project_root` call), rather than a second, separate
/// notion of "root" -- if a real project marker file is found, the sidebar
/// shows the same root rust-analyzer (or whichever language server) is
/// actually analyzing; otherwise it falls back to the file's own parent
/// directory, matching `open_file`'s own single-file-mode fallback.
/// `None` only for a `--synthetic:<n>` label, which has no real path to
/// derive a root from at all.
fn sidebar_root(label: &str) -> Option<PathBuf> {
    if label.starts_with("--synthetic:") {
        return None;
    }
    let file_path = Path::new(label);
    let marker_files = language::detect_language_for_file(file_path)
        .map(|profile| profile.marker_files)
        .unwrap_or_default();
    language::find_project_root(file_path, &marker_files)
        .or_else(|| file_path.parent().map(Path::to_path_buf))
}

/// Resolves a real filesystem path (from a file tree sidebar click) to an
/// already-open file's index, if any -- clicking a file that's already
/// open should switch to it, not spawn a second `OpenFile` (and a second
/// real LSP session) for the same real path. Canonicalizes both sides
/// before comparing so e.g. a relative CLI-launch path and an absolute
/// sidebar-derived path for the same real file still match; falls back to
/// a raw path comparison if canonicalization fails on either side (a file
/// that's been removed out from under an already-open tab, say) rather
/// than treating that as "never matches, always reopen."
fn find_open_file_index(files: &[OpenFile], path: &Path) -> Option<usize> {
    let target_canonical = std::fs::canonicalize(path).ok();
    files.iter().position(|f| {
        if f.label.starts_with("--synthetic:") {
            return false;
        }
        let label_path = Path::new(&f.label);
        match (&target_canonical, std::fs::canonicalize(label_path)) {
            (Some(target), Ok(label_canonical)) => *target == label_canonical,
            _ => label_path == path,
        }
    })
}

/// The window title reflects the active file's label and, for the first
/// time in this crate (§75.16), a real unsaved-changes indicator -- the
/// only user-visible signal of dirty state that exists anywhere yet, since
/// no tab bar (task #16) has been built.
fn window_title(file: &OpenFile) -> String {
    if file.dirty {
        format!("Spartan editor-core — {} *", file.label)
    } else {
        format!("Spartan editor-core — {}", file.label)
    }
}

/// Real close-tab (§75.21): removes the file at `index`, shutting down its
/// LSP session first. Refuses to close the last remaining open file --
/// this crate has no "empty editor" state to fall back to anywhere, and
/// real editors commonly keep at least one tab open too. Silently
/// discards unsaved changes on the closed file, matching every other place
/// a file's in-memory state can go away in this crate right now
/// (`CloseRequested`, switching files with Ctrl+Tab) -- task #18 tracks
/// adding a real confirmation prompt everywhere at once, not just here.
/// Adjusts `active` so it still points at a valid, sensible file
/// afterward: unaffected if it was before `index`, shifted left by one if
/// it was after, and left in place (now referring to what used to be the
/// next file) if it *was* `index` -- unless `index` was the last file, in
/// which case it's clamped to the new last file.
fn close_file(files: &mut Vec<OpenFile>, active: &mut usize, index: usize) {
    if files.len() <= 1 {
        println!("Cannot close the last open file");
        return;
    }
    let file = files.remove(index);
    if let Some(session) = file.lsp_session {
        session.shutdown();
    }
    if *active > index {
        *active -= 1;
    } else if *active >= files.len() {
        *active = files.len() - 1;
    }
}

/// Real tab drag-to-reorder (§75.27, task #25): moves the file at `from`
/// to sit exactly at `to`'s current position, shifting every file between
/// them over by one -- the standard `Vec::remove` + `Vec::insert` reorder.
/// A no-op if `from == to` or either index is out of range (the second
/// case shouldn't happen in practice, since both are resolved from a real
/// tab-bar hit-test against the current `files`, but is still checked
/// rather than trusted, matching this file's own established defensive
/// style). Adjusts `active` so it keeps pointing at the same *logical*
/// file regardless of where that file numerically ends up: follows `from`
/// directly to `to` if `active` was the file being moved; shifts by one in
/// the opposite direction of the move if `active` was strictly between
/// `from` and `to`; otherwise unaffected.
fn reorder_file(files: &mut Vec<OpenFile>, active: &mut usize, from: usize, to: usize) {
    if from == to || from >= files.len() || to >= files.len() {
        return;
    }
    let moved = files.remove(from);
    files.insert(to, moved);
    *active = if *active == from {
        to
    } else if from < to && *active > from && *active <= to {
        *active - 1
    } else if to < from && *active >= to && *active < from {
        *active + 1
    } else {
        *active
    };
}

/// What a real unsaved-changes confirmation modal (§75.23, task #18) is
/// currently blocking on -- either closing one specific tab (mouse click on
/// its `×` while it's dirty) or exiting the whole app (`CloseRequested`
/// while any open file is dirty). Deliberately does *not* cover switching
/// the active file (Ctrl+Tab, clicking a different tab): nothing is lost by
/// switching away from a dirty file, since its `OpenFile` -- and its
/// in-memory edits -- stay right where they are in `files`; only closing a
/// tab or the whole process can actually discard content.
enum PendingClose {
    File(usize),
    App,
}

/// Real modal message text (§75.23), rebuilt from live state every
/// `RedrawRequested` rather than captured once when the modal opens --
/// matching the tab bar's own "always rebuild from current `files`" pattern
/// (§75.21), so e.g. the dirty-file count stays correct even in the
/// unlikely event other state changes while the modal is up. Keyboard-only
/// (Enter/Escape) by design for this first real increment -- no clickable
/// button hit-testing exists yet, a real, named minimal v1, not an
/// oversight.
fn modal_message(pending: &PendingClose, files: &[OpenFile]) -> String {
    match pending {
        PendingClose::App => {
            let dirty_count = files.iter().filter(|f| f.dirty).count();
            format!(
                "{dirty_count} file(s) have unsaved changes.\n\n\
                 Press Enter to discard changes and exit, or Escape to cancel."
            )
        }
        PendingClose::File(index) => {
            let label = files
                .get(*index)
                .map(|f| f.label.as_str())
                .unwrap_or("This file");
            format!(
                "\"{label}\" has unsaved changes.\n\n\
                 Press Enter to discard changes and close it, or Escape to cancel."
            )
        }
    }
}

/// Linear-space black at moderate alpha (§75.23) -- pure black needs no
/// sRGB-to-linear conversion (`srgb_to_linear(0.0) == 0.0` regardless of the
/// curve), unlike the window's own non-black clear color just below.
const MODAL_DIM_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.6];

/// Real Source Control panel (§56.1, task #7): the left sidebar shows
/// either the file tree (§75.26) or real git status, toggled with Ctrl+G,
/// never both at once -- matching how §75.21's tab bar and this sidebar
/// already share the same left-rail region rather than each owning a
/// separate one (that's task #3's three-column shell, not yet built).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarMode {
    FileTree,
    SourceControl,
}

/// Real local-first crash directory (§18, task #13): `~/.spartan/crashes`
/// (`$HOME`, falling back to `$USERPROFILE` for Windows, falling back to
/// the current directory if neither is set -- the same kind of degrade-
/// gracefully-rather-than-panic posture this crate's own LSP/DAP spawn
/// failures already use). No config system exists yet to make this
/// configurable; `.spartan/` matches §15's own `.spartan/memory/team.md`
/// naming convention for this project's local dotfile namespace.
fn crash_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".spartan").join("crashes")
}

fn main() {
    // Installed before anything else in `main` can possibly panic --
    // real crash capture only has real value if it's live for the
    // earliest failure this process can hit, not just failures after
    // setup completes.
    spartan_crash::install_hook(crash_dir());

    // Real GTK initialization (§6.1, task #12) -- a real bug found only by
    // running this crate live on Linux, not by inspection: `wry`'s own
    // documented contract requires `gtk::init()` to have already run
    // before *any* `WebViewBuilder::build()` call on Linux/BSD (winit
    // doesn't initialize GTK itself, unlike a GTK-native windowing
    // toolkit), or it panics with "GDK has not been initialized." Called
    // unconditionally and early, even though Design mode's WebView itself
    // is created lazily on first use (§75.39) -- `gtk::init()` is cheap
    // and harmless if no WebView is ever created this session, and this
    // avoids any ordering hazard between lazy WebView creation and GTK
    // init that a lazier call site would risk. See the `AboutToWait`
    // handler below for the other documented half of this same contract
    // (pumping GTK's own event loop every frame).
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    gtk::init()
        .expect("gtk::init() failed -- required by wry's WebKitGTK backend on this platform");

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
    // reshape, however large the surrounding document is. Bench mode always
    // runs against `files[0]` only, regardless of how many `--open:` extra
    // files are also given -- nobody switches files mid-benchmark.
    let bench_edit_iters: Option<usize> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    let bench_cursor_iters: Option<usize> = std::env::args().nth(3).and_then(|s| s.parse().ok());
    let bench_scroll_iters: Option<usize> = std::env::args().nth(4).and_then(|s| s.parse().ok());
    // Optional 6th arg: a pre-built debug binary to launch under the
    // detected language's DAP adapter (F9/F5/F10/F11 below), for `files[0]`
    // only. Deliberately NOT "build the project automatically" -- see §75.8
    // for why that's a separate, real piece of work this pass doesn't
    // attempt.
    let debug_binary_path: Option<PathBuf> = std::env::args()
        .nth(5)
        .and_then(|s| s.strip_prefix("--debug-binary:").map(PathBuf::from));
    // Any further args of the form `--open:<path>` open additional files at
    // startup, switched between with Ctrl+Tab / Ctrl+Shift+Tab (§75.15). No
    // file-tree/file-dialog UI exists yet (see the crate README and task
    // #16) -- CLI args are the only way to open more than one file for now.
    let extra_files: Vec<String> = std::env::args()
        .skip(6)
        .filter_map(|s| s.strip_prefix("--open:").map(str::to_string))
        .collect();

    // First real combination of spartan-buffer + rendering + spartan-languages
    // (see language.rs's doc comment) -- tree-sitter stays unwired. LSP
    // (§75.6) and DAP (§75.8) are both wired for real below.
    let mut files: Vec<OpenFile> = Vec::with_capacity(1 + extra_files.len());
    files.push(open_file(&fixture_path, debug_binary_path.as_ref()));
    for extra in &extra_files {
        files.push(open_file(extra, None));
    }
    let mut active: usize = 0;

    // Real file tree sidebar (§75.26, task #24) -- rooted at whatever
    // `sidebar_root` resolves for the first opened file (the same real
    // project-root detection an LSP session already uses). `None` for a
    // `--synthetic:<n>` fixture (no real path to root a tree at) -- the
    // sidebar then has nothing to show, same "empty is the ordinary state"
    // pattern the modal/tab bar buffers already established.
    let mut file_tree = sidebar_root(&fixture_path).map(file_tree::FileTree::new);

    // Real Source Control panel (§56.1, task #7) -- `GitRepo::discover`
    // walks upward from the same root the file tree uses (real repos are
    // almost always at or above the project root LSP/DAP already
    // resolve), `None` if this project isn't inside a real git repo at
    // all (not an error -- an ungit'd project is a normal, valid state).
    let git_repo = sidebar_root(&fixture_path)
        .as_deref()
        .and_then(spartan_git::GitRepo::discover);
    let mut sidebar_mode = SidebarMode::FileTree;
    let mut git_rows: Vec<git_panel::PanelRow> = Vec::new();
    let mut commit_message: Option<String> = None;

    // Real Agent/Editor/Design mode toggle (§8, §16.1, task #3) -- starts
    // in `Editor`, the one mode with real content behind it.
    let mut mode = AppMode::Editor;
    let mut mode_hits: Vec<mode_toggle::ModeHit> = Vec::new();

    // Real, scoped command palette (§16.1, task #3) -- `None` is the
    // ordinary closed state. `all_entries` (the static commands plus a
    // real file listing under the project root) is captured once when the
    // palette opens, not re-scanned on every keystroke -- a deliberate
    // difference from `file_tree.rs`'s own always-fresh-read choice, since
    // the palette's own list should stay stable while a query is typed,
    // not flicker with concurrent filesystem changes mid-search.
    // `filtered`/`selected` are recomputed from `all_entries` on every
    // query edit (open, typed character, Backspace).
    struct CommandPaletteState {
        query: String,
        all_entries: Vec<command_palette::PaletteEntry>,
        filtered: Vec<command_palette::PaletteEntry>,
        selected: usize,
    }
    let mut command_palette_state: Option<CommandPaletteState> = None;

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
    // With more than one `--open:` file given, this first bucket now covers
    // every file's own load/LSP-spawn cost, not just one -- a real,
    // honestly-widened definition, not a hidden regression (multi-file
    // cold-open numbers are not directly comparable to every prior single-
    // file §75.x measurement for exactly this reason).
    let t_setup_done = Instant::now();

    // Real accessibility (§16.3, task #9): the event loop's user-event type
    // is `accesskit_winit::Event` directly (the identity `From<T> for T`
    // impl satisfies `Adapter::with_event_loop_proxy`'s own bound) rather
    // than a wrapper enum, matching `accesskit_winit`'s own reference
    // example exactly. AccessKit requires the window to be created hidden,
    // the adapter attached, and only then shown -- see the real doc comment
    // on `Adapter::with_event_loop_proxy` itself.
    let event_loop = EventLoopBuilder::<accesskit_winit::Event>::with_user_event()
        .build()
        .expect("failed to create winit event loop");
    let t_event_loop = Instant::now();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title(window_title(&files[active]))
            .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 700.0))
            .with_visible(false)
            .build(&event_loop)
            .expect("failed to create window"),
    );
    let mut accesskit_adapter =
        accesskit_winit::Adapter::with_event_loop_proxy(&window, event_loop.create_proxy());
    window.set_visible(true);
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
    // a named simplification (see the crate README). Every open file's
    // `Viewport` (constructed with a placeholder 0 by `open_file`, before
    // the window size was known) is patched to the real value here.
    let visible_lines = ((gpu_state.size.height as f32 - 2.0 * text::TEXT_ORIGIN_Y)
        / text::LINE_HEIGHT)
        .floor()
        .max(1.0) as usize;
    for file in files.iter_mut() {
        file.viewport.visible_lines = visible_lines;
    }
    println!(
        "Viewport: {visible_lines} visible lines (vs. {} total in the active document)",
        files[active].editor.document.len_lines()
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
    // window's text, not the whole document -- and, as of this pass, only
    // the currently *active* file's text, since `TextState` is shared
    // across every open file.
    {
        let active_file = &mut files[active];
        reshape_window(
            &mut text_state,
            &active_file.editor,
            &active_file.viewport,
            active_file.highlighter.as_mut(),
        );
    }
    let t_reshape = Instant::now();

    let cursor_renderer = cursor::CursorRenderer::new(&gpu_state.device, gpu_state.config.format);
    let t_cursor_renderer = Instant::now();
    let mut selection_renderer =
        selection::SelectionRenderer::new(&gpu_state.device, gpu_state.config.format);
    // Real active-tab highlight (§75.21) -- a second, independent instance
    // of the same `SelectionRenderer` type used for text selection, reused
    // rather than duplicated: both are just "draw some semi-transparent
    // accent-colored rects," and sharing the color between "this text is
    // selected" and "this is the active tab" is a coherent, deliberate
    // choice, not an accident of reuse.
    let mut tab_bar_renderer =
        selection::SelectionRenderer::new(&gpu_state.device, gpu_state.config.format);
    // Real unsaved-changes modal dim overlay (§75.23) -- a third instance of
    // the same generic `SelectionRenderer`, rendered after (on top of)
    // `tab_bar_renderer`/`selection_renderer` but still before all text, so
    // the modal's own text (a third glyphon `TextArea`, see `text.rs`) draws
    // on top of the dim overlay in the same text pass, the same "quads
    // before text" ordering every other highlight here already relies on.
    let mut modal_renderer =
        selection::SelectionRenderer::new(&gpu_state.device, gpu_state.config.format);

    // 150ms idle default per spec §2.3, now one debouncer per open file
    // (`OpenFile::lsp_debouncer`) rather than a single global one -- see
    // `AboutToWait` below, which polls every file's own debouncer each
    // tick, not just the active one's, so a background file's already-
    // pending edit (there shouldn't be one in practice, since only the
    // active file's `editor` is ever mutated by keyboard input, but the
    // per-file design stays correct regardless) is never silently dropped.
    let mut dap_session: Option<dap_session::DapSession> = None;
    // A real `cargo build` running on its own thread (§75.10) -- the
    // receiver half of a one-shot channel, polled non-blockingly each
    // frame, matching the `LspSession`/`DapSession` "never block the
    // render thread" pattern even though this itself isn't an ongoing
    // session. Global, not per-file: a debug session is one running
    // program, not tied to whichever file happens to be in view.
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
    // Tracked from `WindowEvent::ModifiersChanged` so Ctrl+Tab (next file)
    // and Ctrl+Shift+Tab (previous file) can be detected in the
    // `KeyboardInput` handler below, which only ever sees the key itself.
    let mut modifiers = ModifiersState::empty();
    // Real click-drag text selection (§75.18): true between a left
    // `MouseInput` press and its matching release, so `CursorMoved` in
    // between knows to extend the selection rather than just remembering
    // the position.
    let mut mouse_button_down = false;
    // The document-absolute char position the left button was pressed at,
    // if any -- deliberately *not* the same thing as an armed
    // `EditorView::selection_anchor` (a real, live bug fixed in §75.20 came
    // from conflating the two: see `WindowEvent::MouseInput`'s own comment
    // for the full story). `CursorMoved` arms a real selection anchor from
    // this lazily, only once the cursor has actually moved away from it.
    let mut drag_anchor_pos: Option<usize> = None;
    // Real tab drag-to-reorder (§75.27, task #25): the file index a tab
    // press started on, if the press landed on a tab's body (not its `×`).
    // Resolved on the matching release: if the release also lands on the
    // tab bar, at a *different* file index than the press, the two are
    // swapped via `reorder_file`. A real, deliberate v1 simplification: no
    // visual "ghost tab" follows the cursor mid-drag, only a teleport-on-
    // release reorder -- `CursorMoved` isn't wired to update anything about
    // an in-progress tab drag.
    let mut tab_drag_start: Option<usize> = None;
    // Real OS clipboard integration (§75.20). Construction can genuinely
    // fail (no X11/Wayland clipboard manager reachable, etc.) -- printed
    // once and handled gracefully rather than treated as fatal, matching
    // this crate's existing pattern for LSP/DAP spawn failures: Ctrl+C/X/V
    // simply report "clipboard unavailable" for the rest of the session
    // rather than panicking or silently doing nothing.
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => Some(c),
        Err(e) => {
            println!("Clipboard unavailable ({e}) -- Ctrl+C/X/V will not work this session");
            None
        }
    };
    // Real tab bar hit-testing state (§75.21) -- rebuilt every
    // `RedrawRequested` (alongside the tab bar's own display text) from the
    // real, current `files`/`active`/`dirty` state, then read back by
    // `MouseInput` to resolve a click to a specific tab (or its close
    // button). Persists across frames/events, unlike the tab bar text
    // itself, which `TextState` already owns.
    let mut tab_hits: Vec<tab_bar::TabHit> = Vec::new();
    // Real file tree sidebar hit-testing state (§75.26) -- same rebuild-
    // every-frame, read-back-by-`MouseInput` pattern as `tab_hits` just
    // above.
    let mut sidebar_rows: Vec<file_tree::TreeRow> = Vec::new();
    // Real unsaved-changes confirmation state (§75.23, task #18) -- `None`
    // the vast majority of the time. While `Some`, the dedicated
    // `KeyboardInput` arm below intercepts *all* keyboard input (Enter
    // confirms, Escape cancels, everything else is silently swallowed) and
    // both mouse-press arms are gated off, so nothing else in the app can
    // react to input while a modal is up.
    let mut pending_close: Option<PendingClose> = None;

    // Real embedded WebView bridge for Design mode (§6.1, task #12),
    // promoted from `spikes/ui-shell-spike` (§47.11). `None` until the
    // first time the user actually switches to Design mode -- a real,
    // deliberate lazy-creation choice (an app that never opens Design mode
    // never pays the cost of spawning a child WebKitGTK/WebView2 process).
    let mut webview_bridge: Option<webview_bridge::WebviewBridge> = None;
    // Real §75.41 dev-server bridge in-flight request, if any -- a real
    // `node`/`gui-builder` subprocess spawned on its own thread, polled
    // non-blockingly in `AboutToWait` below (matching `pending_build`'s
    // own established pattern for a different real subprocess).
    let mut component_tree_request: Option<gui_bridge::ComponentTreeRequest> = None;
    // Real §75.42 Canvas -> Code in-flight request, if any -- a real
    // `node`/`gui-builder apply` subprocess spawned on its own thread when
    // the WebView's edit form posts a real `CanvasEdit` over IPC, polled
    // non-blockingly in `AboutToWait` below (same pattern as
    // `component_tree_request` just above, for the opposite sync
    // direction).
    let mut pending_apply_edit: Option<gui_bridge::ApplyEditRequest> = None;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);

            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    // Real accessibility (§16.3, task #9): must be called
                    // for every window event, before this crate's own
                    // handling of it, per `Adapter::process_event`'s own
                    // real documented contract.
                    accesskit_adapter.process_event(&window, &event);

                    // Real Design-mode WebView focus fix (§6.1, task #12).
                    // `spikes/ui-shell-spike`'s own README (§47.11) named
                    // this exact question as "unexplored" for wry's Linux
                    // WebKitGTK backend, having only confirmed and fixed
                    // the Windows/WebView2 case (see `win32_hwnd`'s own
                    // doc comment). It's no longer unexplored: a real,
                    // live click-drag test against this crate's actual
                    // WebKitGTK-backed Design mode confirmed the identical
                    // symptom on Linux -- once the embedded WebView takes
                    // keyboard focus, Ctrl+2 (and every other native
                    // shortcut) silently stops reaching winit's own
                    // `KeyboardInput` events, with no error of any kind.
                    // `window.focus_window()` (winit's own cross-platform
                    // method, backed by a real `XSetInputFocus` call on
                    // X11) *does* reclaim it here, unlike on Windows where
                    // that spike's own testing found it insufficient and
                    // needed the raw Win32 `SetFocus` call instead -- so
                    // both are applied, each covering the platform where
                    // it was actually proven to work. Peeked here (by
                    // reference, before the real `match event` below
                    // consumes it) so it applies to *every* native mouse
                    // press, regardless of which specific region's own
                    // `MouseInput` arm ends up matching.
                    if matches!(
                        &event,
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            ..
                        }
                    ) {
                        window.focus_window();
                        #[cfg(windows)]
                        unsafe {
                            let _ = SetFocus(win32_hwnd(&window));
                        }
                    }

                    match event {
                        WindowEvent::CloseRequested => {
                            // Real unsaved-changes gating (§75.23, task
                            // #18): a dirty file blocks the immediate exit
                            // below and instead raises the modal, which the
                            // dedicated `KeyboardInput` arm further down
                            // (guarded on `pending_close.is_some()`) resolves
                            // on Enter/Escape. If a modal is already up
                            // (either kind), a repeated close request (e.g.
                            // clicking the window's own close button twice)
                            // is deliberately a no-op here -- it doesn't
                            // matter, since the modal it would raise is
                            // already showing.
                            if pending_close.is_none() {
                                if files.iter().any(|f| f.dirty) {
                                    pending_close = Some(PendingClose::App);
                                    window.request_redraw();
                                } else {
                                    println!("\n=== Final reports ===");
                                    println!(
                                        "In-window edits: {in_window_edits} (real reshape) | \
                                         off-window edits: {off_window_edits} (no redraw needed)"
                                    );
                                    edit_latency.report("input-to-photon (edits, random-position)");
                                    cursor_latency.report("input-to-photon (edits, cursor-adjacent)");
                                    scroll_latency.report("scroll re-shape");
                                    // Real bug caught only by actually closing the live app, not
                                    // by inspection: `elwt.exit()` doesn't take effect
                                    // immediately -- winit still delivers at least one more event
                                    // afterward (this crate's own `ControlFlow::Poll` means
                                    // that's typically another `RedrawRequested`), and every
                                    // other handler unconditionally indexes `files[active]`.
                                    // Draining `files` here left it empty, so that next event's
                                    // `files[active]` access panicked with an out-of-bounds
                                    // index. `take()`-ing each file's `lsp_session` in place (not
                                    // draining the `Vec` itself) shuts every session down exactly
                                    // the same way while leaving `files[active]` valid for
                                    // whatever winit delivers before the process actually exits.
                                    for file in files.iter_mut() {
                                        if let Some(session) = file.lsp_session.take() {
                                            session.shutdown();
                                        }
                                    }
                                    if let Some(session) = dap_session.take() {
                                        session.shutdown();
                                    }
                                    elwt.exit();
                                }
                            }
                        }
                        WindowEvent::Resized(new_size) => {
                            gpu_state.resize(new_size);
                            text_state.resize(new_size.width as f32, new_size.height as f32);

                            // Real Design-mode WebView resize (§6.1, task
                            // #12) -- only meaningful while a bridge exists
                            // and is actually showing; `sync_webview_for_
                            // mode`'s own hidden-bounds branch would just
                            // re-hide it at a stale size otherwise, so this
                            // only touches the WebView when Design mode is
                            // the current one.
                            if mode == AppMode::Design {
                                if let Some(bridge) = &webview_bridge {
                                    bridge.set_bounds(design_content_bounds(&window, new_size));
                                }
                            }

                            // Recomputes how many lines are visible from the new window
                            // height -- previously fixed at startup only (a named
                            // limitation, §75.5), meaning a resized window's viewport size
                            // silently drifted from what was actually on screen. Applied to
                            // every open file's own `Viewport`, not just the active one, so
                            // switching files later doesn't see a stale line count.
                            let new_visible_lines = ((new_size.height as f32
                                - 2.0 * text::TEXT_ORIGIN_Y)
                                / text::LINE_HEIGHT)
                                .floor()
                                .max(1.0) as usize;
                            if new_visible_lines != files[active].viewport.visible_lines {
                                for file in files.iter_mut() {
                                    file.viewport.visible_lines = new_visible_lines;
                                }
                                let active_file = &mut files[active];
                                let (cursor_line, _) = active_file.editor.cursor_line_col();
                                let doc_len_lines = active_file.editor.document.len_lines();
                                active_file.viewport.ensure_visible(cursor_line, doc_len_lines);
                                reshape_window(
                                    &mut text_state,
                                    &active_file.editor,
                                    &active_file.viewport,
                                    active_file.highlighter.as_mut(),
                                );
                                window.request_redraw();
                            }
                        }
                        WindowEvent::ModifiersChanged(mods) => {
                            modifiers = mods.state();
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            last_cursor_pos = (position.x as f32, position.y as f32);
                            // Real click-drag selection (§75.18): while the
                            // left button is held, every subsequent
                            // `CursorMoved` moves the cursor to the new
                            // hit-tested position and, the first time it
                            // actually differs from where the button was
                            // pressed (`drag_anchor_pos`), arms a real
                            // selection anchor there -- see `drag_anchor_pos`'s
                            // own declaration comment for why this is
                            // deliberately lazy rather than armed on press.
                            if mouse_button_down {
                                let local_x = last_cursor_pos.0 - text::TEXT_ORIGIN_X;
                                let local_y = last_cursor_pos.1 - text::TEXT_ORIGIN_Y;
                                if let Some((local_line, col_chars)) =
                                    text_state.hit_test(local_x, local_y)
                                {
                                    let active_file = &mut files[active];
                                    let doc_line =
                                        viewport::to_doc_line(local_line, &active_file.viewport);
                                    active_file.editor.set_cursor_to_line_col(doc_line, col_chars);
                                    if let Some(anchor) = drag_anchor_pos {
                                        if active_file.editor.cursor != anchor {
                                            active_file.editor.selection_anchor = Some(anchor);
                                        }
                                    }
                                    window.request_redraw();
                                }
                            }
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && command_palette_state.is_none()
                            && last_cursor_pos.0 < text::SIDEBAR_WIDTH
                            && sidebar_mode == SidebarMode::FileTree =>
                        {
                            // Real file tree sidebar clicks (§75.26) --
                            // resolved via the same real cosmic-text
                            // hit-testing technique as the main editor and
                            // tab bar, not a pixel-guessing geometry model.
                            // A directory row toggles expand/collapse; a
                            // file row opens it, or switches to it if
                            // already open (`find_open_file_index`) rather
                            // than spawning a second `OpenFile` -- and a
                            // second real LSP session -- for the same path.
                            let local_x = last_cursor_pos.0 - text::SIDEBAR_TEXT_LEFT;
                            let local_y = last_cursor_pos.1 - text::SIDEBAR_TEXT_TOP;
                            if let Some(row_index) = text_state.hit_test_sidebar(local_x, local_y)
                            {
                                if let Some(row) = sidebar_rows.get(row_index).cloned() {
                                    if row.is_dir {
                                        if let Some(tree) = file_tree.as_mut() {
                                            tree.toggle(&row.path);
                                        }
                                        window.request_redraw();
                                    } else {
                                        let target_index = find_open_file_index(&files, &row.path)
                                            .unwrap_or_else(|| {
                                                let mut new_file = open_file(
                                                    &row.path.display().to_string(),
                                                    None,
                                                );
                                                new_file.viewport.visible_lines =
                                                    files[active].viewport.visible_lines;
                                                files.push(new_file);
                                                files.len() - 1
                                            });
                                        active = target_index;
                                        let active_file = &mut files[active];
                                        window.set_title(&window_title(active_file));
                                        reshape_window(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            active_file.highlighter.as_mut(),
                                        );
                                        sync_webview_file_info(
                                            &webview_bridge,
                                            &mut component_tree_request,
                                            mode,
                                            &files[active],
                                        );
                                        window.request_redraw();
                                    }
                                }
                            }
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && command_palette_state.is_none()
                            && last_cursor_pos.0 < text::SIDEBAR_WIDTH
                            && sidebar_mode == SidebarMode::SourceControl =>
                        {
                            // Real Source Control panel clicks (§56.1, task
                            // #7) -- resolved via the same real cosmic-text
                            // sidebar hit-testing the file tree already uses.
                            // Clicking a file row in the "Staged Changes"
                            // section unstages it; clicking one in "Changes"
                            // stages it. Clicking a section header is a
                            // no-op (`git_rows.get` returns a `Header`, which
                            // the `if let` below simply doesn't match).
                            let local_x = last_cursor_pos.0 - text::SIDEBAR_TEXT_LEFT;
                            let local_y = last_cursor_pos.1 - text::SIDEBAR_TEXT_TOP;
                            if let Some(row_index) = text_state.hit_test_sidebar(local_x, local_y)
                            {
                                if let Some(git_panel::PanelRow::File { path, staged }) =
                                    git_rows.get(row_index)
                                {
                                    if let Some(repo) = &git_repo {
                                        let result = if *staged {
                                            repo.unstage(path)
                                        } else {
                                            repo.stage(path)
                                        };
                                        if let Err(e) = result {
                                            println!("git operation failed: {e}");
                                        }
                                        window.request_redraw();
                                    }
                                }
                            }
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && command_palette_state.is_none()
                            && last_cursor_pos.0 >= text::SIDEBAR_WIDTH
                            && last_cursor_pos.0
                                < gpu_state.size.width as f32 - text::MODE_TOGGLE_WIDTH
                            && last_cursor_pos.1 < text::TAB_BAR_HEIGHT =>
                        {
                            // Real tab bar clicks (§75.21) -- resolved via the
                            // same real cosmic-text hit-testing technique as
                            // the main editor, not a pixel-guessing geometry
                            // model (see `tab_bar.rs`'s own doc comment).
                            // `+ tab_bar_scroll()` undoes the real horizontal
                            // scroll offset (§75.28) the tab bar may
                            // currently be rendered at, translating a
                            // screen-space click back into the tab bar
                            // buffer's own unscrolled coordinate space.
                            let local_x =
                                last_cursor_pos.0 - text::TEXT_ORIGIN_X + text_state.tab_bar_scroll();
                            let local_y = last_cursor_pos.1 - text::TAB_BAR_TEXT_TOP;
                            if let Some(char_index) = text_state.hit_test_tab_bar(local_x, local_y)
                            {
                                if let Some((file_index, is_close)) =
                                    tab_bar::hit_test_tab_bar(&tab_hits, char_index)
                                {
                                    // Real unsaved-changes gating (§75.23,
                                    // task #18): closing a dirty tab raises
                                    // the modal instead of discarding its
                                    // edits immediately -- switching tabs
                                    // (the `else` arm just below) never
                                    // needs this, since nothing is lost by
                                    // switching away from a dirty file. Only
                                    // when `files.len() > 1`, matching
                                    // `close_file`'s own "refuse to close
                                    // the last open file" guard -- without
                                    // this, closing a dirty *sole* tab would
                                    // raise a modal offering to discard and
                                    // close, then silently fail to actually
                                    // close anything once confirmed (there's
                                    // no "empty editor" state to fall back
                                    // to), which is real, confusing behavior
                                    // this check avoids entirely rather than
                                    // letting `close_file`'s own refusal
                                    // print through *after* a pointless
                                    // confirmation step.
                                    if is_close && files[file_index].dirty && files.len() > 1 {
                                        pending_close = Some(PendingClose::File(file_index));
                                        window.request_redraw();
                                    } else {
                                        if is_close {
                                            close_file(&mut files, &mut active, file_index);
                                        } else {
                                            active = file_index;
                                            // Real tab drag-to-reorder
                                            // (§75.27): armed on a press on
                                            // a tab's body (never its `×`,
                                            // which already took the other
                                            // branch above), resolved on
                                            // the matching `MouseInput`
                                            // release below.
                                            tab_drag_start = Some(file_index);
                                        }
                                        let active_file = &mut files[active];
                                        window.set_title(&window_title(active_file));
                                        reshape_window(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            active_file.highlighter.as_mut(),
                                        );
                                        sync_webview_file_info(
                                            &webview_bridge,
                                            &mut component_tree_request,
                                            mode,
                                            &files[active],
                                        );
                                        window.request_redraw();
                                    }
                                }
                            }
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && command_palette_state.is_none()
                            && last_cursor_pos.0
                                >= gpu_state.size.width as f32 - text::MODE_TOGGLE_WIDTH
                            && last_cursor_pos.1 < text::TAB_BAR_HEIGHT =>
                        {
                            // Real mode toggle clicks (§8, §16.1, task #3)
                            // -- same real cosmic-text hit-testing
                            // technique as every other click target in
                            // this crate, not a pixel-guessing geometry
                            // model.
                            let local_x = last_cursor_pos.0
                                - (gpu_state.size.width as f32 - text::MODE_TOGGLE_WIDTH);
                            let local_y = last_cursor_pos.1 - text::TAB_BAR_TEXT_TOP;
                            if let Some(char_index) =
                                text_state.hit_test_mode_toggle(local_x, local_y)
                            {
                                if let Some(clicked_mode) =
                                    mode_toggle::hit_test(&mode_hits, char_index)
                                {
                                    mode = clicked_mode;
                                    sync_webview_for_mode(
                                        &mut webview_bridge,
                                        &mut component_tree_request,
                                        mode,
                                        &window,
                                        gpu_state.size,
                                        &files[active],
                                    );
                                    window.request_redraw();
                                }
                            }
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } if pending_close.is_none()
                            && command_palette_state.is_none()
                            && mode == AppMode::Editor =>
                        {
                            mouse_button_down = true;
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
                                let active_file = &mut files[active];
                                let doc_line = viewport::to_doc_line(local_line, &active_file.viewport);
                                if modifiers.shift_key() {
                                    // Shift+click extends the existing
                                    // selection (or starts one anchored at
                                    // the cursor's pre-click position) to
                                    // the click point, rather than starting
                                    // a fresh one there.
                                    active_file.editor.start_selection_if_needed();
                                } else {
                                    active_file.editor.selection_anchor = None;
                                }
                                active_file.editor.set_cursor_to_line_col(doc_line, col_chars);
                                // Real, live bug fixed here, found only by
                                // testing paste-after-click, not by
                                // inspection: an earlier version armed
                                // `selection_anchor` unconditionally on
                                // every plain click "in case of a drag."
                                // But `selection_range()` only checks
                                // whether the anchor and cursor differ, so
                                // *any* later cursor movement by *any*
                                // means (typing, pasting, undo/redo, not
                                // just a real drag) silently turned into
                                // what looked like a real, visible
                                // selection. Fixed by only *remembering*
                                // the press position here
                                // (`drag_anchor_pos`) and arming the real
                                // anchor lazily in `CursorMoved`, above,
                                // the first time the cursor actually moves
                                // during this drag.
                                drag_anchor_pos = if modifiers.shift_key() {
                                    None
                                } else {
                                    Some(active_file.editor.cursor)
                                };
                                window.request_redraw();
                            }
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Released,
                            button: MouseButton::Left,
                            ..
                        } => {
                            mouse_button_down = false;
                            drag_anchor_pos = None;
                            // Real tab drag-to-reorder (§75.27): resolved
                            // here, on release, against whatever tab (if
                            // any) the cursor is currently over -- a real
                            // reorder only if the release also landed on
                            // the tab bar, at a *different* file index than
                            // the press started on (a plain click, with no
                            // drag at all, must not silently reorder
                            // anything).
                            if let Some(from) = tab_drag_start.take() {
                                if pending_close.is_none()
                                    && last_cursor_pos.0 >= text::SIDEBAR_WIDTH
                                    && last_cursor_pos.1 < text::TAB_BAR_HEIGHT
                                {
                                    let local_x = last_cursor_pos.0 - text::TEXT_ORIGIN_X
                                        + text_state.tab_bar_scroll();
                                    let local_y = last_cursor_pos.1 - text::TAB_BAR_TEXT_TOP;
                                    if let Some(char_index) =
                                        text_state.hit_test_tab_bar(local_x, local_y)
                                    {
                                        if let Some((to, is_close)) =
                                            tab_bar::hit_test_tab_bar(&tab_hits, char_index)
                                        {
                                            if !is_close && to != from {
                                                reorder_file(&mut files, &mut active, from, to);
                                                let active_file = &mut files[active];
                                                window.set_title(&window_title(active_file));
                                                window.request_redraw();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if pending_close.is_some() => {
                            // Real unsaved-changes modal confirm/cancel
                            // (§75.23, task #18) -- this arm's guard fires
                            // before any of the other `KeyboardInput` arms
                            // below get a chance (match arms are tried in
                            // order), so while a modal is up *every*
                            // keystroke is intercepted here: Enter confirms,
                            // Escape cancels, anything else is silently
                            // swallowed rather than leaking through to
                            // typing/shortcuts underneath.
                            match key_event.logical_key {
                                Key::Named(NamedKey::Enter) => match pending_close.take() {
                                    Some(PendingClose::File(index)) => {
                                        close_file(&mut files, &mut active, index);
                                        let active_file = &mut files[active];
                                        window.set_title(&window_title(active_file));
                                        reshape_window(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            active_file.highlighter.as_mut(),
                                        );
                                        sync_webview_file_info(
                                            &webview_bridge,
                                            &mut component_tree_request,
                                            mode,
                                            &files[active],
                                        );
                                        window.request_redraw();
                                    }
                                    Some(PendingClose::App) => {
                                        println!("\n=== Final reports ===");
                                        println!(
                                            "In-window edits: {in_window_edits} (real reshape) | \
                                             off-window edits: {off_window_edits} (no redraw needed)"
                                        );
                                        edit_latency
                                            .report("input-to-photon (edits, random-position)");
                                        cursor_latency
                                            .report("input-to-photon (edits, cursor-adjacent)");
                                        scroll_latency.report("scroll re-shape");
                                        for file in files.iter_mut() {
                                            if let Some(session) = file.lsp_session.take() {
                                                session.shutdown();
                                            }
                                        }
                                        if let Some(session) = dap_session.take() {
                                            session.shutdown();
                                        }
                                        elwt.exit();
                                    }
                                    None => {}
                                },
                                Key::Named(NamedKey::Escape) => {
                                    pending_close = None;
                                    window.request_redraw();
                                }
                                _ => {}
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if commit_message.is_some() => {
                            // Real commit-message modal (§56.1, task #7) --
                            // same "intercept everything while this state is
                            // Some" pattern as the unsaved-changes modal just
                            // above (mutually exclusive with it in practice:
                            // nothing can open the commit modal while
                            // `pending_close` is already showing, since that
                            // arm's own guard requires `commit_message.is_none()`).
                            // Reuses the same `modal_buffer`/dim-overlay
                            // rendering rather than a second modal type.
                            match &key_event.logical_key {
                                Key::Named(NamedKey::Enter) => {
                                    let message = commit_message.clone().unwrap_or_default();
                                    if message.trim().is_empty() {
                                        println!("Commit message cannot be empty");
                                    } else if let Some(repo) = &git_repo {
                                        match repo.commit(message.trim()) {
                                            Ok(oid) => {
                                                println!(
                                                    "Committed {oid}: {}",
                                                    message.trim()
                                                );
                                                commit_message = None;
                                            }
                                            Err(e) => println!("Commit failed: {e}"),
                                        }
                                    }
                                    window.request_redraw();
                                }
                                Key::Named(NamedKey::Escape) => {
                                    commit_message = None;
                                    window.request_redraw();
                                }
                                Key::Named(NamedKey::Backspace) => {
                                    if let Some(msg) = commit_message.as_mut() {
                                        msg.pop();
                                    }
                                    window.request_redraw();
                                }
                                _ => {
                                    if let Some(text) = &key_event.text {
                                        if !text.is_empty() && text.chars().all(|c| !c.is_control())
                                        {
                                            if let Some(msg) = commit_message.as_mut() {
                                                msg.push_str(text);
                                            }
                                            window.request_redraw();
                                        }
                                    }
                                }
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if command_palette_state.is_some() => {
                            // Real command palette input (§16.1, task #3) --
                            // same "intercept everything while this state is
                            // Some" pattern as the unsaved-changes and
                            // commit-message modals just above (mutually
                            // exclusive with both: the Ctrl+P arm below that
                            // opens this can only fire while `pending_close`
                            // and `commit_message` are both already `None`).
                            match &key_event.logical_key {
                                Key::Named(NamedKey::Escape) => {
                                    command_palette_state = None;
                                    window.request_redraw();
                                }
                                Key::Named(NamedKey::Enter) => {
                                    // Extract the selected entry (a real,
                                    // owned clone) and close the palette
                                    // *before* running its action, not
                                    // after -- several actions below (e.g.
                                    // Close Tab on a dirty file) themselves
                                    // open a *different* modal
                                    // (`pending_close`), which must not be
                                    // immediately clobbered by this palette
                                    // state still being `Some`.
                                    let entry = command_palette_state
                                        .as_ref()
                                        .and_then(|p| p.filtered.get(p.selected).cloned());
                                    command_palette_state = None;
                                    if let Some(entry) = entry {
                                        // Every palette action is
                                        // document-oriented (or explicitly
                                        // chooses a mode itself, below) --
                                        // force `Editor` first so its
                                        // effect is actually visible
                                        // instead of happening silently
                                        // behind an Agent/Design
                                        // placeholder still being shown.
                                        mode = AppMode::Editor;
                                        match entry {
                                            command_palette::PaletteEntry::Command(id) => match id
                                            {
                                                command_palette::CommandId::SaveFile => {
                                                    let active_file = &mut files[active];
                                                    if active_file.label.starts_with("--synthetic:")
                                                    {
                                                        println!(
                                                            "Cannot save {} -- a --synthetic: fixture has no real file path",
                                                            active_file.label
                                                        );
                                                    } else {
                                                        match std::fs::write(
                                                            &active_file.label,
                                                            active_file.editor.text(),
                                                        ) {
                                                            Ok(()) => {
                                                                active_file.dirty = false;
                                                                println!(
                                                                    "Saved: {}",
                                                                    active_file.label
                                                                );
                                                                window.set_title(&window_title(
                                                                    active_file,
                                                                ));
                                                            }
                                                            Err(e) => println!(
                                                                "Failed to save {}: {e}",
                                                                active_file.label
                                                            ),
                                                        }
                                                    }
                                                }
                                                command_palette::CommandId::Undo
                                                | command_palette::CommandId::Redo => {
                                                    let active_file = &mut files[active];
                                                    let is_redo = matches!(
                                                        id,
                                                        command_palette::CommandId::Redo
                                                    );
                                                    let changed = if is_redo {
                                                        active_file.editor.redo()
                                                    } else {
                                                        active_file.editor.undo()
                                                    };
                                                    if changed {
                                                        if !active_file.dirty {
                                                            active_file.dirty = true;
                                                            window.set_title(&window_title(
                                                                active_file,
                                                            ));
                                                        }
                                                        let (cursor_line, _) =
                                                            active_file.editor.cursor_line_col();
                                                        let doc_len_lines =
                                                            active_file.editor.document.len_lines();
                                                        active_file
                                                            .viewport
                                                            .ensure_visible(cursor_line, doc_len_lines);
                                                        reshape_window(
                                                            &mut text_state,
                                                            &active_file.editor,
                                                            &active_file.viewport,
                                                            active_file.highlighter.as_mut(),
                                                        );
                                                    } else {
                                                        println!(
                                                            "Nothing to {}",
                                                            if is_redo { "redo" } else { "undo" }
                                                        );
                                                    }
                                                }
                                                command_palette::CommandId::CloseTab => {
                                                    if files[active].dirty && files.len() > 1 {
                                                        pending_close =
                                                            Some(PendingClose::File(active));
                                                    } else {
                                                        let closing = active;
                                                        close_file(&mut files, &mut active, closing);
                                                        let active_file = &mut files[active];
                                                        window.set_title(&window_title(active_file));
                                                        reshape_window(
                                                            &mut text_state,
                                                            &active_file.editor,
                                                            &active_file.viewport,
                                                            active_file.highlighter.as_mut(),
                                                        );
                                                    }
                                                }
                                                command_palette::CommandId::ToggleSidebar => {
                                                    sidebar_mode = match sidebar_mode {
                                                        SidebarMode::FileTree => {
                                                            SidebarMode::SourceControl
                                                        }
                                                        SidebarMode::SourceControl => {
                                                            SidebarMode::FileTree
                                                        }
                                                    };
                                                }
                                                command_palette::CommandId::SwitchToAgentMode => {
                                                    mode = AppMode::Agent;
                                                }
                                                command_palette::CommandId::SwitchToEditorMode => {
                                                    mode = AppMode::Editor;
                                                }
                                                command_palette::CommandId::SwitchToDesignMode => {
                                                    mode = AppMode::Design;
                                                }
                                            },
                                            command_palette::PaletteEntry::File(path) => {
                                                // Mirrors the file tree
                                                // sidebar's own file-click
                                                // arm above: switch to an
                                                // already-open file rather
                                                // than opening a second
                                                // `OpenFile` (and a second
                                                // real LSP session) for the
                                                // same real path.
                                                let target_index = find_open_file_index(
                                                    &files, &path,
                                                )
                                                .unwrap_or_else(|| {
                                                    let mut new_file = open_file(
                                                        &path.display().to_string(),
                                                        None,
                                                    );
                                                    new_file.viewport.visible_lines =
                                                        files[active].viewport.visible_lines;
                                                    files.push(new_file);
                                                    files.len() - 1
                                                });
                                                active = target_index;
                                                let active_file = &mut files[active];
                                                window.set_title(&window_title(active_file));
                                                reshape_window(
                                                    &mut text_state,
                                                    &active_file.editor,
                                                    &active_file.viewport,
                                                    active_file.highlighter.as_mut(),
                                                );
                                            }
                                        }
                                        // Real, single sync point (§6.1,
                                        // task #12): whatever `mode` ended
                                        // up settling on above (forced
                                        // Editor, then possibly overridden
                                        // by one of the three explicit
                                        // mode-switch commands) is
                                        // reconciled with the WebView
                                        // exactly once here, not at each
                                        // individual command arm.
                                        sync_webview_for_mode(
                                            &mut webview_bridge,
                                            &mut component_tree_request,
                                            mode,
                                            &window,
                                            gpu_state.size,
                                            &files[active],
                                        );
                                    }
                                    window.request_redraw();
                                }
                                Key::Named(NamedKey::ArrowUp) => {
                                    if let Some(palette) = command_palette_state.as_mut() {
                                        if palette.selected > 0 {
                                            palette.selected -= 1;
                                        }
                                    }
                                    window.request_redraw();
                                }
                                Key::Named(NamedKey::ArrowDown) => {
                                    if let Some(palette) = command_palette_state.as_mut() {
                                        if palette.selected + 1 < palette.filtered.len() {
                                            palette.selected += 1;
                                        }
                                    }
                                    window.request_redraw();
                                }
                                Key::Named(NamedKey::Backspace) => {
                                    if let Some(palette) = command_palette_state.as_mut() {
                                        palette.query.pop();
                                        palette.filtered = command_palette::filter_and_rank(
                                            &palette.query,
                                            &palette.all_entries,
                                        );
                                        palette.selected = 0;
                                    }
                                    window.request_redraw();
                                }
                                _ => {
                                    if let Some(text) = &key_event.text {
                                        if !text.is_empty() && text.chars().all(|c| !c.is_control())
                                        {
                                            if let Some(palette) = command_palette_state.as_mut() {
                                                palette.query.push_str(text);
                                                palette.filtered = command_palette::filter_and_rank(
                                                    &palette.query,
                                                    &palette.all_entries,
                                                );
                                                palette.selected = 0;
                                            }
                                            window.request_redraw();
                                        }
                                    }
                                }
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && command_palette_state.is_none()
                            && modifiers.control_key()
                            && key_event.physical_key == PhysicalKey::Code(KeyCode::KeyP) =>
                        {
                            // Ctrl+P: open the real command palette (§16.1,
                            // task #3). The file half of its entry list is
                            // captured once here, from whatever real project
                            // root the file tree sidebar already resolved --
                            // `None` (a `--synthetic:` fixture) means no
                            // real files to list, so the palette shows only
                            // the static command list, same "empty is the
                            // ordinary state" pattern this crate's other
                            // optional panels already use.
                            let mut all_entries = command_palette::static_commands();
                            if let Some(tree) = &file_tree {
                                all_entries.extend(
                                    command_palette::collect_files(tree.root(), 6)
                                        .into_iter()
                                        .map(command_palette::PaletteEntry::File),
                                );
                            }
                            let filtered = command_palette::filter_and_rank("", &all_entries);
                            command_palette_state = Some(CommandPaletteState {
                                query: String::new(),
                                all_entries,
                                filtered,
                                selected: 0,
                            });
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && modifiers.control_key()
                            && key_event.physical_key == PhysicalKey::Code(KeyCode::KeyG) =>
                        {
                            // Ctrl+G: toggle the left sidebar between the
                            // file tree (§75.26) and the real Source Control
                            // panel (§56.1, task #7) -- they share one region
                            // rather than each getting a dedicated pane,
                            // since a real multi-pane left rail is task #3's
                            // three-column shell, not yet built.
                            sidebar_mode = match sidebar_mode {
                                SidebarMode::FileTree => SidebarMode::SourceControl,
                                SidebarMode::SourceControl => SidebarMode::FileTree,
                            };
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && modifiers.control_key()
                            && modifiers.shift_key()
                            && key_event.physical_key == PhysicalKey::Code(KeyCode::KeyC) =>
                        {
                            // Ctrl+Shift+C: open the real commit-message
                            // modal -- only if a real git repo was
                            // discovered and something is actually staged
                            // (an empty commit is refused up front rather
                            // than opening a modal that can only fail).
                            match &git_repo {
                                Some(repo) => match repo.status() {
                                    Ok(status) if status.iter().any(|e| e.staged.is_some()) => {
                                        commit_message = Some(String::new());
                                        window.request_redraw();
                                    }
                                    Ok(_) => println!("Nothing staged to commit"),
                                    Err(e) => println!("git status failed: {e}"),
                                },
                                None => println!("No git repository detected for this project"),
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && modifiers.control_key()
                            && matches!(
                                key_event.physical_key,
                                PhysicalKey::Code(KeyCode::Digit1)
                                    | PhysicalKey::Code(KeyCode::Digit2)
                                    | PhysicalKey::Code(KeyCode::Digit3)
                            ) =>
                        {
                            // Ctrl+1/2/3: real Agent/Editor/Design mode
                            // switching (§8, §16.1, task #3) -- checked
                            // before the "swallow everything while
                            // non-Editor" arm just below, so mode-switch
                            // keys always work regardless of the current
                            // mode, the same way Escape always works to
                            // close a modal regardless of what's on
                            // screen behind it.
                            mode = match key_event.physical_key {
                                PhysicalKey::Code(KeyCode::Digit1) => AppMode::Agent,
                                PhysicalKey::Code(KeyCode::Digit2) => AppMode::Editor,
                                PhysicalKey::Code(KeyCode::Digit3) => AppMode::Design,
                                _ => unreachable!(),
                            };
                            sync_webview_for_mode(
                                &mut webview_bridge,
                                &mut component_tree_request,
                                mode,
                                &window,
                                gpu_state.size,
                                &files[active],
                            );
                            window.request_redraw();
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if pending_close.is_none()
                            && commit_message.is_none()
                            && mode != AppMode::Editor =>
                        {
                            // Real mode-gating (§8, task #3): while Agent/
                            // Design mode is showing its real, honest
                            // empty-state placeholder instead of the real
                            // document, keyboard input must not silently
                            // mutate the hidden document underneath --
                            // swallow everything except the mode-switch
                            // shortcuts handled by the arm just above
                            // (which this arm never sees, since match arms
                            // are tried in order and that one already
                            // claimed Ctrl+1/2/3). A real, named v1 scope
                            // boundary, not a partial/buggy attempt at
                            // gating every individual edit-capable arm
                            // below: every other shortcut, including
                            // save/undo/close, is intentionally inert
                            // while a non-Editor mode is showing.
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if modifiers.control_key()
                            && key_event.physical_key == PhysicalKey::Code(KeyCode::KeyS) =>
                        {
                            // Ctrl+S: real save-to-disk (§75.16) -- the first
                            // save functionality anywhere in this crate. Only
                            // matched via `physical_key` (layout/modifier-
                            // independent), not `logical_key`, since some
                            // platforms report no `text` at all for a
                            // Ctrl-held letter, and this needs to fire
                            // regardless of what `logical_key` resolves to.
                            let active_file = &mut files[active];
                            if active_file.label.starts_with("--synthetic:") {
                                println!(
                                    "Cannot save {} -- a --synthetic: fixture has no real file path",
                                    active_file.label
                                );
                            } else {
                                match std::fs::write(&active_file.label, active_file.editor.text())
                                {
                                    Ok(()) => {
                                        active_file.dirty = false;
                                        println!("Saved: {}", active_file.label);
                                        window.set_title(&window_title(active_file));
                                    }
                                    Err(e) => {
                                        println!("Failed to save {}: {e}", active_file.label)
                                    }
                                }
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if modifiers.control_key()
                            && key_event.physical_key == PhysicalKey::Code(KeyCode::KeyW) =>
                        {
                            // Ctrl+W: real keyboard-driven tab close (§75.24,
                            // task #25), the mouse-only gap §75.21 named
                            // explicitly. Reuses exactly the same dirty-check
                            // (raise the unsaved-changes modal, §75.23) and
                            // last-tab guard (`files.len() > 1`) the tab
                            // bar's own `×` click handler already
                            // established, rather than a second, separate
                            // implementation of the same rule.
                            if files[active].dirty && files.len() > 1 {
                                pending_close = Some(PendingClose::File(active));
                                window.request_redraw();
                            } else {
                                let closing = active;
                                close_file(&mut files, &mut active, closing);
                                let active_file = &mut files[active];
                                window.set_title(&window_title(active_file));
                                reshape_window(
                                    &mut text_state,
                                    &active_file.editor,
                                    &active_file.viewport,
                                    active_file.highlighter.as_mut(),
                                );
                                sync_webview_file_info(
                                    &webview_bridge,
                                    &mut component_tree_request,
                                    mode,
                                    &files[active],
                                );
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
                        } if modifiers.control_key()
                            && (key_event.physical_key == PhysicalKey::Code(KeyCode::KeyZ)
                                || key_event.physical_key == PhysicalKey::Code(KeyCode::KeyY)) =>
                        {
                            // Real undo/redo (§75.19) -- Ctrl+Z undoes, both
                            // Ctrl+Y and Ctrl+Shift+Z redo (the two common
                            // conventions), wired to the real branching undo
                            // tree `spartan-buffer` already had implemented
                            // but nothing anywhere in this crate had ever
                            // called. Matched via `physical_key`, same
                            // reasoning as Ctrl+S above.
                            let active_file = &mut files[active];
                            let is_redo = key_event.physical_key == PhysicalKey::Code(KeyCode::KeyY)
                                || (key_event.physical_key == PhysicalKey::Code(KeyCode::KeyZ)
                                    && modifiers.shift_key());
                            let changed = if is_redo {
                                active_file.editor.redo()
                            } else {
                                active_file.editor.undo()
                            };
                            if changed {
                                if !active_file.dirty {
                                    active_file.dirty = true;
                                    window.set_title(&window_title(active_file));
                                }
                                let (cursor_line, _) = active_file.editor.cursor_line_col();
                                let doc_len_lines = active_file.editor.document.len_lines();
                                active_file.viewport.ensure_visible(cursor_line, doc_len_lines);
                                // Unlike a keystroke edit, undo/redo can change an
                                // unbounded amount of content at once (jumping to
                                // any checkpoint in the tree) -- always a full
                                // reshape, never the cheap per-line fast path.
                                reshape_window(
                                    &mut text_state,
                                    &active_file.editor,
                                    &active_file.viewport,
                                    active_file.highlighter.as_mut(),
                                );
                                window.request_redraw();
                            } else {
                                println!(
                                    "Nothing to {}",
                                    if is_redo { "redo" } else { "undo" }
                                );
                            }
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                key_event @ KeyEvent {
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } if modifiers.control_key()
                            && matches!(
                                key_event.physical_key,
                                PhysicalKey::Code(KeyCode::KeyC)
                                    | PhysicalKey::Code(KeyCode::KeyX)
                                    | PhysicalKey::Code(KeyCode::KeyV)
                            ) =>
                        {
                            // Real OS clipboard integration (§75.20):
                            // Ctrl+C copies the active selection, Ctrl+X
                            // cuts it (copy + delete), Ctrl+V pastes
                            // clipboard content at the cursor (replacing an
                            // active selection, via `insert_at_cursor`'s
                            // existing §75.18 behavior). Matched via
                            // `physical_key`, same reasoning as Ctrl+S.
                            let Some(clip) = clipboard.as_mut() else {
                                println!("Clipboard unavailable this session");
                                return;
                            };
                            let active_file = &mut files[active];
                            let mut edited = false;
                            if key_event.physical_key == PhysicalKey::Code(KeyCode::KeyC)
                                || key_event.physical_key == PhysicalKey::Code(KeyCode::KeyX)
                            {
                                match active_file.editor.selected_text() {
                                    Some(text) => {
                                        if let Err(e) = clip.set_text(&text) {
                                            println!("Failed to copy to clipboard: {e}");
                                        } else if key_event.physical_key
                                            == PhysicalKey::Code(KeyCode::KeyX)
                                        {
                                            let effect = active_file.editor.delete_selection();
                                            edited = effect != editor_view::EditEffect::None;
                                        }
                                    }
                                    None => println!("Nothing selected to copy"),
                                }
                            } else {
                                match clip.get_text() {
                                    Ok(text) if !text.is_empty() => {
                                        let effect = active_file.editor.insert_at_cursor(&text);
                                        edited = effect != editor_view::EditEffect::None;
                                    }
                                    Ok(_) => {}
                                    Err(e) => println!("Failed to paste from clipboard: {e}"),
                                }
                            }
                            if edited {
                                edit_latency.note_key_event();
                                active_file.lsp_debouncer.on_edit();
                                if !active_file.dirty {
                                    active_file.dirty = true;
                                    window.set_title(&window_title(active_file));
                                }
                                let (cursor_line, _) = active_file.editor.cursor_line_col();
                                let doc_len_lines = active_file.editor.document.len_lines();
                                active_file.viewport.ensure_visible(cursor_line, doc_len_lines);
                                // A pasted string can be any length and a cut can
                                // span multiple lines -- always a full reshape,
                                // same reasoning as undo/redo just above.
                                reshape_window(
                                    &mut text_state,
                                    &active_file.editor,
                                    &active_file.viewport,
                                    active_file.highlighter.as_mut(),
                                );
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
                            Key::Named(NamedKey::Tab) if modifiers.control_key() => {
                                // Ctrl+Tab / Ctrl+Shift+Tab: cycle the active
                                // file. No visual tab bar exists yet (task
                                // #16) -- this is the keyboard-only v1.
                                active = if modifiers.shift_key() {
                                    (active + files.len() - 1) % files.len()
                                } else {
                                    (active + 1) % files.len()
                                };
                                println!("Switched to: {}", files[active].label);
                                let active_file = &mut files[active];
                                window.set_title(&window_title(active_file));
                                reshape_window(
                                    &mut text_state,
                                    &active_file.editor,
                                    &active_file.viewport,
                                    active_file.highlighter.as_mut(),
                                );
                                window.request_redraw();
                            }
                            Key::Named(NamedKey::PageDown) => {
                                let active_file = &mut files[active];
                                let page = active_file.viewport.visible_lines as isize;
                                let doc_len_lines = active_file.editor.document.len_lines();
                                if active_file.viewport.scroll_by(page, doc_len_lines) {
                                    reshape_window(
                                        &mut text_state,
                                        &active_file.editor,
                                        &active_file.viewport,
                                        active_file.highlighter.as_mut(),
                                    );
                                    window.request_redraw();
                                }
                            }
                            Key::Named(NamedKey::PageUp) => {
                                let active_file = &mut files[active];
                                let page = -(active_file.viewport.visible_lines as isize);
                                let doc_len_lines = active_file.editor.document.len_lines();
                                if active_file.viewport.scroll_by(page, doc_len_lines) {
                                    reshape_window(
                                        &mut text_state,
                                        &active_file.editor,
                                        &active_file.viewport,
                                        active_file.highlighter.as_mut(),
                                    );
                                    window.request_redraw();
                                }
                            }
                            Key::Named(NamedKey::F9) => {
                                let active_file = &mut files[active];
                                let (cursor_line, _) = active_file.editor.cursor_line_col();
                                let line_1indexed = (cursor_line + 1) as i64;
                                if let Some(pos) = active_file
                                    .breakpoints
                                    .iter()
                                    .position(|&l| l == line_1indexed)
                                {
                                    active_file.breakpoints.remove(pos);
                                    println!("Breakpoint removed at line {line_1indexed}");
                                } else {
                                    active_file.breakpoints.push(line_1indexed);
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

                                // F5 always targets the *active* file's own captured DAP
                                // info and its own breakpoints -- a real, named scope
                                // limitation of this pass: there is no unified multi-file
                                // breakpoint set, since the underlying DAP client
                                // (`launch_and_break`) only ever issues one
                                // `setBreakpoints` call, for one source file (see
                                // `dap.rs`). Switching files after setting breakpoints in
                                // a different one and pressing F5 debugs the *active*
                                // file's target with the *active* file's breakpoints only.
                                let active_file = &files[active];
                                if let Some(session) = &dap_session {
                                    session.send_command(dap_session::DapCommand::Continue);
                                } else if pending_build.is_some() {
                                    println!("A build is already in progress -- please wait");
                                } else if let Some((command, binary_path, cwd, source_path)) =
                                    active_file.dap_launch_info.clone()
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
                                        &active_file.breakpoints,
                                    ));
                                } else if let Some((command, project_root, source_path)) =
                                    active_file.dap_build_info.clone()
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
                            Key::Named(NamedKey::Escape) => {
                                // Real, small addition alongside selection
                                // (§75.18): Escape clears an active
                                // selection without moving the cursor.
                                let active_file = &mut files[active];
                                if active_file.editor.selection_anchor.is_some() {
                                    active_file.editor.selection_anchor = None;
                                    window.request_redraw();
                                }
                            }
                            Key::Named(
                                key @ (NamedKey::ArrowLeft
                                | NamedKey::ArrowRight
                                | NamedKey::ArrowUp
                                | NamedKey::ArrowDown
                                | NamedKey::Home
                                | NamedKey::End),
                            ) => {
                                // Real arrow-key navigation (§75.17), Shift-
                                // aware for text selection (§75.18), and
                                // Home/End/Ctrl+Arrow/sticky-column (§75.22)
                                // -- see `handle_navigation_key`'s own doc
                                // comment for the exact interaction rule.
                                let active_file = &mut files[active];
                                let moved = handle_navigation_key(
                                    &mut active_file.editor,
                                    *key,
                                    modifiers.shift_key(),
                                    modifiers.control_key(),
                                );
                                if moved {
                                    let (cursor_line, _) = active_file.editor.cursor_line_col();
                                    let doc_len_lines = active_file.editor.document.len_lines();
                                    if active_file
                                        .viewport
                                        .ensure_visible(cursor_line, doc_len_lines)
                                    {
                                        reshape_window(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            active_file.highlighter.as_mut(),
                                        );
                                    }
                                    window.request_redraw();
                                }
                            }
                            _ => {
                                let active_file = &mut files[active];
                                let effect =
                                    input::handle_key_event(&mut active_file.editor, &key_event);
                                if effect != editor_view::EditEffect::None {
                                    edit_latency.note_key_event();
                                    active_file.lsp_debouncer.on_edit();
                                    if !active_file.dirty {
                                        active_file.dirty = true;
                                        window.set_title(&window_title(active_file));
                                    }
                                    let (cursor_line, _) = active_file.editor.cursor_line_col();
                                    let doc_len_lines = active_file.editor.document.len_lines();
                                    if active_file
                                        .viewport
                                        .ensure_visible(cursor_line, doc_len_lines)
                                    {
                                        // The cursor moved outside the current window (e.g.
                                        // Enter near the bottom edge) -- the viewport itself
                                        // scrolled, so one full reshape against the new window
                                        // covers both the scroll and the edit's own visual
                                        // change, rather than reshaping twice.
                                        reshape_window(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            active_file.highlighter.as_mut(),
                                        );
                                    } else {
                                        apply_edit_effect(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            effect,
                                            active_file.highlighter.as_mut(),
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

                            // Real tab bar (§75.21): rebuilt from real, current
                            // file state every frame -- cheap for a handful of
                            // short tab labels, and simpler than tracking
                            // exactly when files/active/dirty last changed.
                            // Real basenames, not full paths, to keep tabs
                            // compact -- `--synthetic:` fixtures have no path
                            // to take a basename of, so their raw label is
                            // used as-is.
                            let tab_labels: Vec<(String, bool)> = files
                                .iter()
                                .map(|f| {
                                    let name = if f.label.starts_with("--synthetic:") {
                                        f.label.clone()
                                    } else {
                                        Path::new(&f.label)
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| f.label.clone())
                                    };
                                    (name, f.dirty)
                                })
                                .collect();
                            let (tab_bar_text, new_tab_hits) =
                                tab_bar::build_tab_bar_text(&tab_labels);
                            tab_hits = new_tab_hits;
                            text_state.set_tab_bar_text(&tab_bar_text);

                            // Real accessibility tree (§16.3, task #9),
                            // rebuilt from live state every frame just like
                            // the tab bar's own text just above -- the real,
                            // full active-file text (not the windowed
                            // rendering slice), so a screen reader sees the
                            // real document, not just what's on screen.
                            let active_file_for_a11y = &files[active];
                            let a11y_tabs: Vec<accessibility::TabInfo> = files
                                .iter()
                                .map(|f| accessibility::TabInfo {
                                    label: &f.label,
                                    dirty: f.dirty,
                                })
                                .collect();
                            let window_title_for_a11y = window_title(active_file_for_a11y);
                            let active_text_for_a11y = active_file_for_a11y.editor.text();
                            accesskit_adapter.update_if_active(|| {
                                accessibility::build_tree(
                                    &window_title_for_a11y,
                                    &a11y_tabs,
                                    active,
                                    &active_text_for_a11y,
                                )
                            });

                            // Real tab bar horizontal auto-scroll (§75.28,
                            // task #25's overflow half): scrolls minimally
                            // so the active tab is always reachable, even
                            // once enough tabs are open to overflow the
                            // window's width -- the horizontal analogue of
                            // `Viewport::ensure_visible`, run every frame
                            // just like the tab bar's own text is rebuilt
                            // every frame above.
                            // Narrowed by `MODE_TOGGLE_WIDTH` (§8, task
                            // #3), matching the tab bar's own real,
                            // narrower clip bounds in `TextState::prepare`.
                            let tab_bar_visible_width = (gpu_state.size.width as f32
                                - text::TEXT_ORIGIN_X
                                - text::MODE_TOGGLE_WIDTH)
                                .max(0.0);
                            if let Some(active_hit) =
                                tab_hits.iter().find(|hit| hit.file_index == active)
                            {
                                text_state.ensure_tab_visible(
                                    active_hit.tab_range.clone(),
                                    tab_bar_visible_width,
                                );
                            }

                            // Real left-sidebar text (§75.26 file tree /
                            // §56.1 Source Control, task #7), rebuilt from
                            // fresh real state every frame just like the tab
                            // bar's own text above -- `sidebar_rows`/
                            // `git_rows` are kept around (like `tab_hits`) so
                            // a later `MouseInput` can resolve a click
                            // against the exact same row list that's
                            // actually on screen. Only the active mode's
                            // real state is queried each frame (a real git
                            // `status()` walk is real I/O, not free) -- the
                            // other mode's row list is left stale rather
                            // than cleared, but it's never rendered or
                            // clicked while its mode isn't active, so a
                            // stale list there is inert.
                            match sidebar_mode {
                                SidebarMode::FileTree => {
                                    sidebar_rows = file_tree
                                        .as_ref()
                                        .map(|t| t.visible_rows())
                                        .unwrap_or_default();
                                    text_state
                                        .set_sidebar_text(&file_tree::build_tree_text(&sidebar_rows));
                                }
                                SidebarMode::SourceControl => {
                                    let git_status: Vec<spartan_git::StatusEntry> = git_repo
                                        .as_ref()
                                        .and_then(|r| r.status().ok())
                                        .unwrap_or_default();
                                    git_rows = git_panel::build_rows(&git_status);
                                    text_state.set_sidebar_text(&git_panel::build_panel_text(
                                        &git_rows,
                                        &git_status,
                                    ));
                                }
                            }

                            // Real modal text (§75.23 unsaved-changes / §56.1
                            // commit message, task #7), rebuilt from live
                            // state every frame just like the tab bar's own
                            // text just above -- empty when neither modal is
                            // showing, which shapes to zero glyphs at
                            // effectively no cost. The two modals are
                            // mutually exclusive in practice (see the
                            // `commit_message.is_none()` guard everywhere
                            // `pending_close` can be set, and vice versa).
                            let modal_text = if let Some(pending) = &pending_close {
                                modal_message(pending, &files)
                            } else if let Some(message) = &commit_message {
                                format!(
                                    "Commit message (Enter to commit, Escape to cancel):\n\n{message}_"
                                )
                            } else if let Some(palette) = &command_palette_state {
                                // Real command palette text (§16.1, task
                                // #3), checked before the mode placeholder
                                // just below since the palette can be
                                // opened from any mode (Ctrl+P isn't gated
                                // on `mode == Editor`) and should always
                                // take rendering priority over it while
                                // open.
                                command_palette::build_palette_text(
                                    &palette.query,
                                    &palette.filtered,
                                    palette.selected,
                                )
                            } else if let Some(placeholder) = mode.placeholder_message() {
                                // Real Agent/Design mode placeholder (§8,
                                // task #3) -- reuses this same modal
                                // rendering/dim-overlay infrastructure
                                // rather than a sixth glyphon `TextArea`,
                                // the same way the commit modal already
                                // reused it instead of the unsaved-changes
                                // modal's own dedicated buffer.
                                placeholder.to_string()
                            } else {
                                String::new()
                            };
                            text_state.set_modal_text(&modal_text);

                            // Real mode toggle text (§8, §16.1, task #3),
                            // rebuilt every frame just like the tab bar's
                            // own text -- cheap for three fixed short
                            // labels.
                            let (mode_toggle_text, new_mode_hits) =
                                mode_toggle::build_mode_toggle_text();
                            let active_mode_range = new_mode_hits
                                .iter()
                                .find(|hit| hit.mode == mode)
                                .map(|hit| hit.range.clone())
                                .unwrap_or(0..0);
                            mode_hits = new_mode_hits;
                            text_state.set_mode_toggle_text(&mode_toggle_text, active_mode_range);

                            text_state
                                .prepare(
                                    &gpu_state.device,
                                    &gpu_state.queue,
                                    gpu_state.size.width,
                                    gpu_state.size.height,
                                )
                                .expect("glyphon text prepare failed");

                            // Real active-tab highlight rect (§75.21), via the
                            // real char range `build_tab_bar_text` already
                            // computed for the active file's tab and the same
                            // real pixel lookup the tab bar's own hit-testing
                            // uses in reverse.
                            let active_tab_rects: Vec<selection::SelectionRect> = tab_hits
                                .iter()
                                .find(|hit| hit.file_index == active)
                                .and_then(|hit| {
                                    let x_start = text_state.tab_bar_pixel_pos(hit.tab_range.start)?;
                                    let x_end = text_state.tab_bar_pixel_pos(hit.tab_range.end)?;
                                    let scroll = text_state.tab_bar_scroll();
                                    Some(selection::SelectionRect {
                                        x: text::TEXT_ORIGIN_X + x_start - scroll,
                                        y: 0.0,
                                        width: x_end - x_start,
                                        height: text::TAB_BAR_HEIGHT,
                                        color: selection::ACCENT_HIGHLIGHT,
                                    })
                                })
                                .into_iter()
                                .collect();
                            tab_bar_renderer.update(
                                &gpu_state.device,
                                &gpu_state.queue,
                                &active_tab_rects,
                                &cursor::ScreenSize {
                                    width: gpu_state.size.width as f32,
                                    height: gpu_state.size.height as f32,
                                },
                            );

                            let active_file = &files[active];
                            let (cursor_doc_line, cursor_col) = active_file.editor.cursor_line_col();
                            let doc_len_lines = active_file.editor.document.len_lines();
                            let cursor_pixel_pos = viewport::to_local_line(
                                cursor_doc_line,
                                &active_file.viewport,
                                doc_len_lines,
                            )
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

                            // Real selection-highlight rects (§75.18), one per
                            // visual line the active selection spans -- reuses
                            // `TextState::cursor_pixel_pos` (the same lookup the
                            // caret itself uses) to turn each line's char-column
                            // span into real pixel positions. `usize::MAX` for
                            // an unbounded `end_col` deliberately leans on
                            // `cursor_pixel_pos`'s own existing end-of-line
                            // fallback (an out-of-range column resolves to the
                            // last laid-out glyph's right edge) rather than
                            // needing a second, separate "width of this line"
                            // lookup.
                            let mut selection_rects: Vec<selection::SelectionRect> = Vec::new();
                            if let Some((sel_start, sel_end)) = active_file.editor.selection_range()
                            {
                                let spans = viewport::selection_line_spans(
                                    &active_file.editor.document,
                                    sel_start,
                                    sel_end,
                                );
                                for (doc_line, start_col, end_col) in spans {
                                    let Some(local_line) = viewport::to_local_line(
                                        doc_line,
                                        &active_file.viewport,
                                        doc_len_lines,
                                    ) else {
                                        continue;
                                    };
                                    let Some((x_start, y)) =
                                        text_state.cursor_pixel_pos(local_line, start_col)
                                    else {
                                        continue;
                                    };
                                    let x_end = match end_col {
                                        Some(c) => text_state.cursor_pixel_pos(local_line, c),
                                        None => text_state.cursor_pixel_pos(local_line, usize::MAX),
                                    }
                                    .map(|(x, _)| x)
                                    .unwrap_or(x_start);
                                    if x_end > x_start {
                                        selection_rects.push(selection::SelectionRect {
                                            x: text::TEXT_ORIGIN_X + x_start,
                                            y: text::TEXT_ORIGIN_Y + y,
                                            width: x_end - x_start,
                                            height: text::LINE_HEIGHT,
                                            color: selection::ACCENT_HIGHLIGHT,
                                        });
                                    }
                                }
                            }
                            selection_renderer.update(
                                &gpu_state.device,
                                &gpu_state.queue,
                                &selection_rects,
                                &cursor::ScreenSize {
                                    width: gpu_state.size.width as f32,
                                    height: gpu_state.size.height as f32,
                                },
                            );

                            // Real unsaved-changes modal dim overlay
                            // (§75.23) -- a single full-window rect when a
                            // modal is up, none otherwise. Text (both the
                            // editor's own and the modal's) still draws on
                            // top of this in the same text pass, the same
                            // "quads before text" ordering `selection_rects`
                            // above already relies on.
                            let modal_rects: Vec<selection::SelectionRect> =
                                if pending_close.is_some()
                                    || commit_message.is_some()
                                    || command_palette_state.is_some()
                                    || mode.placeholder_message().is_some()
                                {
                                    vec![selection::SelectionRect {
                                        x: 0.0,
                                        y: 0.0,
                                        width: gpu_state.size.width as f32,
                                        height: gpu_state.size.height as f32,
                                        color: MODAL_DIM_COLOR,
                                    }]
                                } else {
                                    Vec::new()
                                };
                            modal_renderer.update(
                                &gpu_state.device,
                                &gpu_state.queue,
                                &modal_rects,
                                &cursor::ScreenSize {
                                    width: gpu_state.size.width as f32,
                                    height: gpu_state.size.height as f32,
                                },
                            );

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
                                tab_bar_renderer.render(&mut pass);
                                selection_renderer.render(&mut pass);
                                modal_renderer.render(&mut pass);
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
                            // are surfaced to stdout here, for every open file's own
                            // session, labeled with that file so background-file
                            // diagnostics aren't confused with the active file's.
                            for file in files.iter() {
                                if let Some(session) = &file.lsp_session {
                                    for update in session.poll_updates() {
                                        let lsp_session::LspUpdate::Diagnostics(lines) = update;
                                        println!(
                                            "LSP diagnostics for {} ({} item(s)):",
                                            file.label,
                                            lines.len()
                                        );
                                        for line in lines {
                                            println!("  {line}");
                                        }
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
                                                &files[active].breakpoints,
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
                Event::UserEvent(accesskit_winit::Event { window_event, .. }) => {
                    match window_event {
                        accesskit_winit::WindowEvent::InitialTreeRequested => {
                            let active_file = &files[active];
                            let tabs: Vec<accessibility::TabInfo> = files
                                .iter()
                                .map(|f| accessibility::TabInfo {
                                    label: &f.label,
                                    dirty: f.dirty,
                                })
                                .collect();
                            accesskit_adapter.update_if_active(|| {
                                accessibility::build_tree(
                                    &window_title(active_file),
                                    &tabs,
                                    active,
                                    &active_file.editor.text(),
                                )
                            });
                        }
                        // Real, named gap (§16.3): no `ActionRequest`
                        // handling yet (e.g. a screen reader's own
                        // "activate this tab" request) -- the tree is
                        // real and readable, but not yet actionable from
                        // the assistive-technology side. See this
                        // increment's own documentation for the full list
                        // of what remains.
                        accesskit_winit::WindowEvent::ActionRequested(_) => {}
                        accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
                    }
                }
                Event::AboutToWait => {
                    // Real GTK event-loop pump (§6.1, task #12), required
                    // on Linux/BSD by wry's own documented contract: since
                    // winit doesn't drive GTK's own main loop, WebKitGTK's
                    // real rendering/input/IPC processing would otherwise
                    // never actually run, even though the WebView object
                    // exists. A genuine bug was found here only by running
                    // this crate live, not by inspection: `wry::
                    // WebViewBuilder::build()` panics with "GDK has not
                    // been initialized" the first time a WebView is
                    // created, unless `gtk::init()` has already run (see
                    // its call near the top of `main()`) -- this per-frame
                    // pump is the other documented half of that same real
                    // contract.
                    #[cfg(any(
                        target_os = "linux",
                        target_os = "dragonfly",
                        target_os = "freebsd",
                        target_os = "openbsd",
                        target_os = "netbsd"
                    ))]
                    while gtk::events_pending() {
                        gtk::main_iteration_do(false);
                    }

                    // Real §75.41 dev-server bridge poll -- non-blocking
                    // (`try_recv`), matching `pending_build`'s own
                    // established pattern for a different real subprocess.
                    // Only meaningful while a request is actually
                    // in-flight; a stale result for a file the user has
                    // since switched away from can't happen, since
                    // `sync_webview_content` always replaces (not just
                    // adds to) `component_tree_request` before a new
                    // subprocess is spawned.
                    if let Some(request) = &component_tree_request {
                        if let Ok(result) = request.receiver.try_recv() {
                            if let Some(bridge) = &webview_bridge {
                                match result {
                                    Ok(json) => bridge.push_component_tree(&json),
                                    Err(message) => bridge.push_component_tree_error(&message),
                                }
                            }
                            component_tree_request = None;
                        }
                    }

                    // Real §75.42 Canvas -> Code poll: the moment the
                    // WebView's edit form posts a real `CanvasEdit` over
                    // IPC, spawn a real apply-edit subprocess fed the
                    // *active file's live buffer* (`editor.text()`, not
                    // whatever's on disk) as its stdin -- an edit must
                    // never silently discard or race against unsaved
                    // keystrokes already made in the real editor. Dropped
                    // (not queued) if a previous apply is still in flight,
                    // since a second `CanvasEdit` before the first
                    // resolves would target node ids computed against a
                    // tree that no longer matches the file this second
                    // request itself needs to be based on.
                    if pending_apply_edit.is_none() {
                        if let Some(bridge) = &webview_bridge {
                            if let Some(edit_json) = bridge.take_pending_edit() {
                                let active_file = &files[active];
                                let source = active_file.editor.text();
                                pending_apply_edit =
                                    Some(gui_bridge::spawn_apply_edit_request(source, edit_json));
                            }
                        }
                    }

                    // Real §75.42 apply-edit poll, same non-blocking
                    // pattern as `component_tree_request` just above. A
                    // successful apply replaces the active file's entire
                    // real live buffer via `EditorView::replace_all_text`
                    // -- going through the same undo/dirty-tracking path
                    // any other edit already does -- then immediately
                    // re-requests a fresh component tree, since node ids
                    // are a pure function of tree structure and a
                    // structural edit can shift every id after the one
                    // just changed. Deliberately does *not* call
                    // `edit_latency.note_key_event()`: this isn't a
                    // keystroke, and folding a real subprocess round-trip
                    // (tens-to-hundreds of ms) into the input-to-photon
                    // typing-latency report would misrepresent what that
                    // report measures, not just add noise to it.
                    if let Some(request) = &pending_apply_edit {
                        if let Ok(result) = request.receiver.try_recv() {
                            pending_apply_edit = None;
                            match result {
                                Ok(new_source) => {
                                    let active_file = &mut files[active];
                                    let effect = active_file.editor.replace_all_text(&new_source);
                                    if effect != editor_view::EditEffect::None {
                                        if !active_file.dirty {
                                            active_file.dirty = true;
                                            window.set_title(&window_title(active_file));
                                        }
                                        active_file.lsp_debouncer.on_edit();
                                        let (cursor_line, _) = active_file.editor.cursor_line_col();
                                        let doc_len_lines = active_file.editor.document.len_lines();
                                        active_file
                                            .viewport
                                            .ensure_visible(cursor_line, doc_len_lines);
                                        reshape_window(
                                            &mut text_state,
                                            &active_file.editor,
                                            &active_file.viewport,
                                            active_file.highlighter.as_mut(),
                                        );
                                    }
                                    if let Some(bridge) = &webview_bridge {
                                        bridge.push_edit_applied();
                                        bridge.push_component_tree_loading();
                                    }
                                    component_tree_request =
                                        Some(gui_bridge::spawn_component_tree_request(Path::new(
                                            &files[active].label,
                                        )));
                                    window.request_redraw();
                                }
                                Err(message) => {
                                    if let Some(bridge) = &webview_bridge {
                                        bridge.push_edit_error(&message);
                                    }
                                }
                            }
                        }
                    }

                    if edit_bench_remaining > 0 {
                        let bench_file = &mut files[0];
                        let effect = bench_file.editor.insert_random(&mut bench_rng, "x");
                        if effect != editor_view::EditEffect::None {
                            edit_latency.note_key_event();
                            let redrew = apply_edit_effect(
                                &mut text_state,
                                &bench_file.editor,
                                &bench_file.viewport,
                                effect,
                                None,
                            );
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
                        let bench_file = &mut files[0];
                        let effect = bench_file.editor.insert_at_cursor("x");
                        if effect != editor_view::EditEffect::None {
                            cursor_latency.note_key_event();
                            apply_edit_effect(
                                &mut text_state,
                                &bench_file.editor,
                                &bench_file.viewport,
                                effect,
                                None,
                            );
                        }
                        cursor_bench_remaining -= 1;
                    } else if scroll_bench_remaining > 0 {
                        let bench_file = &mut files[0];
                        let doc_len_lines = bench_file.editor.document.len_lines();
                        let direction = if bench_file.viewport.scroll_line == 0 {
                            1isize
                        } else {
                            -1isize
                        };
                        let page = direction * bench_file.viewport.visible_lines as isize;
                        scroll_latency.note_key_event();
                        if bench_file.viewport.scroll_by(page, doc_len_lines) {
                            reshape_window(&mut text_state, &bench_file.editor, &bench_file.viewport, None);
                        }
                        scroll_bench_remaining -= 1;
                    }
                    // Polled every tick because this crate runs `ControlFlow::Poll`
                    // unconditionally (see `OpenFile::lsp_debouncer`'s declaration
                    // comment) -- every open file's own debouncer/session, not just the
                    // active one's, so a background file is never silently starved.
                    for file in files.iter_mut() {
                        if file.lsp_debouncer.should_dispatch_now() {
                            if let Some(session) = &file.lsp_session {
                                session.notify_edit(file.editor.text());
                            }
                        }
                    }
                    window.request_redraw();
                }
                _ => {}
            }
        })
        .expect("event loop exited with an error");
}
