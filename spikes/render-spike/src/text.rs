use glyphon::{
    Attrs, AttrsList, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer,
};

/// Owns everything glyphon/cosmic-text needs to shape and rasterize real
/// text onto the GPU atlas each frame. One `Buffer` for the whole visible
/// document for this increment -- no per-line virtualization yet (see the
/// exit report's "what this does not confirm" section).
pub struct TextState {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub atlas: TextAtlas,
    pub renderer: TextRenderer,
    pub buffer: Buffer,
}

pub const FONT_SIZE: f32 = 16.0;
pub const LINE_HEIGHT: f32 = 20.0;

/// Text origin within the window, in pixels -- kept as constants (rather than
/// re-typing `8.0` in both `prepare()`'s `TextArea` and the cursor-position
/// math in `main.rs`) so the two can never drift apart and mis-align the
/// caret against the glyphs it's supposed to sit next to.
pub const TEXT_ORIGIN_X: f32 = 8.0;
pub const TEXT_ORIGIN_Y: f32 = 8.0;

impl TextState {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: f32,
        height: f32,
    ) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let mut atlas = TextAtlas::new(device, queue, surface_format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        let mut buffer = Buffer::new(&mut font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        buffer.set_size(&mut font_system, width, height);

        Self {
            font_system,
            swash_cache,
            atlas,
            renderer,
            buffer,
        }
    }

    /// Replaces the buffer's entire content -- a full-document reshape.
    /// Still required for structural edits (a newline inserted or removed),
    /// since cosmic-text has no public API for cheap line insert/delete.
    /// For same-line edits, prefer `set_line_text` (the damage-region
    /// increment, Track A of this spike's deepening pass).
    pub fn set_text(&mut self, text: &str) {
        self.buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.buffer.shape_until_scroll(&mut self.font_system);
    }

    /// Real damage-region increment: replaces only the ONE line whose text
    /// changed, via `BufferLine::set_text` -- a public cosmic-text API
    /// (confirmed by reading cosmic-text 0.10.0's source, not assumed) that
    /// invalidates only that line's cached shape/layout, leaving every other
    /// line's already-computed shape/layout untouched. `shape_until_scroll`
    /// then re-shapes only lines whose cache was invalidated, skipping the
    /// rest.
    ///
    /// This fixes the CPU-shaping half of the full-reshape-on-every-edit
    /// shortcut named in the exit report -- it does NOT fix the GPU-upload
    /// half: `glyphon::TextRenderer::prepare()` still walks every visible
    /// line's `layout_runs()` and re-uploads its glyphs on every call,
    /// regardless of which lines changed, with no scoped/partial API to
    /// avoid that. See the exit report for the real, measured effect of
    /// this change (and why the <5ms target is still not expected to be
    /// met by this alone).
    ///
    /// Only valid for edits that don't change the document's line count --
    /// callers must route structural edits (newline inserted/removed)
    /// through `set_text` instead (see `EditEffect::Structural`). Callers
    /// must also check `line_i < self.line_count()` first (see that
    /// method's doc comment for why) -- an out-of-range `line_i` here is
    /// silently ignored rather than panicking, since this increment treats
    /// "fell outside what's known to be safe" as "do nothing, let the
    /// caller's fallback handle it" rather than a hard error.
    pub fn set_line_text(&mut self, line_i: usize, text: &str) {
        if let Some(line) = self.buffer.lines.get_mut(line_i) {
            line.set_text(text, AttrsList::new(Attrs::new().family(Family::Monospace)));
        }
        self.buffer.shape_until_scroll(&mut self.font_system);
    }

    /// Number of lines cosmic-text's `Buffer` currently knows about.
    ///
    /// A real bug this increment's own verification found: `Document`
    /// (ropey) and cosmic-text disagree about line counts on a document
    /// ending in "\n" (see `cursor_pixel_pos`'s doc comment for the same
    /// mismatch's cursor-rendering half). Concretely: right after a newline
    /// is inserted, ropey immediately considers the cursor to be on a new,
    /// real line -- but cosmic-text's `buffer.lines` isn't extended until
    /// the *next* full `set_text()` rebuild processes that content. If a
    /// same-line edit's `EditEffect::Line(line_i)` names a `line_i` that
    /// hasn't been created in `buffer.lines` yet, calling `set_line_text`
    /// directly silently drops the edit (`get_mut` returns `None`) --
    /// found by actually typing across a line boundary and watching a
    /// character vanish, not by inspection. Callers must check
    /// `line_i < line_count()` and fall back to `set_text` otherwise.
    pub fn line_count(&self) -> usize {
        self.buffer.lines.len()
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.buffer.set_size(&mut self.font_system, width, height);
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
            [TextArea {
                buffer: &self.buffer,
                left: TEXT_ORIGIN_X,
                top: TEXT_ORIGIN_Y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                },
                default_color: Color::rgb(0xE9, 0xE7, 0xE4),
            }],
            &mut self.swash_cache,
        )
    }

    pub fn render<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
    ) -> Result<(), glyphon::RenderError> {
        self.renderer.render(&self.atlas, pass)
    }

    /// Maps a (line, column-in-chars) position to a pixel-space `(x, y)` for
    /// cursor rendering, matching this buffer's actual shaped glyph layout --
    /// not a naive `column * average_char_width` guess, which would drift on
    /// any non-fixed-advance shaping. Returns `None` if `line` isn't part of
    /// the buffer's currently laid-out (visible) runs -- this increment has
    /// no scroll-to-cursor, so a cursor outside the visible viewport simply
    /// isn't drawn rather than computed incorrectly.
    ///
    /// `LayoutGlyph::start`/`end` are *byte* offsets within the line's text,
    /// while `col_chars` is a *char* count from `EditorView::cursor_line_col`
    /// -- the two only coincide for pure-ASCII lines, so the column is first
    /// converted to a byte offset via the run's own line text.
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

        // Real mismatch found by running this, not spotted by inspection:
        // `Document` (ropey) treats a file ending in "\n" as having one more,
        // empty line after that final newline than cosmic-text's `Buffer`
        // does -- cosmic-text never synthesizes a `BufferLine` past the last
        // line terminator, so that phantom trailing line has no layout run
        // at all. Rather than silently failing to draw a cursor whenever the
        // caret sits at true end-of-file, treat that specific case (one line
        // past the last laid-out line, at its start) as one row below the
        // last real line.
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
}
