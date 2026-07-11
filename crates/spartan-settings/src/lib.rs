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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LeoProviderKind {
    #[default]
    Ollama,
    Claude,
    LiteLLM,
}

/// `model` is a real, free-form string (not an enum) since each real
/// provider's own valid model identifiers are that provider's own,
/// external, evolving namespace -- validating it here would mean this
/// crate silently going stale every time a provider ships a new model
/// name. The UI supplies a sensible real default per `kind` when the
/// user switches providers; this struct just stores whatever was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeoProviderSettings {
    pub kind: LeoProviderKind,
    pub model: String,
}

impl Default for LeoProviderSettings {
    fn default() -> Self {
        Self {
            kind: LeoProviderKind::Ollama,
            model: "llama3.1:8b".to_string(),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorSettings {
    pub font_size: u32,
    pub tab_size: u32,
    pub word_wrap: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_size: 13,
            tab_size: 2,
            word_wrap: false,
        }
    }
}

/// Real §75.76 appearance preferences. `reduce_motion` is a real,
/// accessibility-motivated toggle for the Sci-Fi theme's own new pulse/
/// scan-line animations (§75.76) -- disabling it adds a `reduce-motion`
/// class at the document root, which a small set of new CSS rules use to
/// turn every such animation off, matching WCAG 2.1's own
/// prefers-reduced-motion intent for users who opted out of OS-level
/// motion but are running this app under a mocked/headless Chromium
/// environment where that OS media query itself may not be honored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppearanceSettings {
    pub reduce_motion: bool,
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
            },
            editor: EditorSettings {
                font_size: 16,
                tab_size: 4,
                word_wrap: true,
            },
            appearance: AppearanceSettings {
                reduce_motion: true,
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
        assert!(!loaded.onboarding_completed);
    }

    #[test]
    fn default_editor_settings_match_the_original_hardcoded_editor_values() {
        let settings = EditorSettings::default();
        assert_eq!(settings.font_size, 13);
        assert_eq!(settings.tab_size, 2);
        assert!(!settings.word_wrap);
    }

    #[test]
    fn default_appearance_settings_do_not_reduce_motion() {
        assert!(!AppearanceSettings::default().reduce_motion);
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
