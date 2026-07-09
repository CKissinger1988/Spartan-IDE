//! Real settings UI (§42, user-requested) -- first real increment: local-
//! model GPU offload configuration, reusing exactly the persistence
//! `spartan-settings` already provides. Pure, headlessly-tested display-
//! text/state logic only, mirroring `tab_bar.rs`/`agent_panel.rs`'s own
//! "no GPU dependency in this module" split -- keyboard wiring and the
//! real save-to-disk call live in `main.rs`.
//!
//! Deliberately small and honest: one settings screen, three rows,
//! keyboard-only (no mouse hit-testing yet, matching the unsaved-changes/
//! commit modals' own existing v1 scope), no live-reload of an in-flight
//! Leo request (a change only takes effect on the *next* plan request,
//! since `leo_bridge::spawn_plan_request` reads settings once per call,
//! not a subscription).
//!
//! The third row (§75.49, user-requested "automatic update features") is
//! a real, live "Check for Updates" action -- triggering it is `main.rs`'s
//! job (via `update_bridge::spawn_update_check`, off the render thread);
//! this module only owns the resulting *display* state, matching
//! `agent_panel.rs`'s own "pure display logic, real I/O lives in main.rs"
//! split.

use spartan_settings::Settings;
use spartan_updater::UpdateCheckResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    GpuOffloadEnabled,
    GpuOffloadLayers,
    CheckForUpdates,
}

impl SettingsRow {
    const ALL: [SettingsRow; 3] = [
        SettingsRow::GpuOffloadEnabled,
        SettingsRow::GpuOffloadLayers,
        SettingsRow::CheckForUpdates,
    ];

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|r| *r == self).unwrap();
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|r| *r == self).unwrap();
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// The real, live display state of the "Check for Updates" row -- `main.rs`
/// drives every transition (spawning the real background check on
/// `Checking`, applying its real result on `Ready`/`Failed`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum UpdateCheckDisplay {
    #[default]
    NotChecked,
    Checking,
    Ready(UpdateCheckResult),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsPanelState {
    pub settings: Settings,
    pub selected: SettingsRow,
    pub update_check: UpdateCheckDisplay,
}

impl SettingsPanelState {
    pub fn opened_with(settings: Settings) -> Self {
        Self {
            settings,
            selected: SettingsRow::GpuOffloadEnabled,
            update_check: UpdateCheckDisplay::NotChecked,
        }
    }

    pub fn move_selection_down(&mut self) {
        self.selected = self.selected.next();
    }

    pub fn move_selection_up(&mut self) {
        self.selected = self.selected.prev();
    }

    /// Real toggle -- only meaningful on the `GpuOffloadEnabled` row.
    pub fn toggle_selected(&mut self) {
        if self.selected == SettingsRow::GpuOffloadEnabled {
            self.settings.gpu_offload.enabled = !self.settings.gpu_offload.enabled;
        }
    }

    /// Real layer-count adjustment (`GpuOffloadLayers` row only) --
    /// cycles `None` ("Auto" -- Ollama decides) through `0..=MAX_LAYERS`
    /// and back, so both extremes (fully automatic, and an explicit `0`
    /// which the `enabled` toggle already covers more directly) stay
    /// reachable without a separate "clear" action.
    pub fn adjust_layers(&mut self, delta: i32) {
        const MAX_LAYERS: u32 = 128;
        if self.selected != SettingsRow::GpuOffloadLayers {
            return;
        }
        let current: i64 = match self.settings.gpu_offload.layers {
            None => -1,
            Some(n) => n as i64,
        };
        let next = current + delta as i64;
        self.settings.gpu_offload.layers = if (0..=MAX_LAYERS as i64).contains(&next) {
            Some(next as u32)
        } else {
            None
        };
    }
}

fn row_marker(state: &SettingsPanelState, row: SettingsRow) -> &'static str {
    if state.selected == row {
        ">"
    } else {
        " "
    }
}

/// The real, live text for the "Check for Updates" row's own current
/// state -- a real category breakdown on a real update, not just
/// "update available."
fn update_check_line(state: &UpdateCheckDisplay) -> String {
    match state {
        UpdateCheckDisplay::NotChecked => "Check for Updates (Space/Enter to check)".to_string(),
        UpdateCheckDisplay::Checking => "Checking for updates...".to_string(),
        UpdateCheckDisplay::Ready(result) if result.up_to_date => {
            format!("Up to date ({})", short_commit(&result.current_commit))
        }
        UpdateCheckDisplay::Ready(result) => {
            let mut parts = Vec::new();
            if result.categories.language_definitions_changed {
                parts.push("language definitions");
            }
            if result.categories.leo_changed {
                parts.push("Leo/agent core");
            }
            if result.categories.other_changed {
                parts.push("other IDE code");
            }
            let what = if parts.is_empty() {
                "changes".to_string()
            } else {
                parts.join(", ")
            };
            format!(
                "Update available: {what} ({} -> {})",
                short_commit(&result.current_commit),
                short_commit(&result.latest_commit)
            )
        }
        UpdateCheckDisplay::Failed(message) => format!("Update check failed: {message}"),
    }
}

fn short_commit(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// The real, live display text for the settings panel -- rebuilt every
/// frame from live state, matching every other real panel in this crate.
pub fn build_panel_text(state: &SettingsPanelState) -> String {
    let enabled_box = if state.settings.gpu_offload.enabled {
        "[x]"
    } else {
        "[ ]"
    };
    let layers_text = match state.settings.gpu_offload.layers {
        None => "Auto".to_string(),
        Some(n) => n.to_string(),
    };
    format!(
        "Settings (§42, user-requested)\n\n\
         {} {enabled_box} GPU offloading enabled (Space/Enter to toggle)\n\
         {}     GPU layers to offload: {layers_text} (Left/Right to adjust)\n\
         {} {}\n\n\
         Up/Down to move -- Escape to save and close.",
        row_marker(state, SettingsRow::GpuOffloadEnabled),
        row_marker(state, SettingsRow::GpuOffloadLayers),
        row_marker(state, SettingsRow::CheckForUpdates),
        update_check_line(&state.update_check),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use spartan_settings::GpuOffloadSettings;
    use spartan_updater::ChangeCategories;

    #[test]
    fn opens_with_the_real_given_settings_and_first_row_selected() {
        let settings = Settings {
            gpu_offload: GpuOffloadSettings {
                enabled: false,
                layers: Some(4),
            },
        };
        let state = SettingsPanelState::opened_with(settings);
        assert_eq!(state.selected, SettingsRow::GpuOffloadEnabled);
        assert_eq!(state.settings, settings);
    }

    #[test]
    fn selection_moves_down_and_wraps() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::GpuOffloadLayers);
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::CheckForUpdates);
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::GpuOffloadEnabled);
    }

    #[test]
    fn selection_moves_up_and_wraps() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.move_selection_up();
        assert_eq!(state.selected, SettingsRow::CheckForUpdates);
    }

    #[test]
    fn toggle_only_affects_the_enabled_row() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        assert!(state.settings.gpu_offload.enabled);
        state.toggle_selected();
        assert!(!state.settings.gpu_offload.enabled);

        state.move_selection_down();
        state.toggle_selected();
        assert!(
            !state.settings.gpu_offload.enabled,
            "toggling while the layers row is selected must not touch `enabled`"
        );
    }

    #[test]
    fn adjust_layers_only_affects_the_layers_row() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.adjust_layers(1);
        assert_eq!(
            state.settings.gpu_offload.layers, None,
            "adjusting while the enabled row is selected must not touch layers"
        );
    }

    #[test]
    fn adjust_layers_from_auto_increments_to_zero_then_up() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.move_selection_down();
        assert_eq!(state.settings.gpu_offload.layers, None);
        state.adjust_layers(1);
        assert_eq!(state.settings.gpu_offload.layers, Some(0));
        state.adjust_layers(1);
        assert_eq!(state.settings.gpu_offload.layers, Some(1));
    }

    #[test]
    fn adjust_layers_decrementing_from_zero_wraps_to_auto() {
        let mut state = SettingsPanelState::opened_with(Settings {
            gpu_offload: GpuOffloadSettings {
                enabled: true,
                layers: Some(0),
            },
        });
        state.move_selection_down();
        state.adjust_layers(-1);
        assert_eq!(state.settings.gpu_offload.layers, None);
    }

    #[test]
    fn adjust_layers_incrementing_past_the_real_max_wraps_to_auto() {
        let mut state = SettingsPanelState::opened_with(Settings {
            gpu_offload: GpuOffloadSettings {
                enabled: true,
                layers: Some(128),
            },
        });
        state.move_selection_down();
        state.adjust_layers(1);
        assert_eq!(state.settings.gpu_offload.layers, None);
    }

    #[test]
    fn panel_text_reflects_real_live_state() {
        let mut state = SettingsPanelState::opened_with(Settings {
            gpu_offload: GpuOffloadSettings {
                enabled: true,
                layers: Some(16),
            },
        });
        let text = build_panel_text(&state);
        assert!(text.contains("[x]"));
        assert!(text.contains("16"));

        state.settings.gpu_offload.enabled = false;
        state.settings.gpu_offload.layers = None;
        let text = build_panel_text(&state);
        assert!(text.contains("[ ]"));
        assert!(text.contains("Auto"));
    }

    fn sample_update_result(up_to_date: bool, categories: ChangeCategories) -> UpdateCheckResult {
        UpdateCheckResult {
            current_commit: "a".repeat(40),
            latest_commit: "b".repeat(40),
            up_to_date,
            categories,
        }
    }

    #[test]
    fn not_checked_shows_the_real_prompt() {
        let state = SettingsPanelState::opened_with(Settings::default());
        let text = build_panel_text(&state);
        assert!(text.contains("Check for Updates"));
    }

    #[test]
    fn checking_shows_a_real_in_progress_message() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.update_check = UpdateCheckDisplay::Checking;
        let text = build_panel_text(&state);
        assert!(text.contains("Checking for updates"));
    }

    #[test]
    fn up_to_date_result_shows_the_real_short_commit() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.update_check =
            UpdateCheckDisplay::Ready(sample_update_result(true, ChangeCategories::default()));
        let text = build_panel_text(&state);
        assert!(text.contains("Up to date"));
        assert!(text.contains(&"a".repeat(7)));
    }

    #[test]
    fn an_available_update_names_every_real_changed_category() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.update_check = UpdateCheckDisplay::Ready(sample_update_result(
            false,
            ChangeCategories {
                language_definitions_changed: true,
                leo_changed: true,
                other_changed: false,
            },
        ));
        let text = build_panel_text(&state);
        assert!(text.contains("Update available"));
        assert!(text.contains("language definitions"));
        assert!(text.contains("Leo/agent core"));
        assert!(!text.contains("other IDE code"));
    }

    #[test]
    fn a_failed_check_shows_the_real_error_message() {
        let mut state = SettingsPanelState::opened_with(Settings::default());
        state.update_check = UpdateCheckDisplay::Failed("network error: timed out".to_string());
        let text = build_panel_text(&state);
        assert!(text.contains("network error: timed out"));
    }
}
