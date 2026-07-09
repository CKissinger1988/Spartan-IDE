//! Real `plugin.toml` capability-manifest parsing (§5.2).

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    #[serde(default)]
    pub filesystem: Vec<String>,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub editor_api: Vec<String>,
}

impl PluginManifest {
    pub fn parse(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text)?)
    }

    /// Real capability check the host consults *before* binding a host
    /// import into the `Linker` -- not a runtime permission check inside
    /// the function body, which the plugin could never actually observe
    /// being skipped either way. See `PluginHost::load_and_activate`.
    pub fn has_editor_capability(&self, cap: &str) -> bool {
        self.capabilities.editor_api.iter().any(|c| c == cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_real_manifest_with_full_capabilities() {
        let toml_str = r#"
            [plugin]
            name = "spartan-eslint-bridge"
            version = "0.1.0"

            [capabilities]
            filesystem = ["read"]
            network = false
            editor_api = ["log", "diagnostics.publish"]
        "#;
        let manifest = PluginManifest::parse(toml_str).unwrap();
        assert_eq!(manifest.plugin.name, "spartan-eslint-bridge");
        assert_eq!(manifest.capabilities.filesystem, vec!["read"]);
        assert!(!manifest.capabilities.network);
        assert!(manifest.has_editor_capability("log"));
        assert!(manifest.has_editor_capability("diagnostics.publish"));
        assert!(!manifest.has_editor_capability("commands.register"));
    }

    #[test]
    fn a_manifest_with_no_capabilities_section_defaults_to_nothing_granted() {
        let toml_str = r#"
            [plugin]
            name = "spartan-theme-pack"
            version = "0.1.0"
        "#;
        let manifest = PluginManifest::parse(toml_str).unwrap();
        assert_eq!(manifest.capabilities, Capabilities::default());
        assert!(!manifest.has_editor_capability("log"));
    }

    #[test]
    fn a_malformed_manifest_returns_a_parse_error_not_a_panic() {
        let toml_str = "not valid toml {{{";
        assert!(PluginManifest::parse(toml_str).is_err());
    }
}
