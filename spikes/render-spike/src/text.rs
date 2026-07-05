use glyphon::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer,
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

    /// Replaces the buffer's content. Called on every document change for
    /// this first increment -- full-content reshaping every edit, not real
    /// damage-region tracking (see the exit report).
    pub fn set_text(&mut self, text: &str) {
        self.buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        self.buffer.shape_until_scroll(&mut self.font_system);
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
