//! Real settings UI (§42, user-requested) -- first real increment: local-
//! model GPU offload configuration, reusing exactly the persistence
//! `spartan-settings` already provides. Pure, headlessly-tested display-
//! text/state logic only, mirroring `tab_bar.rs`/`agent_panel.rs`'s own
//! "no GPU dependency in this module" split -- keyboard wiring and the
//! real save-to-disk call live in `main.rs`.
//!
//! Deliberately small and honest: one settings screen, five rows,
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
//!
//! The `Theme`/`FontFamily` rows (§75.93, user-requested "Add user
//! customizable theme and font options to all Spartan interfaces") are a
//! real, deliberately narrower increment than the Electron shell's own
//! live version: this renderer has no display/GPU available in the
//! environment this pass was built in to verify a *live* palette swap or
//! font reload, and every real color token this crate uses
//! (`theme::bg_linear()` and friends) is read once, at process startup,
//! from `theme::init_theme()` -- so a change here is real and persisted
//! but only takes visible effect the *next time the IDE is launched*,
//! matching this exact panel's own already-established "GPU offload/Leo
//! settings apply next request, not live" precedent rather than
//! inventing a new, inconsistent live-reload story for this one setting.
//! `FontFamily` accepts real typed text (`main.rs` routes character keys
//! to it while selected, the same real free-text pattern the commit-
//! message modal already established) rather than a fixed enum, matching
//! `spartan_settings::EditorSettings.font_family`'s own free-form-string
//! shape.

use spartan_settings::{Settings, ThemeName};
use spartan_updater::UpdateCheckResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    GpuOffloadEnabled,
    GpuOffloadLayers,
    Theme,
    FontFamily,
    CheckForUpdates,
}

impl SettingsRow {
    const ALL: [SettingsRow; 5] = [
        SettingsRow::GpuOffloadEnabled,
        SettingsRow::GpuOffloadLayers,
        SettingsRow::Theme,
        SettingsRow::FontFamily,
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
    /// Real, static renderer diagnostics (§75.50, user-requested) --
    /// captured once at startup from the real `wgpu::AdapterInfo`
    /// (`main.rs` formats it), not re-queried live since the adapter
    /// can't change mid-session. Read-only, not a selectable row.
    pub renderer_info: String,
}

impl SettingsPanelState {
    pub fn opened_with(settings: Settings, renderer_info: String) -> Self {
        Self {
            settings,
            selected: SettingsRow::GpuOffloadEnabled,
            update_check: UpdateCheckDisplay::NotChecked,
            renderer_info,
        }
    }

    pub fn move_selection_down(&mut self) {
        self.selected = self.selected.next();
    }

    pub fn move_selection_up(&mut self) {
        self.selected = self.selected.prev();
    }

    /// Real toggle -- `GpuOffloadEnabled` flips its bool; `Theme` cycles
    /// the same as Left/Right (§75.93) so Space/Enter works as an
    /// intuitive "change it" action here too, matching this row's own
    /// binary choice (unlike `GpuOffloadLayers`, which has no sensible
    /// single "toggle" and stays Left/Right-only).
    pub fn toggle_selected(&mut self) {
        match self.selected {
            SettingsRow::GpuOffloadEnabled => {
                self.settings.gpu_offload.enabled = !self.settings.gpu_offload.enabled;
            }
            SettingsRow::Theme => self.cycle_theme(),
            _ => {}
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

    /// Real theme toggle (`Theme` row only), extended from 2 to 7 real
    /// variants by the "make all GUI designs user changeable" pass.
    /// Left/Right/Space/Enter all call this same, single-direction cycle
    /// (this crate's own `main.rs` calls it identically for both arrow
    /// keys -- no delta is threaded through) -- a real, deliberate, minor
    /// UX simplification over a true bidirectional cycle, since a 7-stop
    /// forward-only wrap is still a small, fast cycle to reach every
    /// theme; a true Left-goes-back UX is real, separate, unstarted
    /// follow-up if wanted.
    pub fn cycle_theme(&mut self) {
        if self.selected != SettingsRow::Theme {
            return;
        }
        self.settings.appearance.theme = match self.settings.appearance.theme {
            ThemeName::SpartanDark => ThemeName::SpartanLight,
            ThemeName::SpartanLight => ThemeName::MinimalistZen,
            ThemeName::MinimalistZen => ThemeName::NeonAftergrid,
            ThemeName::NeonAftergrid => ThemeName::WarmPaper,
            ThemeName::WarmPaper => ThemeName::CommandDeck,
            ThemeName::CommandDeck => ThemeName::GlassNative,
            ThemeName::GlassNative => ThemeName::SpartanDark,
        };
    }

    /// Real §75.93 font-family typed input (`FontFamily` row only) --
    /// appends real, non-control text, the same free-text pattern the
    /// commit-message modal already established in `main.rs`. `None`
    /// (blank -- use the real bundled default) becomes `Some(String::
    /// new())` on the first keystroke, exactly like the commit modal's
    /// own `Option<String>` -> populated-`String` transition.
    pub fn push_font_family_text(&mut self, text: &str) {
        if self.selected != SettingsRow::FontFamily {
            return;
        }
        self.settings
            .editor
            .font_family
            .get_or_insert_with(String::new)
            .push_str(text);
    }

    /// Real §75.93 font-family backspace (`FontFamily` row only) --
    /// reverts to `None` once the field is emptied, rather than leaving
    /// a real, meaningless `Some("")` around (`EditorSettings.font_family
    /// == Some(String::new())` would otherwise be a distinct, pointless
    /// third state alongside `None` and a real name).
    pub fn backspace_font_family(&mut self) {
        if self.selected != SettingsRow::FontFamily {
            return;
        }
        if let Some(font_family) = self.settings.editor.font_family.as_mut() {
            font_family.pop();
            if font_family.is_empty() {
                self.settings.editor.font_family = None;
            }
        }
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

fn theme_text(theme: ThemeName) -> &'static str {
    match theme {
        ThemeName::SpartanDark => "Spartan Dark",
        ThemeName::SpartanLight => "Spartan Light",
        ThemeName::MinimalistZen => "Minimalist Zen",
        ThemeName::NeonAftergrid => "Neon Aftergrid",
        ThemeName::WarmPaper => "Warm Paper",
        ThemeName::CommandDeck => "Command Deck",
        ThemeName::GlassNative => "Glass Native",
    }
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
    let font_family_text = state
        .settings
        .editor
        .font_family
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(bundled JetBrains Mono)");
    format!(
        "Settings (§42, user-requested)\n\n\
         {} {enabled_box} GPU offloading enabled (Space/Enter to toggle)\n\
         {}     GPU layers to offload: {layers_text} (Left/Right to adjust)\n\
         {}     Theme: {} (Space/Enter/Left/Right to change -- applies next launch)\n\
         {}     Font family: {font_family_text} (type to edit, Backspace to delete -- applies \
         next launch)\n\
         {} {}\n\n\
         Renderer: {}\n\n\
         Up/Down to move -- Escape to save and close.",
        row_marker(state, SettingsRow::GpuOffloadEnabled),
        row_marker(state, SettingsRow::GpuOffloadLayers),
        row_marker(state, SettingsRow::Theme),
        theme_text(state.settings.appearance.theme),
        row_marker(state, SettingsRow::FontFamily),
        row_marker(state, SettingsRow::CheckForUpdates),
        update_check_line(&state.update_check),
        state.renderer_info,
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
            ..Default::default()
        };
        let state = SettingsPanelState::opened_with(settings.clone(), "test-renderer".to_string());
        assert_eq!(state.selected, SettingsRow::GpuOffloadEnabled);
        assert_eq!(state.settings, settings);
    }

    #[test]
    fn selection_moves_down_and_wraps() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::GpuOffloadLayers);
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::Theme);
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::FontFamily);
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::CheckForUpdates);
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::GpuOffloadEnabled);
    }

    #[test]
    fn selection_moves_up_and_wraps() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.move_selection_up();
        assert_eq!(state.selected, SettingsRow::CheckForUpdates);
    }

    #[test]
    fn toggle_only_affects_the_enabled_row() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
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
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.adjust_layers(1);
        assert_eq!(
            state.settings.gpu_offload.layers, None,
            "adjusting while the enabled row is selected must not touch layers"
        );
    }

    #[test]
    fn adjust_layers_from_auto_increments_to_zero_then_up() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.move_selection_down();
        assert_eq!(state.settings.gpu_offload.layers, None);
        state.adjust_layers(1);
        assert_eq!(state.settings.gpu_offload.layers, Some(0));
        state.adjust_layers(1);
        assert_eq!(state.settings.gpu_offload.layers, Some(1));
    }

    #[test]
    fn adjust_layers_decrementing_from_zero_wraps_to_auto() {
        let mut state = SettingsPanelState::opened_with(
            Settings {
                gpu_offload: GpuOffloadSettings {
                    enabled: true,
                    layers: Some(0),
                },
                ..Default::default()
            },
            "test-renderer".to_string(),
        );
        state.move_selection_down();
        state.adjust_layers(-1);
        assert_eq!(state.settings.gpu_offload.layers, None);
    }

    #[test]
    fn adjust_layers_incrementing_past_the_real_max_wraps_to_auto() {
        let mut state = SettingsPanelState::opened_with(
            Settings {
                gpu_offload: GpuOffloadSettings {
                    enabled: true,
                    layers: Some(128),
                },
                ..Default::default()
            },
            "test-renderer".to_string(),
        );
        state.move_selection_down();
        state.adjust_layers(1);
        assert_eq!(state.settings.gpu_offload.layers, None);
    }

    #[test]
    fn cycle_theme_only_affects_the_theme_row_and_visits_all_7_real_variants_before_wrapping() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        assert_eq!(state.settings.appearance.theme, ThemeName::SpartanDark);
        state.cycle_theme();
        assert_eq!(
            state.settings.appearance.theme,
            ThemeName::SpartanDark,
            "cycling while the enabled row is selected must not touch theme"
        );

        state.move_selection_down();
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::Theme);

        let expected = [
            ThemeName::SpartanLight,
            ThemeName::MinimalistZen,
            ThemeName::NeonAftergrid,
            ThemeName::WarmPaper,
            ThemeName::CommandDeck,
            ThemeName::GlassNative,
            ThemeName::SpartanDark,
        ];
        for want in expected {
            state.cycle_theme();
            assert_eq!(state.settings.appearance.theme, want);
        }
    }

    #[test]
    fn toggle_selected_on_the_theme_row_also_cycles_it() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.move_selection_down();
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::Theme);
        state.toggle_selected();
        assert_eq!(state.settings.appearance.theme, ThemeName::SpartanLight);
    }

    #[test]
    fn font_family_text_only_applies_to_the_font_family_row() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.push_font_family_text("Fira Code");
        assert_eq!(
            state.settings.editor.font_family, None,
            "typing while the enabled row is selected must not touch font_family"
        );

        state.move_selection_down();
        state.move_selection_down();
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::FontFamily);
        state.push_font_family_text("Fira");
        state.push_font_family_text(" Code");
        assert_eq!(
            state.settings.editor.font_family,
            Some("Fira Code".to_string())
        );
    }

    #[test]
    fn backspace_font_family_pops_a_char_and_reverts_to_none_when_emptied() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.move_selection_down();
        state.move_selection_down();
        state.move_selection_down();
        assert_eq!(state.selected, SettingsRow::FontFamily);
        state.push_font_family_text("Go");
        state.backspace_font_family();
        assert_eq!(state.settings.editor.font_family, Some("G".to_string()));
        state.backspace_font_family();
        assert_eq!(
            state.settings.editor.font_family, None,
            "emptying the field should revert to None, not a real, meaningless Some(\"\")"
        );
        // A no-op backspace on an already-`None` field must not panic or
        // produce `Some("")`.
        state.backspace_font_family();
        assert_eq!(state.settings.editor.font_family, None);
    }

    #[test]
    fn backspace_font_family_only_applies_to_the_font_family_row() {
        let mut state = SettingsPanelState::opened_with(
            Settings {
                editor: spartan_settings::EditorSettings {
                    font_family: Some("Menlo".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            "test-renderer".to_string(),
        );
        state.backspace_font_family();
        assert_eq!(
            state.settings.editor.font_family,
            Some("Menlo".to_string()),
            "backspacing while the enabled row is selected must not touch font_family"
        );
    }

    #[test]
    fn panel_text_shows_the_real_theme_and_font_family() {
        let mut state = SettingsPanelState::opened_with(
            Settings {
                appearance: spartan_settings::AppearanceSettings {
                    theme: ThemeName::SpartanLight,
                    reduce_motion: false,
                },
                editor: spartan_settings::EditorSettings {
                    font_family: Some("Fira Code".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            "test-renderer".to_string(),
        );
        let text = build_panel_text(&state);
        assert!(text.contains("Spartan Light"));
        assert!(text.contains("Fira Code"));

        state.settings.appearance.theme = ThemeName::SpartanDark;
        state.settings.editor.font_family = None;
        let text = build_panel_text(&state);
        assert!(text.contains("Spartan Dark"));
        assert!(text.contains("(bundled JetBrains Mono)"));
    }

    #[test]
    fn theme_text_gives_a_distinct_real_label_for_all_7_variants() {
        let all = [
            ThemeName::SpartanDark,
            ThemeName::SpartanLight,
            ThemeName::MinimalistZen,
            ThemeName::NeonAftergrid,
            ThemeName::WarmPaper,
            ThemeName::CommandDeck,
            ThemeName::GlassNative,
        ];
        let labels: Vec<&str> = all.iter().map(|t| theme_text(*t)).collect();
        for i in 0..labels.len() {
            for j in (i + 1)..labels.len() {
                assert_ne!(
                    labels[i], labels[j],
                    "every real theme label must be distinct"
                );
            }
        }
    }

    #[test]
    fn panel_text_reflects_real_live_state() {
        let mut state = SettingsPanelState::opened_with(
            Settings {
                gpu_offload: GpuOffloadSettings {
                    enabled: true,
                    layers: Some(16),
                },
                ..Default::default()
            },
            "test-renderer".to_string(),
        );
        let text = build_panel_text(&state);
        assert!(text.contains("[x]"));
        assert!(text.contains("16"));

        state.settings.gpu_offload.enabled = false;
        state.settings.gpu_offload.layers = None;
        let text = build_panel_text(&state);
        assert!(text.contains("[ ]"));
        assert!(text.contains("Auto"));
    }

    #[test]
    fn panel_text_shows_the_real_renderer_info() {
        let state = SettingsPanelState::opened_with(
            Settings::default(),
            "llvmpipe (Vulkan, software/virtual)".to_string(),
        );
        let text = build_panel_text(&state);
        assert!(text.contains("llvmpipe (Vulkan, software/virtual)"));
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
        let state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        let text = build_panel_text(&state);
        assert!(text.contains("Check for Updates"));
    }

    #[test]
    fn checking_shows_a_real_in_progress_message() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.update_check = UpdateCheckDisplay::Checking;
        let text = build_panel_text(&state);
        assert!(text.contains("Checking for updates"));
    }

    #[test]
    fn up_to_date_result_shows_the_real_short_commit() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.update_check =
            UpdateCheckDisplay::Ready(sample_update_result(true, ChangeCategories::default()));
        let text = build_panel_text(&state);
        assert!(text.contains("Up to date"));
        assert!(text.contains(&"a".repeat(7)));
    }

    #[test]
    fn an_available_update_names_every_real_changed_category() {
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
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
        let mut state =
            SettingsPanelState::opened_with(Settings::default(), "test-renderer".to_string());
        state.update_check = UpdateCheckDisplay::Failed("network error: timed out".to_string());
        let text = build_panel_text(&state);
        assert!(text.contains("network error: timed out"));
    }
}
