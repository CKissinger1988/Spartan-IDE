//! Real, minimal IDE settings persistence (§42, user-requested). First real
//! increment: local-model GPU offload configuration, the concrete feature
//! this pass was asked to expose a settings surface for. Deliberately
//! small and honest -- no settings UI framework, no per-workspace
//! overrides, no live-reload across running processes; just a real,
//! persisted `Settings` struct a caller loads once at startup and saves
//! when the user changes something, the same "no config system exists yet"
//! posture `spartan-editor-core`'s own `crash_dir()` already named before
//! this crate existed.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Real local-model GPU offload configuration, matching Ollama's own real
/// `options.num_gpu` request field (§57, §44) -- the number of model
/// layers to run on the GPU rather than the CPU. `enabled: false` forces
/// pure CPU inference (`num_gpu = 0`); `enabled: true` with `layers: None`
/// means "let Ollama decide" (no override sent at all, Ollama's own real
/// default auto-offload behavior); `enabled: true` with `layers: Some(n)`
/// forces exactly `n` layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuOffloadSettings {
    pub enabled: bool,
    pub layers: Option<u32>,
}

impl Default for GpuOffloadSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            layers: None,
        }
    }
}

impl GpuOffloadSettings {
    /// The real value to send as Ollama's `options.num_gpu` -- `None` means
    /// "send no override at all," not "send zero."
    pub fn num_gpu(self) -> Option<u32> {
        if !self.enabled {
            Some(0)
        } else {
            self.layers
        }
    }
}

/// Real §75.69 Leo approval-mode setting -- a real, user-facing mirror of
/// `spartan_leo::approval::ApprovalMode` (deliberately a local, self-
/// contained copy rather than a new `spartan-leo` dependency on this
/// otherwise-leaf settings crate; `spartan-backend` maps this to the real
/// enum at its one real call site). `ManualEveryStep` is the real,
/// non-negotiable default -- `AutoApproveSafe` only ever changes whether
/// a `Safe`-classified call (`read_file`/`search_files`/`list_directory`)
/// still needs a human click; a `Destructive` call (`edit_file`/
/// `run_terminal`) is never auto-approved by either setting, matching
/// §9's own non-negotiable rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LeoApprovalMode {
    #[default]
    ManualEveryStep,
    AutoApproveSafe,
}

/// Real §75.70 Leo model-provider setting -- closes the concrete "LLM
/// Agnostic: Seamlessly switch between local models... and remote APIs"
/// concept researched from `CKissinger1988/SpartanAI_Assistant` (the
/// user's own separate repo; concepts adapted into this project's real
/// `spartan-model` providers, no code ported -- see
/// `docs/architecture-spec.md` §75.70 for the full discussion). All three
/// real `ModelProvider` implementations (`OllamaProvider`,
/// `ClaudeProvider`, `LiteLLMProvider`) have existed in `spartan-model`
/// since §75.43/§75.57, but Leo itself had always hardcoded Ollama --
/// this is the first real setting choosing between them.
///
/// `LlamaCpp` (user-requested: "Integrate llama.cpp into the desktop
/// IDE") is a real fourth option -- unlike the other three, it's not an
/// HTTP client to a separate server process, it's real in-process GGUF
/// inference via `spartan_model::LlamaCppProvider`. See that module's own
/// doc comment for what's real (streamed text completion) and what's
/// explicitly out of scope this pass (native tool-calling through Leo's
/// existing plan/execute loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LeoProviderKind {
    #[default]
    Ollama,
    Claude,
    LiteLLM,
    LlamaCpp,
    /// LM Studio's local server (§57) -- OpenAI-compatible, runs the model
    /// in-process on this machine, so a real local runtime/privacy boundary.
    LmStudio,
}

/// `model` is a real, free-form string (not an enum) since each real
/// provider's own valid model identifiers are that provider's own,
/// external, evolving namespace -- validating it here would mean this
/// crate silently going stale every time a provider ships a new model
/// name. The UI supplies a sensible real default per `kind` when the
/// user switches providers; this struct just stores whatever was chosen.
///
/// For `LeoProviderKind::LlamaCpp`, this field holds a real local
/// filesystem path to a `.gguf` model file instead of a model-name string
/// -- the closest fit to this struct's existing "whatever the provider's
/// own real identifier namespace expects" contract, since llama.cpp's own
/// real identifier for a model *is* the path to its weights file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeoProviderSettings {
    pub kind: LeoProviderKind,
    pub model: String,
    /// Optional ordered fallback providers. When non-empty, Leo uses a
    /// `FailoverProvider` (see `spartan-model`): the primary (`kind`/`model`)
    /// is tried first, then each fallback in order, failing over only when a
    /// provider is unavailable *before* emitting output (a real 429/401/
    /// connection error), never mid-stream. A fallback's own `fallbacks` field
    /// is ignored (the chain is exactly primary + this list, not a tree) --
    /// `#[serde(default)]` so every pre-existing settings file still parses.
    #[serde(default)]
    pub fallbacks: Vec<LeoProviderSettings>,
}

impl Default for LeoProviderSettings {
    fn default() -> Self {
        Self {
            kind: LeoProviderKind::Ollama,
            model: "llama3.1:8b".to_string(),
            fallbacks: Vec::new(),
        }
    }
}

/// Real §75.76 editor preferences -- font size, tab size, word wrap.
/// Deliberately small: this is the same "thorough settings for every
/// feature" pass that added `AppearanceSettings`/`onboarding_completed`
/// below, not a general editor-config framework. `desktop/src/components/
/// Editor.tsx` reads this once per file open and applies it as real
/// inline style overrides on the highlight layer + textarea, matching
/// the wgpu shell's own "no live cross-window reload" precedent
/// (§75.48's own settings panel doc comment).
///
/// `font_family` (§75.93, user-requested: "Add user customizable theme
/// and font options to all Spartan interfaces") is deliberately
/// `Option<String>`, not a required field with a hardcoded default -- the
/// real default lives at each real call site (§75.92's bundled JetBrains
/// Mono: `crates/spartan-editor-core::fonts::DEFAULT_FONT_FAMILY`,
/// `desktop/`/`web/`'s own `--font-mono` CSS custom property), not
/// duplicated here as a string literal that could drift from the actual
/// bundled font name. `None` means "use the real bundled default";
/// `Some(name)` is a real, honest best-effort override -- neither this
/// crate nor its callers can guarantee `name` is actually installed/
/// resolvable on every platform, matching the same "fontconfig picks
/// whatever's actually available" reality §75.92 already documented for
/// the wgpu shell's own font-loading code.
///
/// Adding this field is why `EditorSettings` is no longer `Copy` --
/// `Option<String>` isn't `Copy` -- a real, mechanical ripple matching
/// the identical one `LeoProviderSettings`'s own `String` field already
/// caused for `Settings` itself; every call site the compiler flagged
/// was fixed the same way (an explicit `.clone()` or accepting the
/// already-correct partial-move behavior of `settings_set`'s own
/// `unwrap_or(current.editor)` pattern, which needs no change at all
/// since each field of `current` is only ever read once).
///
/// Real, load-bearing `#[serde(default)]` on this struct too, not just on
/// the outer `Settings` container -- found by actually running this
/// crate's own test suite after adding `font_family`, not by inspection:
/// `spartan-backend`'s `settings_set` dispatch deserializes an incoming
/// `"editor"` JSON object directly as a standalone `EditorSettings`
/// (`crates/spartan-backend/src/lib.rs`'s own `settings_set` arm), and the
/// container-level default on `Settings` only ever covers a *whole field
/// missing entirely* -- it does nothing for a present `"editor"` object
/// that's merely missing this one new sub-field, the exact real shape any
/// caller (a real pre-existing settings request, or this crate's own
/// pre-existing tests) built before `font_family` existed. Without this
/// attribute, that real shape would fail to parse at all -- not "silently
/// lose the new field," but reject the whole real request, dropping the
/// user's real font_size/tab_size/word_wrap choices along with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub font_size: u32,
    pub tab_size: u32,
    pub word_wrap: bool,
    pub font_family: Option<String>,
    /// Real "Format Document on Save" toggle (task #187) -- runs the same
    /// real `format_document` a manual Ctrl+Shift+F does, immediately
    /// before the real `save_file` write, every time Ctrl+S is pressed.
    /// Defaults `false`: an unconfigured formatter (no `black`/`rustfmt`/
    /// etc. installed, or a language the registry has none for at all,
    /// e.g. Java) must never turn every future save into a silent,
    /// surprising failure for a user who never opted in.
    pub format_on_save: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_size: 13,
            tab_size: 2,
            word_wrap: false,
            font_family: None,
            format_on_save: false,
        }
    }
}

/// Real §75.93 theme selection, extended to 7 real options by the
/// "make all GUI designs user changeable" pass. `SpartanDark` (the real,
/// only theme this project has ever shipped -- §50.3/§75.54/§75.55/§75.76's
/// own Antigravity-2.0-researched, Sci-Fi-accented palette) is the
/// non-negotiable default so every existing real user's already-running
/// app looks unchanged after upgrading. `SpartanLight` is the real, second
/// theme built by §75.93. The 5 newest variants (`MinimalistZen`,
/// `NeonAftergrid`, `WarmPaper`, `CommandDeck`, `GlassNative`) are the real,
/// standalone GUI design concepts first shipped as unwired HTML mockups
/// (matching this project's own `prototypes/*.jsx` convention) and then
/// wired live into both real Electron-based shells -- see
/// `desktop/src/theme.css`'s own `[data-theme="..."]` override blocks for
/// each one's real token values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeName {
    #[default]
    SpartanDark,
    SpartanLight,
    MinimalistZen,
    NeonAftergrid,
    WarmPaper,
    CommandDeck,
    GlassNative,
}

/// Real §75.76 appearance preferences. `reduce_motion` is a real,
/// accessibility-motivated toggle for the Sci-Fi theme's own new pulse/
/// scan-line animations (§75.76) -- disabling it adds a `reduce-motion`
/// class at the document root, which a small set of new CSS rules use to
/// turn every such animation off, matching WCAG 2.1's own
/// prefers-reduced-motion intent for users who opted out of OS-level
/// motion but are running this app under a mocked/headless Chromium
/// environment where that OS media query itself may not be honored.
///
/// `theme` (§75.93) is the real, user-facing theme choice -- see
/// `ThemeName`'s own doc comment. Struct-level `#[serde(default)]` for the
/// identical real reason `EditorSettings`'s own doc comment names --
/// `spartan-backend`'s `settings_set` deserializes an incoming
/// `"appearance"` object as a standalone `AppearanceSettings`, so a
/// present-but-`theme`-missing object (any real request built before this
/// field existed) must still parse rather than being rejected outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppearanceSettings {
    pub reduce_motion: bool,
    pub theme: ThemeName,
}

/// Real §75.82 crash-report upload configuration, closing task #35 --
/// `upload_endpoint` is deliberately `Option<String>`, defaulting to
/// `None` (never a hardcoded/well-known endpoint of this project's own):
/// there is no real telemetry backend this project operates, so "where do
/// reports go" has to be something a beta tester or self-hoster explicitly
/// types in themselves, not a default that could silently start sending
/// data anywhere. Pairing with `spartan_crash::upload_report` (§75.82),
/// which is likewise never called except from a real, explicit user
/// action -- this field being set does not, by itself, cause any upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrashReportingSettings {
    pub upload_endpoint: Option<String>,
}

/// `Settings` itself is no longer `Copy` (`LeoProviderSettings` owns a
/// real, non-`Copy` `String`) -- a real, mechanical ripple from adding
/// this field; every existing call site that relied on implicit copies
/// needed an explicit `.clone()`, found and fixed by the compiler, not
/// missed silently.
///
/// Real, load-bearing `#[serde(default)]` on the container -- found live
/// by a code-review pass, not by inspection: without it, any real
/// pre-existing `~/.spartan/settings.json` written by a build before
/// `editor`/`appearance`/`onboarding_completed` existed has no way to
/// satisfy a plain (non-defaulted) `Deserialize` for those fields, so
/// `serde_json::from_str` fails outright -- and `load_from`'s own
/// "a corrupt file is a recoverable, not fatal, state" `.unwrap_or_
/// default()` then silently discards the *entire* real file, wiping a
/// real user's already-chosen GPU offload/Leo provider/approval mode
/// and, worse, resetting `onboarding_completed` back to `false`,
/// re-triggering onboarding for an already-onboarded real user. This
/// attribute makes each individual missing field fall back to its own
/// type's default instead, the correct real schema-evolution behavior
/// this struct's own settings-file-is-forward-compatible design already
/// assumed but didn't actually implement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub gpu_offload: GpuOffloadSettings,
    pub leo_approval_mode: LeoApprovalMode,
    pub leo_provider: LeoProviderSettings,
    pub editor: EditorSettings,
    pub appearance: AppearanceSettings,
    pub crash_reporting: CrashReportingSettings,
    /// Real §75.76 first-run onboarding flag -- `false` until the user
    /// completes or explicitly skips the onboarding flow once; the
    /// Electron shell checks this on startup and shows onboarding only
    /// when it's still `false`.
    pub onboarding_completed: bool,
}

/// `~/.spartan/settings.json` (`$HOME`, falling back to `$USERPROFILE` for
/// Windows, falling back to the current directory) -- the same real
/// degrade-gracefully convention `spartan-editor-core`'s own `crash_dir()`
/// already established for this project's `.spartan/` dotfile namespace.
pub fn settings_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".spartan").join("settings.json")
}

/// Real load -- real, defaulted `Settings` (not an error) if the file
/// doesn't exist yet or fails to parse, matching "no settings saved yet"
/// and "a corrupt file" both as recoverable, not fatal, states.
pub fn load() -> Settings {
    load_from(&settings_path())
}

fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Real save -- creates `~/.spartan/` if it doesn't exist yet.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    save_to(&settings_path(), settings)
}

fn save_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spartan-settings-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn default_gpu_offload_is_enabled_with_no_explicit_layer_count() {
        let settings = GpuOffloadSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.layers, None);
        assert_eq!(settings.num_gpu(), None);
    }

    #[test]
    fn disabled_offload_forces_zero_gpu_layers() {
        let settings = GpuOffloadSettings {
            enabled: false,
            layers: Some(20),
        };
        assert_eq!(
            settings.num_gpu(),
            Some(0),
            "disabling GPU offload must force pure CPU inference regardless of a stale layer count"
        );
    }

    #[test]
    fn enabled_with_an_explicit_layer_count_passes_it_through() {
        let settings = GpuOffloadSettings {
            enabled: true,
            layers: Some(32),
        };
        assert_eq!(settings.num_gpu(), Some(32));
    }

    #[test]
    fn loading_a_missing_file_returns_real_defaults_not_an_error() {
        let path = temp_path("missing");
        let settings = load_from(&path);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn loading_a_corrupt_file_falls_back_to_real_defaults() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "not valid json{{{").unwrap();
        let settings = load_from(&path);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn save_then_load_round_trips_real_settings() {
        let path = temp_path("roundtrip");
        let settings = Settings {
            gpu_offload: GpuOffloadSettings {
                enabled: false,
                layers: Some(12),
            },
            leo_approval_mode: LeoApprovalMode::AutoApproveSafe,
            leo_provider: LeoProviderSettings {
                kind: LeoProviderKind::Claude,
                model: "claude-3-5-sonnet-latest".to_string(),
                ..Default::default()
            },
            editor: EditorSettings {
                font_size: 16,
                tab_size: 4,
                word_wrap: true,
                font_family: Some("Fira Code".to_string()),
                format_on_save: true,
            },
            appearance: AppearanceSettings {
                reduce_motion: true,
                theme: ThemeName::SpartanLight,
            },
            crash_reporting: CrashReportingSettings {
                upload_endpoint: Some("https://reports.example.com/upload".to_string()),
            },
            onboarding_completed: true,
        };
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, settings);
    }

    /// Real regression test for a real bug found live by code review, not
    /// by inspection: a genuine `~/.spartan/settings.json` written by any
    /// build before `editor`/`appearance`/`onboarding_completed` existed
    /// has exactly this shape -- missing all three fields. Confirms the
    /// container-level `#[serde(default)]` actually does its job: the
    /// real, already-chosen values are preserved, and only the genuinely
    /// missing fields fall back to their own defaults, rather than the
    /// whole file failing to parse and silently discarding everything
    /// (including a real user's `onboarding_completed: true`, which would
    /// have re-triggered onboarding for someone who'd already seen it).
    #[test]
    fn loading_a_real_pre_existing_file_missing_the_newer_fields_preserves_the_old_ones() {
        let path = temp_path("old-format-upgrade");
        std::fs::write(
            &path,
            r#"{
                "gpu_offload": { "enabled": false, "layers": 8 },
                "leo_approval_mode": "AutoApproveSafe",
                "leo_provider": { "kind": "Claude", "model": "claude-3-5-sonnet-latest" }
            }"#,
        )
        .unwrap();
        let loaded = load_from(&path);
        assert!(!loaded.gpu_offload.enabled);
        assert_eq!(loaded.gpu_offload.layers, Some(8));
        assert_eq!(loaded.leo_approval_mode, LeoApprovalMode::AutoApproveSafe);
        assert_eq!(loaded.leo_provider.kind, LeoProviderKind::Claude);
        // The genuinely missing fields fall back to real defaults --
        // never `onboarding_completed: false` wiping a real user's
        // already-set `true` (there was none to wipe here; that's the
        // whole point -- this file simply never had the field).
        assert_eq!(loaded.editor, EditorSettings::default());
        assert_eq!(loaded.appearance, AppearanceSettings::default());
        assert_eq!(loaded.crash_reporting, CrashReportingSettings::default());
        assert!(!loaded.onboarding_completed);
    }

    /// Real regression test matching the exact shape `spartan-backend`'s
    /// own `settings_set` dispatch actually deserializes: a standalone
    /// `EditorSettings` JSON object (not wrapped in a full `Settings`)
    /// missing the newer `font_family` field entirely -- the real shape
    /// of every `"editor": {...}` payload written before this field
    /// existed. Confirms the struct-level `#[serde(default)]` really
    /// does its job at this granularity, not just at the outer
    /// `Settings` container.
    #[test]
    fn deserializing_a_standalone_editor_object_missing_font_family_still_parses() {
        let value = serde_json::json!({ "font_size": 18, "tab_size": 4, "word_wrap": true });
        let settings: EditorSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.font_size, 18);
        assert_eq!(settings.tab_size, 4);
        assert!(settings.word_wrap);
        assert_eq!(settings.font_family, None);
    }

    /// Same real check for `EditorSettings`'s own newer `format_on_save`
    /// field (task #187) -- a real pre-existing `"editor": {...}` payload
    /// written before this field existed (even one that already had
    /// `font_family`) must still parse, falling back to the real, safe
    /// default of `false` rather than rejecting the whole request.
    #[test]
    fn deserializing_a_standalone_editor_object_missing_format_on_save_still_parses() {
        let value = serde_json::json!({
            "font_size": 18,
            "tab_size": 4,
            "word_wrap": true,
            "font_family": "Fira Code"
        });
        let settings: EditorSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings.font_size, 18);
        assert_eq!(settings.font_family, Some("Fira Code".to_string()));
        assert!(!settings.format_on_save);
    }

    /// Same real check for `AppearanceSettings`'s own new `theme` field.
    #[test]
    fn deserializing_a_standalone_appearance_object_missing_theme_still_parses() {
        let value = serde_json::json!({ "reduce_motion": true });
        let settings: AppearanceSettings = serde_json::from_value(value).unwrap();
        assert!(settings.reduce_motion);
        assert_eq!(settings.theme, ThemeName::SpartanDark);
    }

    #[test]
    fn default_crash_reporting_has_no_upload_endpoint_configured() {
        // No default/well-known endpoint of this project's own -- a real
        // upload target has to be something a user explicitly typed in.
        assert_eq!(Settings::default().crash_reporting.upload_endpoint, None);
    }

    #[test]
    fn default_editor_settings_match_the_original_hardcoded_editor_values() {
        let settings = EditorSettings::default();
        assert_eq!(settings.font_size, 13);
        assert_eq!(settings.tab_size, 2);
        assert!(!settings.word_wrap);
        assert_eq!(
            settings.font_family, None,
            "no font override by default -- the real bundled JetBrains Mono default lives at \
             each call site, not duplicated here as a string literal"
        );
        assert!(
            !settings.format_on_save,
            "an unconfigured formatter must never turn every future save into a silent failure"
        );
    }

    #[test]
    fn default_appearance_settings_do_not_reduce_motion() {
        assert!(!AppearanceSettings::default().reduce_motion);
    }

    #[test]
    fn default_theme_is_spartan_dark_so_every_existing_user_sees_no_change() {
        assert_eq!(AppearanceSettings::default().theme, ThemeName::SpartanDark);
        assert_eq!(Settings::default().appearance.theme, ThemeName::SpartanDark);
    }

    #[test]
    fn all_7_theme_variants_round_trip_through_json_and_are_pairwise_distinct() {
        let all = [
            ThemeName::SpartanDark,
            ThemeName::SpartanLight,
            ThemeName::MinimalistZen,
            ThemeName::NeonAftergrid,
            ThemeName::WarmPaper,
            ThemeName::CommandDeck,
            ThemeName::GlassNative,
        ];
        for theme in all {
            let json = serde_json::to_string(&theme).unwrap();
            let back: ThemeName = serde_json::from_str(&json).unwrap();
            assert_eq!(back, theme, "real round trip through JSON for {json}");
        }
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "every real theme variant must be distinct");
            }
        }
    }

    #[test]
    fn a_settings_json_naming_one_of_the_5_new_theme_variants_deserializes_correctly() {
        let settings: Settings =
            serde_json::from_str(r#"{"appearance": {"theme": "CommandDeck"}}"#).unwrap();
        assert_eq!(settings.appearance.theme, ThemeName::CommandDeck);
    }

    #[test]
    fn default_onboarding_completed_is_false_so_a_fresh_install_sees_onboarding() {
        assert!(!Settings::default().onboarding_completed);
    }

    #[test]
    fn default_leo_approval_mode_is_the_real_non_negotiable_manual_every_step() {
        assert_eq!(
            Settings::default().leo_approval_mode,
            LeoApprovalMode::ManualEveryStep
        );
    }

    #[test]
    fn default_leo_provider_is_local_ollama() {
        assert_eq!(
            Settings::default().leo_provider,
            LeoProviderSettings {
                kind: LeoProviderKind::Ollama,
                model: "llama3.1:8b".to_string(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn save_creates_the_real_parent_directory() {
        let dir = std::env::temp_dir().join("spartan-settings-test-mkdir-parent");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.json");
        save_to(&path, &Settings::default()).unwrap();
        assert!(path.exists());
    }
}
