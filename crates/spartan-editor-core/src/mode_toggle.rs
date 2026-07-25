//! Real Agent/Editor/Terminal/Workflow mode toggle (§8, §16.1, task #3)
//! -- pure layout logic, no GPU dependency, mirroring `tab_bar.rs`/
//! `file_tree.rs`/`git_panel.rs`'s own "pure logic, test it headlessly"
//! split.
//!
//! A fifth mode, `Design`, used to sit between `Editor` and `Terminal` and
//! hosted the GUI Builder's embedded WebView canvas. The GUI Builder was
//! removed from Spartan IDE at the user's explicit request; the mode went
//! with it, and every remaining mode here has real content behind it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Agent,
    Editor,
    /// Real integrated terminal (§75.56, user-requested: "I don't see...
    /// a terminal") -- the third real mode, joining Agent/Editor.
    Terminal,
    /// Real node-graph workflow builder (§75.57, user-requested) -- the
    /// fourth real mode: a real draggable node/edge canvas for orchestrating
    /// multiple real external CLI-tool sessions (`cli_session.rs`) as one
    /// workflow, with a session-detail trace view and run comparison.
    Workflow,
}

impl AppMode {
    pub const ALL: [AppMode; 4] = [
        AppMode::Agent,
        AppMode::Editor,
        AppMode::Terminal,
        AppMode::Workflow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppMode::Agent => "Agent",
            AppMode::Editor => "Editor",
            AppMode::Terminal => "Term",
            AppMode::Workflow => "Flow",
        }
    }

    /// The real, honest placeholder message shown in place of document
    /// content while this mode is active and has no real backing yet.
    /// `Editor` never needs one -- it's the one mode with real content.
    /// `Agent` no longer needs one either as of §75.47/task #5's
    /// real Leo UI wiring -- see `agent_panel.rs`'s own doc comment for
    /// exactly what it shows and doesn't.
    pub fn placeholder_message(self) -> Option<&'static str> {
        match self {
            AppMode::Agent => None,
            AppMode::Editor => None,
            AppMode::Terminal => None,
            AppMode::Workflow => None,
        }
    }
}

/// One mode label's real char range within the toggle's built text --
/// resolved by `hit_test` the same way `tab_bar::TabHit` resolves a tab
/// bar click.
pub struct ModeHit {
    pub mode: AppMode,
    pub range: std::ops::Range<usize>,
}

/// Builds the real "Agent|Editor|Term|Flow" display text plus each
/// label's real char range, in document order. A tight `|` separator (no
/// surrounding spaces) -- real room found tighter than expected once
/// `Terminal` was added, same fix `activity_bar.rs` already needed for its
/// own narrower row (§75.55); now five real labels wide.
pub fn build_mode_toggle_text() -> (String, Vec<ModeHit>) {
    let mut text = String::new();
    let mut hits = Vec::with_capacity(AppMode::ALL.len());
    for (index, mode) in AppMode::ALL.iter().enumerate() {
        if index > 0 {
            text.push('|');
        }
        let start = text.chars().count();
        text.push_str(mode.label());
        let end = text.chars().count();
        hits.push(ModeHit {
            mode: *mode,
            range: start..end,
        });
    }
    (text, hits)
}

/// Resolves a real click's char-column (from `TextState::hit_test_mode_toggle`)
/// to the mode it landed on -- `None` for a click on the `|` separator
/// between two labels. A trailing click past the last label clamps to it
/// (same real fix `activity_bar::hit_test` needed, §75.55, for the same
/// underlying cosmic-text `hit()` end-of-buffer clamping behavior).
pub fn hit_test(hits: &[ModeHit], col_chars: usize) -> Option<AppMode> {
    if let Some(hit) = hits.iter().find(|hit| hit.range.contains(&col_chars)) {
        return Some(hit.mode);
    }
    hits.last()
        .filter(|hit| col_chars >= hit.range.end)
        .map(|hit| hit.mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mode_toggle_text_produces_the_expected_real_string() {
        let (text, _) = build_mode_toggle_text();
        assert_eq!(text, "Agent|Editor|Term|Flow");
    }

    #[test]
    fn each_label_s_real_range_resolves_back_to_the_right_mode() {
        let (text, hits) = build_mode_toggle_text();
        for hit in &hits {
            let slice: String = text
                .chars()
                .skip(hit.range.start)
                .take(hit.range.len())
                .collect();
            assert_eq!(slice, hit.mode.label());
        }
    }

    #[test]
    fn hit_test_resolves_a_click_inside_each_real_label() {
        let (_, hits) = build_mode_toggle_text();
        assert_eq!(hit_test(&hits, 0), Some(AppMode::Agent));
        assert_eq!(hit_test(&hits, 6), Some(AppMode::Editor));
        assert_eq!(hit_test(&hits, 13), Some(AppMode::Terminal));
        assert_eq!(hit_test(&hits, 19), Some(AppMode::Workflow));
    }

    #[test]
    fn hit_test_on_the_separator_between_labels_resolves_to_none() {
        let (_, hits) = build_mode_toggle_text();
        // "Agent|Editor|Term|Flow"
        //  0123456789...
        // index 5 is the '|' separator between Agent (0..5) and Editor (6..12).
        assert_eq!(hit_test(&hits, 5), None);
    }

    #[test]
    fn hit_test_clamps_a_trailing_click_to_the_last_label() {
        let (text, hits) = build_mode_toggle_text();
        let past_end = text.chars().count();
        assert_eq!(hit_test(&hits, past_end), Some(AppMode::Workflow));
        assert_eq!(hit_test(&hits, past_end + 5), Some(AppMode::Workflow));
    }

    #[test]
    fn no_mode_has_a_placeholder_message_anymore() {
        // Editor always had real content; Agent gained real Leo UI wiring
        // in §75.47/task #5; Terminal gained a real PTY in §75.56;
        // Workflow gained a real node-graph canvas in §75.57 -- every mode
        // now shows real content.
        assert!(AppMode::Editor.placeholder_message().is_none());
        assert!(AppMode::Agent.placeholder_message().is_none());
        assert!(AppMode::Terminal.placeholder_message().is_none());
        assert!(AppMode::Workflow.placeholder_message().is_none());
    }
}
