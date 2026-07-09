use glyphon::{
    Attrs, AttrsList, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Wrap,
};
use spartan_editor_core::highlight::HighlightSpan;

/// Owns everything glyphon/cosmic-text needs to shape and rasterize real
/// text onto the GPU atlas each frame. Promoted from
/// `spikes/render-spike/src/text.rs` (§39.1, §47.9-§47.10) -- the struct and
/// its methods needed no logic changes to become real product code, because
/// they were already agnostic to *how much* text they're given. What
/// changed is the caller (`main.rs`): it now always passes this a
/// `viewport::windowed_text()` slice (~40-60 lines), never the whole
/// document, which is what actually closes render-spike's own named
/// cold-open gap -- see the crate README for the real, measured effect.
pub struct TextState {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub atlas: TextAtlas,
    pub renderer: TextRenderer,
    pub buffer: Buffer,
    /// Real tab bar rendering (§75.21) -- a second, independent cosmic-text
    /// `Buffer` for the tab-label row, sharing this same `FontSystem`/
    /// `TextAtlas`/`TextRenderer`/`SwashCache` rather than needing a whole
    /// parallel rendering pipeline: `TextRenderer::prepare` already accepts
    /// *multiple* `TextArea`s in one call (confirmed against the actual
    /// installed `glyphon-0.5.0` source before relying on it, not assumed),
    /// so both buffers are prepared and rendered together in `prepare()`.
    pub tab_bar_buffer: Buffer,
    /// Real unsaved-changes modal text (§75.23) -- a third, independent
    /// cosmic-text `Buffer`, same reasoning as `tab_bar_buffer`: shares this
    /// same `FontSystem`/`TextAtlas`/`TextRenderer`/`SwashCache` rather than
    /// a parallel pipeline. Empty (no text set) whenever no modal is
    /// showing, which is enough to make it render nothing -- no separate
    /// on/off flag needed here, matching how the tab bar itself has no
    /// "hidden" state distinct from "empty content."
    pub modal_buffer: Buffer,
    /// Real file tree sidebar text (§75.26) -- a fourth, independent
    /// cosmic-text `Buffer`, same sharing reasoning as `tab_bar_buffer`/
    /// `modal_buffer`. One real line of text per visible tree row (see
    /// `file_tree::build_tree_text`), so hit-testing a click just needs the
    /// clicked line index, not a per-row char-range list the way the tab
    /// bar's single-line-many-tabs layout needs.
    pub sidebar_buffer: Buffer,
    /// Real tab bar horizontal scroll (§75.28, the overflow half of task
    /// #25) -- the pixel offset the tab bar's rendered text and
    /// hit-testing are both shifted by, so tabs beyond the visible strip's
    /// right edge can still be reached once enough tabs are open to
    /// overflow the window's width. The horizontal analogue of
    /// `Viewport::scroll_line`, but in real pixels (tabs have variable
    /// width) rather than line counts. Kept private -- `ensure_tab_visible`
    /// is the only way to change it, and `tab_bar_scroll()` the only way to
    /// read it, the same encapsulation `Viewport` itself uses for
    /// `scroll_line`.
    tab_bar_scroll: f32,
}

pub const FONT_SIZE: f32 = 16.0;
pub const LINE_HEIGHT: f32 = 20.0;

/// Real tab bar row height (§75.21), reserved at the top of the window --
/// `TEXT_ORIGIN_Y` below is defined in terms of this, so every existing
/// call site that already uses `TEXT_ORIGIN_Y` symbolically (cursor
/// position, hit-testing, visible-line count) shifts down to make room for
/// it automatically, with no other call site needing to change.
pub const TAB_BAR_HEIGHT: f32 = 28.0;
pub const TAB_BAR_FONT_SIZE: f32 = 13.0;
pub const TAB_BAR_LINE_HEIGHT: f32 = 20.0;
/// Rough vertical centering of the tab bar's own text within
/// `TAB_BAR_HEIGHT` -- not pixel-perfect typographic centering (that would
/// need real font-metrics ascent/descent, not attempted here), just visibly
/// reasonable.
pub const TAB_BAR_TEXT_TOP: f32 = (TAB_BAR_HEIGHT - TAB_BAR_LINE_HEIGHT) / 2.0;

/// Real file tree sidebar width (§75.26), reserved on the left of the
/// window -- `TEXT_ORIGIN_X` below is defined in terms of this, so every
/// existing call site that already uses `TEXT_ORIGIN_X` symbolically (main
/// editor and tab bar rendering *and* hit-testing, both already routed
/// through this one constant) shifts right to make room for it
/// automatically, the exact same trick `TAB_BAR_HEIGHT`/`TEXT_ORIGIN_Y`
/// already used in §75.21 to make room vertically. The sidebar itself uses
/// its own, separate small left/top padding (`SIDEBAR_TEXT_LEFT`/
/// `SIDEBAR_TEXT_TOP`), not `TEXT_ORIGIN_X`/`TEXT_ORIGIN_Y` -- it isn't
/// positioned relative to itself.
pub const SIDEBAR_WIDTH: f32 = 200.0;
pub const SIDEBAR_FONT_SIZE: f32 = 13.0;
pub const SIDEBAR_LINE_HEIGHT: f32 = 18.0;
pub const SIDEBAR_TEXT_LEFT: f32 = 8.0;
pub const SIDEBAR_TEXT_TOP: f32 = 8.0;

/// Text origin within the window, in pixels -- kept as constants (rather than
/// re-typing `8.0` in both `prepare()`'s `TextArea` and the cursor-position
/// math in `main.rs`) so the two can never drift apart and mis-align the
/// caret against the glyphs it's supposed to sit next to.
pub const TEXT_ORIGIN_X: f32 = SIDEBAR_WIDTH + 8.0;
pub const TEXT_ORIGIN_Y: f32 = 8.0 + TAB_BAR_HEIGHT;

impl TextState {
    /// Takes an already-constructed `FontSystem` rather than building one
    /// internally. `FontSystem::new()` scans and parses every font on the
    /// system -- a real, measured ~93-97ms cost (§75.9) that has nothing to
    /// do with the GPU device/queue this constructor otherwise needs, so
    /// `main.rs` builds it on a background thread concurrently with
    /// `GpuState::new()`'s async GPU setup instead of paying both costs
    /// back-to-back on the same thread.
    pub fn new(
        mut font_system: FontSystem,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: f32,
        height: f32,
    ) -> Self {
        let swash_cache = SwashCache::new();
        let mut atlas = TextAtlas::new(device, queue, surface_format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        buffer.set_size(&mut font_system, width, height);

        let mut tab_bar_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(TAB_BAR_FONT_SIZE, TAB_BAR_LINE_HEIGHT),
        );
        tab_bar_buffer.set_size(&mut font_system, width, TAB_BAR_HEIGHT);
        // Real bug found only by testing tab overflow with enough tabs open
        // (§75.28), not by inspection: cosmic-text's default `Wrap::Word`
        // silently word-wraps text onto additional internal layout runs
        // once it exceeds the buffer's configured *width* -- which,
        // pre-§75.28, never mattered (the tab bar was never scrolled, and
        // wrapped-off tabs were simply invisible either way), but broke
        // `tab_bar_pixel_pos`'s "always exactly one layout run" assumption
        // once real horizontal scrolling started depending on it: a tab
        // past the wrap point resolved to a wrong, too-small pixel position
        // (silently clamped to the *first* run's own end, not its real
        // position), which under-scrolled and could never actually bring
        // it into view. The tab bar is conceptually always exactly one
        // real line, regardless of how wide it gets -- `Wrap::None` makes
        // that real, not just assumed.
        tab_bar_buffer.set_wrap(&mut font_system, Wrap::None);

        let mut modal_buffer = Buffer::new(&mut font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        modal_buffer.set_size(&mut font_system, width, height);

        let mut sidebar_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(SIDEBAR_FONT_SIZE, SIDEBAR_LINE_HEIGHT),
        );
        sidebar_buffer.set_size(&mut font_system, SIDEBAR_WIDTH, height);

        Self {
            font_system,
            swash_cache,
            atlas,
            renderer,
            buffer,
            tab_bar_buffer,
            modal_buffer,
            sidebar_buffer,
            tab_bar_scroll: 0.0,
        }
    }

    /// Replaces the tab bar's entire content -- always a full reshape
    /// (there's no windowing/per-line fast path for this, unlike the main
    /// editor buffer, since the tab bar is always short). No syntax
    /// highlighting, just the plain monospace text `tab_bar::
    /// build_tab_bar_text` already produced.
    pub fn set_tab_bar_text(&mut self, text: &str) {
        self.tab_bar_buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.tab_bar_buffer
            .shape_until_scroll(&mut self.font_system);
    }

    /// Replaces the unsaved-changes modal's entire content (§75.23). Same
    /// "always a full reshape" reasoning as `set_tab_bar_text` -- the modal
    /// is short-lived and its text is never large enough to need a windowed
    /// or per-line fast path. An empty string is the real, ordinary "no
    /// modal showing" state, not a special case this method has to guard
    /// against.
    pub fn set_modal_text(&mut self, text: &str) {
        self.modal_buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.modal_buffer.shape_until_scroll(&mut self.font_system);
    }

    /// Replaces the file tree sidebar's entire content (§75.26). Same
    /// "always a full reshape" reasoning as `set_tab_bar_text` -- a file
    /// tree is never large enough in practice to need a windowed or
    /// per-line fast path the way the main editor buffer does.
    pub fn set_sidebar_text(&mut self, text: &str) {
        self.sidebar_buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.sidebar_buffer
            .shape_until_scroll(&mut self.font_system);
    }

    /// Replaces the buffer's entire content -- a full reshape, but of
    /// whatever text the caller gives it. When the caller passes a windowed
    /// slice (the normal case, see `main.rs`), this is cheap regardless of
    /// document size. Still required for structural edits (a newline
    /// inserted or removed) and for re-slicing after a scroll, since
    /// cosmic-text has no public API for cheap line insert/delete.
    pub fn set_text(&mut self, text: &str) {
        self.buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.buffer.shape_until_scroll(&mut self.font_system);
    }

    /// Same as `set_text`, but colors each byte range in `spans`
    /// differently via `Buffer::set_rich_text` -- the real cosmic-text API
    /// for rendering more than one color within a buffer, confirmed
    /// against the actual installed `cosmic-text-0.10.0` source before
    /// writing this (§75.11). Bytes outside every span keep the plain
    /// monospace `Attrs` (no explicit color, so `TextArea::default_color`
    /// applies at render time). `spans` must be sorted by `start_byte` and
    /// non-overlapping -- `highlight::Highlighter::highlight()`'s output
    /// already satisfies this (it walks the source in byte order), so
    /// callers passing that straight through are safe; this method does
    /// not re-sort or validate, since re-validating output from the one
    /// real producer of `HighlightSpan`s on every reshape would be pure
    /// overhead.
    pub fn set_text_highlighted(&mut self, text: &str, spans: &[HighlightSpan]) {
        let default_attrs = Attrs::new().family(Family::Monospace);
        let mut rich_spans: Vec<(&str, Attrs)> = Vec::new();
        let mut cursor = 0usize;
        for span in spans {
            if span.start_byte > cursor {
                rich_spans.push((&text[cursor..span.start_byte], default_attrs));
            }
            rich_spans.push((
                &text[span.start_byte..span.end_byte],
                default_attrs.color(span.color),
            ));
            cursor = span.end_byte;
        }
        if cursor < text.len() {
            rich_spans.push((&text[cursor..], default_attrs));
        }
        self.buffer
            .set_rich_text(&mut self.font_system, rich_spans, Shaping::Advanced);
        self.buffer.shape_until_scroll(&mut self.font_system);
    }

    /// Real damage-region path: replaces only the ONE line whose text
    /// changed, via `BufferLine::set_text` -- a public cosmic-text API that
    /// invalidates only that line's cached shape/layout, leaving every
    /// other line's already-computed shape/layout untouched.
    /// `shape_until_scroll` then re-shapes only lines whose cache was
    /// invalidated, skipping the rest.
    ///
    /// `line_i` here is a *window-local* index (relative to the current
    /// viewport's `scroll_line`), not a document-absolute one -- callers
    /// must translate via `viewport::to_local_line` first. Only valid for
    /// edits that don't change the document's line count -- callers must
    /// route structural edits through `set_text` instead (see
    /// `editor_view::EditEffect::Structural`). Callers must also check
    /// `line_i < self.line_count()` first (see that method's doc comment
    /// for why) -- an out-of-range `line_i` here is silently ignored rather
    /// than panicking.
    pub fn set_line_text(&mut self, line_i: usize, text: &str) {
        if let Some(line) = self.buffer.lines.get_mut(line_i) {
            line.set_text(text, AttrsList::new(Attrs::new().family(Family::Monospace)));
        }
        self.buffer.shape_until_scroll(&mut self.font_system);
    }

    /// Number of lines cosmic-text's `Buffer` currently knows about --
    /// bounded by the viewport's `visible_lines` now, not document size.
    ///
    /// A real bug `render-spike` found by running it, still relevant in the
    /// windowed context: `Document` (ropey) and cosmic-text disagree about
    /// line counts on text ending in "\n" -- cosmic-text never synthesizes a
    /// `BufferLine` past the last line terminator. If a windowed slice's
    /// last line is exactly the document's own phantom trailing empty line,
    /// the same mismatch recurs at the window's own local boundary. Callers
    /// must check `line_i < line_count()` and fall back to `set_text`
    /// (re-slicing the window) otherwise.
    pub fn line_count(&self) -> usize {
        self.buffer.lines.len()
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.buffer.set_size(&mut self.font_system, width, height);
        self.tab_bar_buffer
            .set_size(&mut self.font_system, width, TAB_BAR_HEIGHT);
        self.modal_buffer
            .set_size(&mut self.font_system, width, height);
        self.sidebar_buffer
            .set_size(&mut self.font_system, SIDEBAR_WIDTH, height);
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<(), glyphon::PrepareError> {
        self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            Resolution { width, height },
            [
                TextArea {
                    buffer: &self.buffer,
                    left: TEXT_ORIGIN_X,
                    top: TEXT_ORIGIN_Y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: TAB_BAR_HEIGHT as i32,
                        right: width as i32,
                        bottom: height as i32,
                    },
                    default_color: Color::rgb(0xE9, 0xE7, 0xE4),
                },
                TextArea {
                    buffer: &self.tab_bar_buffer,
                    // Real tab bar horizontal scroll (§75.28): shifting
                    // `left` while `bounds` stays fixed to the visible
                    // strip is what makes the tab bar scroll -- glyphon
                    // clips anything outside `bounds` regardless of where
                    // `left` puts it, so a large `tab_bar_scroll` slides
                    // earlier tabs out of view to the left without them
                    // reappearing anywhere they shouldn't.
                    left: TEXT_ORIGIN_X - self.tab_bar_scroll,
                    top: TAB_BAR_TEXT_TOP,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: SIDEBAR_WIDTH as i32,
                        top: 0,
                        right: width as i32,
                        bottom: TAB_BAR_HEIGHT as i32,
                    },
                    default_color: Color::rgb(0xE9, 0xE7, 0xE4),
                },
                // Real unsaved-changes modal text (§75.23) -- roughly
                // vertically centered in the real current window height
                // (not pixel-perfect centering of the text block itself,
                // same "visibly reasonable, not typographically exact"
                // precedent `TAB_BAR_TEXT_TOP` already set), rendered over
                // the dim overlay `main.rs` draws in the same "quads before
                // text" phase every other highlight already uses. Empty
                // text (the ordinary non-modal state) shapes to zero glyphs,
                // so this costs nothing when no modal is showing.
                TextArea {
                    buffer: &self.modal_buffer,
                    left: TEXT_ORIGIN_X,
                    top: (height as f32 / 2.0 - 40.0).max(TAB_BAR_HEIGHT),
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    },
                    default_color: Color::rgb(0xE9, 0xE7, 0xE4),
                },
                // Real file tree sidebar text (§75.26) -- its own
                // coordinate space starting at the window's left edge (not
                // `TEXT_ORIGIN_X`, which already accounts for the
                // sidebar's own width), clipped to `SIDEBAR_WIDTH` so a
                // long file name can't visually bleed into the main
                // editor. Empty text (no project root known) shapes to
                // zero glyphs, same "no separate on/off flag" pattern
                // every other optional buffer here already uses.
                TextArea {
                    buffer: &self.sidebar_buffer,
                    left: SIDEBAR_TEXT_LEFT,
                    top: SIDEBAR_TEXT_TOP,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: SIDEBAR_WIDTH as i32,
                        bottom: height as i32,
                    },
                    default_color: Color::rgb(0xE9, 0xE7, 0xE4),
                },
            ],
            &mut self.swash_cache,
        )
    }

    pub fn render<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
    ) -> Result<(), glyphon::RenderError> {
        self.renderer.render(&self.atlas, pass)
    }

    /// Maps a (window-local-line, column-in-chars) position to a pixel-space
    /// `(x, y)` for cursor rendering, matching this buffer's actual shaped
    /// glyph layout. `line` must already be a window-local index (translated
    /// via `viewport::to_local_line`) -- if the cursor's document-absolute
    /// line isn't currently in the viewport at all, callers should skip
    /// calling this entirely and simply not draw a cursor (this increment
    /// has no auto-scroll-to-cursor -- a named, deliberate simplification,
    /// not an oversight; see the crate README).
    ///
    /// `LayoutGlyph::start`/`end` are *byte* offsets within the line's text,
    /// while `col_chars` is a *char* count -- the two only coincide for
    /// pure-ASCII lines, so the column is first converted to a byte offset
    /// via the run's own line text.
    pub fn cursor_pixel_pos(&self, line: usize, col_chars: usize) -> Option<(f32, f32)> {
        let mut last_seen: Option<(usize, f32)> = None;
        for run in self.buffer.layout_runs() {
            if run.line_i == line {
                let byte_offset = run
                    .text
                    .char_indices()
                    .nth(col_chars)
                    .map(|(b, _)| b)
                    .unwrap_or(run.text.len());

                let x = run
                    .glyphs
                    .iter()
                    .find(|g| byte_offset >= g.start && byte_offset < g.end)
                    .map(|g| g.x)
                    .unwrap_or_else(|| run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0));

                return Some((x, run.line_top));
            }
            last_seen = Some((run.line_i, run.line_top));
        }

        // Real mismatch render-spike found by running it, not spotted by
        // inspection: `Document` (ropey) treats text ending in "\n" as
        // having one more, empty line after that final newline than
        // cosmic-text's `Buffer` does. Rather than silently failing to draw
        // a cursor whenever the caret sits at true end-of-window on such
        // text, treat that specific case (one line past the last laid-out
        // line, at its start) as one row below the last real line.
        let (last_line_i, last_line_top) = last_seen?;
        if line > last_line_i && col_chars == 0 {
            Some((
                0.0,
                last_line_top + (line - last_line_i) as f32 * LINE_HEIGHT,
            ))
        } else {
            None
        }
    }

    /// Inverse of `cursor_pixel_pos`: converts a pixel-space `(x, y)`
    /// position -- relative to the buffer's own origin, same convention
    /// `cursor_pixel_pos` uses (callers subtract `TEXT_ORIGIN_X`/
    /// `TEXT_ORIGIN_Y` first) -- into a `(window-local-line,
    /// column-in-chars)` position, via cosmic-text's own real `Buffer::hit`
    /// hit-testing (confirmed against the actual installed
    /// `cosmic-text-0.10.0` source before writing this, not assumed from
    /// docs). `hit`'s returned `Cursor::index` is a *byte* offset within the
    /// line, while this crate's cursor positions are char-based (matching
    /// `cursor_pixel_pos`'s own `col_chars`), so it's converted by counting
    /// chars up to that byte offset in the line's own text. `None` if the
    /// click landed outside any laid-out line (e.g. below the last visible
    /// line, in the empty space past end-of-window).
    pub fn hit_test(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let cursor = self.buffer.hit(x, y)?;
        let line_text = self.buffer.lines.get(cursor.line)?.text();
        let col_chars = line_text
            .get(..cursor.index)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        Some((cursor.line, col_chars))
    }

    /// Real hit-testing for the tab bar (§75.21) -- the same real
    /// cosmic-text `Buffer::hit` technique `hit_test` uses for the main
    /// editor, applied to `tab_bar_buffer` instead. `x`/`y` are relative to
    /// the tab bar's own origin (`TEXT_ORIGIN_X`/`TAB_BAR_TEXT_TOP`).
    /// Returns a *char* index into the tab bar's single line of text,
    /// matching `tab_bar::build_tab_bar_text`'s own char-counted
    /// convention (that module resolves this index to a specific tab via
    /// `tab_bar::hit_test_tab_bar`) -- not a byte offset, and not a
    /// `(line, col)` pair, since the tab bar is always exactly one line.
    pub fn hit_test_tab_bar(&self, x: f32, y: f32) -> Option<usize> {
        let cursor = self.tab_bar_buffer.hit(x, y)?;
        let line_text = self.tab_bar_buffer.lines.first()?.text();
        let col_chars = line_text
            .get(..cursor.index)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        Some(col_chars)
    }

    /// Real hit-testing for the file tree sidebar (§75.26) -- simpler than
    /// `hit_test_tab_bar`: since the sidebar is genuinely multi-line (one
    /// real line per tree row, via `file_tree::build_tree_text`), the real
    /// cosmic-text `Buffer::hit`'s own `Cursor::line` *is* the row index
    /// directly, with no char-range list to resolve against. `x`/`y` are
    /// relative to the sidebar's own origin (`SIDEBAR_TEXT_LEFT`/
    /// `SIDEBAR_TEXT_TOP`). `None` if the click landed below the last row.
    pub fn hit_test_sidebar(&self, x: f32, y: f32) -> Option<usize> {
        self.sidebar_buffer.hit(x, y).map(|cursor| cursor.line)
    }

    /// Real pixel x-position for a char column within the tab bar's single
    /// line (§75.21) -- the tab-bar equivalent of `cursor_pixel_pos`,
    /// simplified since the tab bar is always exactly one line (no
    /// multi-line lookup, no ropey-phantom-line edge case to handle).
    /// `None` only if the tab bar has no laid-out content at all yet (an
    /// empty string, e.g. no files open).
    pub fn tab_bar_pixel_pos(&self, col_chars: usize) -> Option<f32> {
        let run = self.tab_bar_buffer.layout_runs().next()?;
        let byte_offset = run
            .text
            .char_indices()
            .nth(col_chars)
            .map(|(b, _)| b)
            .unwrap_or(run.text.len());
        let x = run
            .glyphs
            .iter()
            .find(|g| byte_offset >= g.start && byte_offset < g.end)
            .map(|g| g.x)
            .unwrap_or_else(|| run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0));
        Some(x)
    }

    /// The current tab bar horizontal scroll offset, in real pixels --
    /// callers (main.rs's rendering and hit-testing) both need to read
    /// this, so it can't stay purely internal the way `Viewport::
    /// scroll_line` mostly can.
    pub fn tab_bar_scroll(&self) -> f32 {
        self.tab_bar_scroll
    }

    /// Real tab bar horizontal auto-scroll (§75.28) -- the pixel analogue
    /// of `Viewport::ensure_visible`, scrolling minimally so the tab
    /// spanning `active_range` (a real char-column range, from `tab_bar::
    /// TabHit::tab_range`) is fully visible within a strip `visible_width`
    /// pixels wide. A no-op if it already is, or if either end of the
    /// range can't be resolved to a pixel position (e.g. no tabs at all).
    /// Clamped so scrolling can never reveal empty space past the last
    /// tab's own right edge, the same `max_scroll` clamp `Viewport::
    /// ensure_visible` applies in line-count space, computed here instead
    /// from the tab bar's own real laid-out width. Returns whether the
    /// scroll position actually changed.
    pub fn ensure_tab_visible(
        &mut self,
        active_range: std::ops::Range<usize>,
        visible_width: f32,
    ) -> bool {
        let Some(x_start) = self.tab_bar_pixel_pos(active_range.start) else {
            return false;
        };
        let Some(x_end) = self.tab_bar_pixel_pos(active_range.end) else {
            return false;
        };
        let new_scroll = if x_start < self.tab_bar_scroll {
            x_start
        } else if x_end > self.tab_bar_scroll + visible_width {
            x_end - visible_width
        } else {
            self.tab_bar_scroll
        };
        let total_width = self
            .tab_bar_buffer
            .layout_runs()
            .next()
            .and_then(|run| run.glyphs.last().map(|g| g.x + g.w))
            .unwrap_or(0.0);
        let max_scroll = (total_width - visible_width).max(0.0);
        let new_scroll = new_scroll.clamp(0.0, max_scroll);
        let changed = new_scroll != self.tab_bar_scroll;
        self.tab_bar_scroll = new_scroll;
        changed
    }
}
