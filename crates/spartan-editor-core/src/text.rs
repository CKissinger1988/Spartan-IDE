use glyphon::{
    Attrs, AttrsList, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer,
};

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

        Self {
            font_system,
            swash_cache,
            atlas,
            renderer,
            buffer,
        }
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
}
