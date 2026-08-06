//! Real Linux packaging pipeline (task #14). Deliberately scoped to
//! Linux only -- the one platform this environment can actually build
//! and, critically, *run* the resulting package on to verify it, rather
//! than generating Windows `.msi`/macOS `.dmg` packaging this environment
//! has no way to build or test and could only ever produce unverified.
//! Real XDG conventions (`~/.local/bin`, `~/.local/share/applications`,
//! a `.desktop` entry) -- the standard, no-root-required way a Linux
//! desktop app installs for a single user, matching how this environment
//! itself has no privilege to install system-wide.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const BIN_NAME: &str = "spartan-editor-core";
pub const APP_NAME: &str = "Spartan IDE";
pub const APP_ID: &str = "com.spartan-ide.SpartanIDE";

/// Real, pure staging-directory naming -- testable without touching disk.
pub fn staging_dir_name(version: &str, target_triple: &str) -> String {
    format!("spartan-ide-{version}-{target_triple}")
}

/// Real XDG desktop entry (freedesktop.org Desktop Entry Specification).
/// `Icon` deliberately falls back to a real, standard icon-theme name
/// (`text-editor`) rather than a fabricated bespoke icon this project has
/// no real asset for yet -- a named, honest placeholder, not an invented
/// piece of branding.
pub fn desktop_entry_contents(exec_path: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={APP_NAME}\n\
         Comment=From-scratch, agent-first desktop IDE\n\
         Exec={exec_path} %F\n\
         Icon=text-editor\n\
         Terminal=false\n\
         Categories=Development;IDE;TextEditor;\n\
         MimeType=text/plain;inode/directory;\n"
    )
}

/// Real, minimal end-user-facing package README -- deliberately not
/// `CLAUDE.md` (a development-process document with no place in an
/// end-user package) or `docs/architecture-spec.md` (an internal design
/// document, not user documentation).
pub fn package_readme_contents(version: &str) -> String {
    format!(
        "{APP_NAME} {version}\n\
         =====================\n\n\
         From-scratch, agent-first desktop IDE.\n\n\
         Install (no root required):\n\
         \u{20}   ./install.sh\n\n\
         This installs the {BIN_NAME} binary to ~/.local/bin and a desktop\n\
         launcher entry to ~/.local/share/applications. Make sure ~/.local/bin\n\
         is on your PATH.\n\n\
         Run directly without installing:\n\
         \u{20}   ./{BIN_NAME} <file-or-project-path>\n"
    )
}

/// Real install script -- copies the binary and desktop entry into the
/// real per-user XDG locations. `update-desktop-database` is invoked only
/// if present on `$PATH`; its absence is not an error, matching how a
/// minimal container/CI environment (like the one that built this
/// package) may not have it, without blocking a real desktop user who
/// does.
pub fn install_sh_contents() -> String {
    format!(
        "#!/bin/sh\n\
         set -eu\n\
         umask 077\n\
         SCRIPT_DIR=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\n\
         BIN_DIR=\"$HOME/.local/bin\"\n\
         APPS_DIR=\"$HOME/.local/share/applications\"\n\
         LOG_DIR=\"${{XDG_STATE_HOME:-$HOME/.local/state}}/spartan\"\n\
         LOG_FILE=\"$LOG_DIR/install-errors.log\"\n\
         trap 'status=$?; if [ \"$status\" -ne 0 ]; then mkdir -p \"$LOG_DIR\"; printf \"%s install failed (exit %s)\\n\" \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\" \"$status\" >> \"$LOG_FILE\"; printf \"Spartan installation failed; diagnostic recorded at %s\\n\" \"$LOG_FILE\" >&2; fi' 0\n\
         mkdir -p \"$BIN_DIR\" \"$APPS_DIR\"\n\
         TMP_BIN=\"$BIN_DIR/.{BIN_NAME}.new.$$\"\n\
         cp \"$SCRIPT_DIR/{BIN_NAME}\" \"$TMP_BIN\"\n\
         chmod +x \"$TMP_BIN\"\n\
         mv -f \"$TMP_BIN\" \"$BIN_DIR/{BIN_NAME}\"\n\
         chmod +x \"$BIN_DIR/{BIN_NAME}\"\n\
         sed \"s|Exec={BIN_NAME}|Exec=$BIN_DIR/{BIN_NAME}|\" \\\n\
         \u{20}   \"$SCRIPT_DIR/{APP_ID}.desktop\" > \"$APPS_DIR/{APP_ID}.desktop\"\n\
         if command -v update-desktop-database >/dev/null 2>&1; then\n\
         \u{20}   update-desktop-database \"$APPS_DIR\" || true\n\
         fi\n\
         echo \"Installed {APP_NAME} to $BIN_DIR/{BIN_NAME}\"\n\
         echo \"Make sure $BIN_DIR is on your PATH.\"\n"
    )
}

/// Real `cargo build --release -p spartan-editor-core`, returning the
/// real resulting binary path. Does not skip the build if a stale binary
/// already exists -- a package should always reflect exactly what's in
/// the working tree right now, not whatever happened to be built last.
pub fn build_release_binary(workspace_root: &Path) -> Result<PathBuf> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "spartan-editor-core"])
        .current_dir(workspace_root)
        .status()
        .context("failed to spawn cargo build")?;
    if !status.success() {
        bail!("cargo build --release -p spartan-editor-core failed");
    }
    let bin_path = workspace_root.join("target").join("release").join(BIN_NAME);
    if !bin_path.exists() {
        bail!(
            "expected a real release binary at {}, but it doesn't exist",
            bin_path.display()
        );
    }
    Ok(bin_path)
}

/// Assembles a real staging directory and a real `.tar.gz` archive of it.
/// Returns the path to the real archive on success.
pub fn package(
    workspace_root: &Path,
    dist_dir: &Path,
    version: &str,
    target_triple: &str,
) -> Result<PathBuf> {
    let binary_path = build_release_binary(workspace_root)?;

    std::fs::create_dir_all(dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;
    let staging_name = staging_dir_name(version, target_triple);
    let staging_dir = dist_dir.join(&staging_name);
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir)
            .with_context(|| format!("failed to clean {}", staging_dir.display()))?;
    }
    std::fs::create_dir_all(&staging_dir)?;

    std::fs::copy(&binary_path, staging_dir.join(BIN_NAME))
        .with_context(|| format!("failed to copy {}", binary_path.display()))?;

    let desktop_path = staging_dir.join(format!("{APP_ID}.desktop"));
    std::fs::write(&desktop_path, desktop_entry_contents(BIN_NAME))?;

    let readme_path = staging_dir.join("README.txt");
    std::fs::write(&readme_path, package_readme_contents(version))?;

    let install_path = staging_dir.join("install.sh");
    std::fs::write(&install_path, install_sh_contents())?;
    make_executable(&install_path)?;
    make_executable(&staging_dir.join(BIN_NAME))?;

    let archive_name = format!("{staging_name}.tar.gz");
    let archive_path = dist_dir.join(&archive_name);
    let status = Command::new("tar")
        .args(["czf", &archive_name, &staging_name])
        .current_dir(dist_dir)
        .status()
        .context("failed to spawn tar")?;
    if !status.success() {
        bail!("tar czf failed");
    }

    Ok(archive_path)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_dir_name_combines_version_and_target() {
        assert_eq!(
            staging_dir_name("0.1.0", "x86_64-unknown-linux-gnu"),
            "spartan-ide-0.1.0-x86_64-unknown-linux-gnu"
        );
    }

    #[test]
    fn desktop_entry_contains_the_real_exec_path_and_required_fields() {
        let contents = desktop_entry_contents("/home/user/.local/bin/spartan-editor-core");
        assert!(contents.contains("[Desktop Entry]"));
        assert!(contents.contains("Type=Application"));
        assert!(contents.contains("Exec=/home/user/.local/bin/spartan-editor-core %F"));
        assert!(contents.contains("Name=Spartan IDE"));
    }

    #[test]
    fn install_script_references_the_real_binary_and_app_id_names() {
        let contents = install_sh_contents();
        assert!(contents.starts_with("#!/bin/sh"));
        assert!(contents.contains(BIN_NAME));
        assert!(contents.contains(APP_ID));
        assert!(contents.contains("$HOME/.local/bin"));
        assert!(contents.contains("$HOME/.local/share/applications"));
        assert!(contents.contains("install-errors.log"));
        assert!(contents.contains(".spartan-editor-core.new"));
    }

    #[test]
    fn readme_mentions_the_real_version_and_install_instructions() {
        let contents = package_readme_contents("0.1.0");
        assert!(contents.contains("Spartan IDE 0.1.0"));
        assert!(contents.contains("./install.sh"));
    }
}
