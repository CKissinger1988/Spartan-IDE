use spartan_buffer::Document;

/// The real, new piece of engineering this crate adds beyond what any spike
/// built: only ever feeding the renderer the *visible slice* of the
/// document, never the whole file. This is the literal reading of §2.2's
/// damage-region requirement ("only re-rasterize the visible viewport... "),
/// which `spikes/render-spike` never implemented -- its own exit report
/// (§47.9-§47.10) names cold-open as untouched specifically because its
/// `TextState` always received the *entire* document via
/// `Buffer::set_text`, regardless of window size.
///
/// `scroll_line` and `visible_lines` are both in document-absolute line
/// units. This struct owns no rendering state itself -- it's the pure,
/// headlessly-testable slicing logic; `main.rs` wires it to `TextState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub scroll_line: usize,
    pub visible_lines: usize,
}

impl Viewport {
    pub fn new(visible_lines: usize) -> Self {
        Self {
            scroll_line: 0,
            visible_lines,
        }
    }

    /// The document-absolute line range this viewport currently shows,
    /// clamped to the document's real line count -- callers must not assume
    /// the range is always exactly `visible_lines` long (it's shorter once
    /// scrolled near the end of a document).
    pub fn visible_range(&self, doc_len_lines: usize) -> std::ops::Range<usize> {
        let start = self.scroll_line.min(doc_len_lines.saturating_sub(1));
        let end = (start + self.visible_lines).min(doc_len_lines);
        start..end
    }

    /// Scrolls by `delta` lines (negative scrolls up), clamped so
    /// `scroll_line` never exceeds the document's last valid line index.
    /// Returns whether the scroll position actually changed -- callers use
    /// this to skip a reshape when scrolling is already clamped at an edge
    /// (e.g. repeated PageDown at end-of-file).
    pub fn scroll_by(&mut self, delta: isize, doc_len_lines: usize) -> bool {
        let max_scroll = doc_len_lines.saturating_sub(1);
        let new_scroll = (self.scroll_line as isize + delta).clamp(0, max_scroll as isize) as usize;
        let changed = new_scroll != self.scroll_line;
        self.scroll_line = new_scroll;
        changed
    }

    /// True if `doc_line` (a document-absolute line index) currently falls
    /// within this viewport's visible range.
    pub fn contains_line(&self, doc_line: usize, doc_len_lines: usize) -> bool {
        self.visible_range(doc_len_lines).contains(&doc_line)
    }

    /// Scrolls minimally so `doc_line` becomes visible -- a no-op if it
    /// already is. If `doc_line` is above the current window, it becomes
    /// the new first visible line; if below, it becomes the new last
    /// visible line. This is the fix for a named, real gap from §75.5: with
    /// no auto-follow, typing enough newlines near the bottom edge of the
    /// window used to move the cursor off-screen with the viewport never
    /// following it. Returns whether the scroll position actually changed,
    /// matching `scroll_by`'s convention, so callers know whether a reshape
    /// is needed.
    pub fn ensure_visible(&mut self, doc_line: usize, doc_len_lines: usize) -> bool {
        if self.contains_line(doc_line, doc_len_lines) {
            return false;
        }
        let new_scroll = if doc_line < self.scroll_line {
            doc_line
        } else {
            (doc_line + 1).saturating_sub(self.visible_lines)
        };
        let max_scroll = doc_len_lines.saturating_sub(1);
        let new_scroll = new_scroll.min(max_scroll);
        let changed = new_scroll != self.scroll_line;
        self.scroll_line = new_scroll;
        changed
    }
}

/// Extracts only the currently-visible slice of the document as a single
/// string, suitable for feeding directly to `TextState::set_text` -- this
/// is what makes cosmic-text's `Buffer` only ever see ~40-60 lines instead
/// of up to 50,000, regardless of document size. `Document::line()` already
/// includes each line's own trailing terminator (ropey convention), so
/// simple concatenation reconstructs the exact windowed text verbatim.
pub fn windowed_text(document: &Document, viewport: &Viewport) -> String {
    let range = viewport.visible_range(document.len_lines());
    let mut out = String::new();
    for i in range {
        if let Ok(line) = document.line(i) {
            out.push_str(&line);
        }
    }
    out
}

/// Translates a document-absolute line index into the index it would have
/// *within the currently-windowed text* (i.e. relative to `scroll_line`),
/// or `None` if that line isn't currently visible at all -- meaning the
/// edit happened off-screen and needs no redraw whatsoever, a real
/// virtualization win `render-spike` (which always rendered the whole
/// document) never had the opportunity to take.
pub fn to_local_line(doc_line: usize, viewport: &Viewport, doc_len_lines: usize) -> Option<usize> {
    if viewport.contains_line(doc_line, doc_len_lines) {
        Some(doc_line - viewport.scroll_line)
    } else {
        None
    }
}

/// Inverse of `to_local_line`: translates a window-local line index (e.g.
/// from `TextState::hit_test`, which only ever sees the windowed slice) back
/// into a document-absolute one. Always valid -- unlike `to_local_line`,
/// there's no "off-screen" case to report, since a `local_line` by
/// definition came from something that was actually laid out on screen.
pub fn to_doc_line(local_line: usize, viewport: &Viewport) -> usize {
    viewport.scroll_line + local_line
}
