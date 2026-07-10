/// Real activity-bar icon row (§75.56, user-requested "more tools/features/
/// options" pass) -- four short labels giving a real, clickable, on-screen
/// affordance for actions that previously existed only as keybindings
/// (Ctrl+G for Explorer/Git, Ctrl+1 for Agent mode, Ctrl+, for Settings),
/// mirroring `mode_toggle::AppMode`'s own enum/build/hit-test shape exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityId {
    Explorer,
    SourceControl,
    Agent,
    Settings,
}

impl ActivityId {
    pub const ALL: [ActivityId; 4] = [
        ActivityId::Explorer,
        ActivityId::SourceControl,
        ActivityId::Agent,
        ActivityId::Settings,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ActivityId::Explorer => "Files",
            ActivityId::SourceControl => "Git",
            ActivityId::Agent => "Agent",
            ActivityId::Settings => "Set",
        }
    }
}

/// One icon's real char range within the row's built text -- same shape as
/// `mode_toggle::ModeHit`.
pub struct ActivityHit {
    pub id: ActivityId,
    pub range: std::ops::Range<usize>,
}

/// Builds the real "Files|Git|Agent|Set" display text plus each label's
/// real char range, in document order -- same shape as
/// `mode_toggle::build_mode_toggle_text`, but a tighter, space-free `|`
/// separator (`mode_toggle`'s own `" | "` doesn't fit here): a real bug
/// found only by looking at the actual live screenshot, not by inspection
/// -- unlike the mode toggle's own text (anchored flush with its own
/// unbounded-right `TextBounds`), this row's `TextBounds` clips at exactly
/// `SIDEBAR_WIDTH` while its own text additionally starts `SIDEBAR_TEXT_LEFT`
/// pixels inset *within* that same bound, leaving noticeably less real
/// horizontal budget than the mode toggle has -- `" | "`'s three real
/// characters per separator was enough to clip the last label's tail.
pub fn build_activity_bar_text() -> (String, Vec<ActivityHit>) {
    let mut text = String::new();
    let mut hits = Vec::with_capacity(ActivityId::ALL.len());
    for (index, id) in ActivityId::ALL.iter().enumerate() {
        if index > 0 {
            text.push('|');
        }
        let start = text.chars().count();
        text.push_str(id.label());
        let end = text.chars().count();
        hits.push(ActivityHit {
            id: *id,
            range: start..end,
        });
    }
    (text, hits)
}

/// Resolves a real click's char-column (from
/// `TextState::hit_test_activity_bar`) to the icon it landed in, if any --
/// same shape as `mode_toggle::hit_test`, plus one real fix found only by
/// live-clicking this row, not by inspection: cosmic-text's own `hit()`
/// clamps a click past the last real glyph to the buffer's own end column
/// (`text.chars().count()`), which sits exactly one past the last label's
/// own exclusive `range.end` -- so a click on or just past the last
/// label's trailing pixels silently hit nothing at all. Clamping any
/// out-of-range trailing column to the last real hit fixes exactly that
/// case without changing any other label's own range semantics.
pub fn hit_test(hits: &[ActivityHit], col_chars: usize) -> Option<ActivityId> {
    if let Some(hit) = hits.iter().find(|hit| hit.range.contains(&col_chars)) {
        return Some(hit.id);
    }
    hits.last()
        .filter(|hit| col_chars >= hit.range.end)
        .map(|hit| hit.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_activity_bar_text_produces_the_expected_real_string() {
        let (text, _) = build_activity_bar_text();
        assert_eq!(text, "Files|Git|Agent|Set");
    }

    #[test]
    fn hit_test_resolves_each_real_label_range() {
        let (_, hits) = build_activity_bar_text();
        assert_eq!(hit_test(&hits, 0), Some(ActivityId::Explorer));
        let git_start = "Files|".chars().count();
        assert_eq!(hit_test(&hits, git_start), Some(ActivityId::SourceControl));
        let agent_start = "Files|Git|".chars().count();
        assert_eq!(hit_test(&hits, agent_start), Some(ActivityId::Agent));
        let set_start = "Files|Git|Agent|".chars().count();
        assert_eq!(hit_test(&hits, set_start), Some(ActivityId::Settings));
    }

    #[test]
    fn hit_test_misses_the_real_separators() {
        let (_, hits) = build_activity_bar_text();
        let sep_col = "Files".chars().count();
        assert_eq!(hit_test(&hits, sep_col), None);
    }

    #[test]
    fn hit_test_clamps_a_trailing_click_to_the_last_label() {
        let (text, hits) = build_activity_bar_text();
        let past_end = text.chars().count();
        assert_eq!(hit_test(&hits, past_end), Some(ActivityId::Settings));
        assert_eq!(hit_test(&hits, past_end + 5), Some(ActivityId::Settings));
    }
}
