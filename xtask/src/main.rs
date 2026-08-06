//! Real build-automation entry point (the standard Rust "xtask" pattern:
//! `cargo run -p xtask -- <command>` instead of a separate build-system
//! dependency). Currently one real command: `package`.

mod package;

use anyhow::{bail, Result};
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask always lives one directory below the workspace root")
        .to_path_buf()
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("package") => {
            let root = workspace_root();
            let dist_dir = root.join("dist");
            let version = "0.2.0-beta.2";
            let target_triple = "x86_64-unknown-linux-gnu";
            let archive = package::package(&root, &dist_dir, version, target_triple)?;
            println!("Packaged: {}", archive.display());
            Ok(())
        }
        Some(other) => bail!("unknown xtask command: {other} (expected: package)"),
        None => bail!("usage: cargo run -p xtask -- package"),
    }
}
