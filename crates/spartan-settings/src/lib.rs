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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Settings {
    pub gpu_offload: GpuOffloadSettings,
    pub leo_approval_mode: LeoApprovalMode,
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
        };
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, settings);
    }

    #[test]
    fn default_leo_approval_mode_is_the_real_non_negotiable_manual_every_step() {
        assert_eq!(
            Settings::default().leo_approval_mode,
            LeoApprovalMode::ManualEveryStep
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
