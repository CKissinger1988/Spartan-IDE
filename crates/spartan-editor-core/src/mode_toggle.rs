//! Real Agent/Editor/Design mode toggle (§8, §16.1, task #3) -- pure
//! layout logic, no GPU dependency, mirroring `tab_bar.rs`/`file_tree.rs`/
//! `git_panel.rs`'s own "pure logic, test it headlessly" split.
//!
//! Only `Editor` mode has real content behind it in this crate -- there is
//! no Leo/`ModelProvider` (§3, §4) and no GUI Builder/WebView bridge (§6)
//! anywhere in this workspace yet. `Agent` and `Design` modes are real,
//! honestly-empty placeholder states, not simulated content: switching to
//! either one shows a real message explaining exactly what's missing and
//! why, the same "name the gap, don't fake it" discipline this crate's own
//! §75.x history has followed throughout, applied here to a whole mode
//! rather than one feature within a mode.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Agent,
    Editor,
    Design,
}

impl AppMode {
    pub const ALL: [AppMode; 3] = [AppMode::Agent, AppMode::Editor, AppMode::Design];

    pub fn label(self) -> &'static str {
        match self {
            AppMode::Agent => "Agent",
            AppMode::Editor => "Editor",
            AppMode::Design => "Design",
        }
    }

    /// The real, honest placeholder message shown in place of document
    /// content while this mode is active and has no real backing yet.
    /// `Editor` never needs one -- it's the one mode with real content.
    /// `Design` no longer needs one either as of §75.39/task #12's WebView
    /// bridge increment -- it now shows a real embedded WebView instead of
    /// text (still honestly not a live component canvas yet; see
    /// `webview_bridge.rs`'s own doc comment for exactly what it does
    /// show).
    pub fn placeholder_message(self) -> Option<&'static str> {
        match self {
            AppMode::Agent => Some(
                "Agent mode\n\n\
                 No ModelProvider is configured in this build (§3, §4).\n\
                 Leo requires a real Claude API key or a running Ollama\n\
                 instance -- neither is available here, so this mode has\n\
                 no real functionality to show yet, rather than a\n\
                 simulated chat interface.\n\n\
                 Press Ctrl+2 to return to Editor mode.",
            ),
            AppMode::Editor => None,
            AppMode::Design => None,
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

/// Builds the real "Agent | Editor | Design" display text plus each
/// label's real char range, in document order.
pub fn build_mode_toggle_text() -> (String, Vec<ModeHit>) {
    let mut text = String::new();
    let mut hits = Vec::with_capacity(AppMode::ALL.len());
    for (index, mode) in AppMode::ALL.iter().enumerate() {
        if index > 0 {
            text.push_str(" | ");
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
/// to the mode it landed on -- `None` for a click on the " | " separator
/// between two labels.
pub fn hit_test(hits: &[ModeHit], col_chars: usize) -> Option<AppMode> {
    hits.iter()
        .find(|hit| hit.range.contains(&col_chars))
        .map(|hit| hit.mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mode_toggle_text_produces_the_expected_real_string() {
        let (text, _) = build_mode_toggle_text();
        assert_eq!(text, "Agent | Editor | Design");
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
        assert_eq!(hit_test(&hits, 8), Some(AppMode::Editor));
        assert_eq!(hit_test(&hits, 17), Some(AppMode::Design));
    }

    #[test]
    fn hit_test_on_the_separator_between_labels_resolves_to_none() {
        let (_, hits) = build_mode_toggle_text();
        // "Agent | Editor | Design"
        //  0123456
        // index 6 is inside " | " between Agent (0..5) and Editor (8..14).
        assert_eq!(hit_test(&hits, 6), None);
    }

    #[test]
    fn only_agent_mode_has_a_placeholder_message() {
        // Editor always had real content; Design gained a real embedded
        // WebView in §75.39/task #12, so only Agent (no ModelProvider/Leo)
        // still needs a placeholder.
        assert!(AppMode::Editor.placeholder_message().is_none());
        assert!(AppMode::Design.placeholder_message().is_none());
        assert!(AppMode::Agent.placeholder_message().is_some());
    }
}
