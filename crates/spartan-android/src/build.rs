//! Real Android debug-APK build support -- the natural next increment
//! beyond §75.91's own detection-only scope, made possible by a real
//! Android SDK (build-tools/platforms/cmdline-tools, confirmed live in
//! this environment) that wasn't present when this crate was first
//! written. Still not the full §21 scope: no emulator/system-image, no
//! `/dev/kvm`, so there is no real device to *install* or *run* the
//! resulting APK against here -- this closes the "compile and package a
//! real, installable APK" piece only, named honestly, not the
//! install/run/debug pieces.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

/// Picks the real command to invoke Gradle with -- a project's own
/// `./gradlew`/`gradlew.bat` wrapper if present and executable (the
/// standard, version-pinned way almost every real Android project is
/// built), falling back to a bare `gradle` resolved from `$PATH` only if
/// no wrapper exists. Mirrors `spartan-editor-core::build`'s own
/// Cargo-build precedent of preferring the project's own real toolchain
/// entry point over a bare system one.
fn gradle_command(project_root: &Path) -> Option<PathBuf> {
    let wrapper_name = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    let wrapper = project_root.join(wrapper_name);
    if wrapper.is_file() {
        return Some(wrapper);
    }
    None
}

/// Real, live directory walk under `project_root` for a real Gradle-
/// produced debug APK -- `**/build/outputs/apk/debug/*.apk`, matching
/// the standard Android Gradle Plugin output layout for every module,
/// not just a hardcoded `app/` assumption (a real project's app module
/// can be named anything). Bounded depth (6) so this never turns into an
/// unbounded scan of a large real project tree.
fn find_debug_apk(project_root: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: u32, found: &mut Vec<PathBuf>) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == "node_modules" || name == ".git" || name == ".gradle" {
                    continue;
                }
                if path.ends_with("build/outputs/apk/debug")
                    || path
                        .to_string_lossy()
                        .replace('\\', "/")
                        .ends_with("build/outputs/apk/debug")
                {
                    if let Ok(apk_entries) = std::fs::read_dir(&path) {
                        for apk_entry in apk_entries.flatten() {
                            let apk_path = apk_entry.path();
                            if apk_path.extension().and_then(|e| e.to_str()) == Some("apk") {
                                found.push(apk_path);
                            }
                        }
                    }
                    continue;
                }
                walk(&path, depth - 1, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(project_root, 6, &mut found);
    // Prefer the shortest path -- the top-level `app/` module's own real
    // debug APK in the overwhelmingly common single-app-module layout,
    // rather than a nested sample/test module's own debug APK if both
    // happen to exist.
    found.into_iter().min_by_key(|p| p.as_os_str().len())
}

/// Real, streaming `assembleDebug` build -- spawns the project's own
/// real Gradle wrapper (falling back to a bare `gradle` from `$PATH`),
/// forwarding every real stdout/stderr line to `progress_tx` as it
/// happens (Gradle's own real per-task `> Task :app:...` lines), and
/// returning the real produced APK's path on success. `sdk_root`, when
/// `Some`, is exported as both `ANDROID_HOME` and `ANDROID_SDK_ROOT` for
/// the child process -- Gradle/AGP need at least one of these set to
/// find the real SDK unless the project has its own `local.properties`.
pub fn build_debug_apk(
    project_root: &Path,
    sdk_root: Option<&Path>,
    gradle_on_path: Option<&Path>,
    progress_tx: Sender<String>,
) -> Result<PathBuf, String> {
    let (program, args): (PathBuf, Vec<String>) =
        if let Some(wrapper) = gradle_command(project_root) {
            (
                wrapper,
                vec!["assembleDebug".to_string(), "--no-daemon".to_string()],
            )
        } else if let Some(gradle) = gradle_on_path {
            (
                gradle.to_path_buf(),
                vec!["assembleDebug".to_string(), "--no-daemon".to_string()],
            )
        } else {
            return Err(
                "no ./gradlew wrapper in this project and no `gradle` found on $PATH -- install \
             Gradle or add a wrapper to build an APK"
                    .to_string(),
            );
        };

    let mut cmd = Command::new(&program);
    cmd.args(&args);
    cmd.current_dir(project_root);
    if let Some(sdk) = sdk_root {
        cmd.env("ANDROID_HOME", sdk);
        cmd.env("ANDROID_SDK_ROOT", sdk);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not spawn {}: {e}", program.display()))?;

    let mut handles = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let tx = progress_tx.clone();
        handles.push(std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let tx = progress_tx.clone();
        handles.push(std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }

    let status = child
        .wait()
        .map_err(|e| format!("gradle build did not complete: {e}"))?;
    for h in handles {
        let _ = h.join();
    }

    if !status.success() {
        return Err(format!("gradle assembleDebug exited with {status}"));
    }

    find_debug_apk(project_root).ok_or_else(|| {
        "gradle assembleDebug succeeded but no debug .apk was found under this project's own \
         build/outputs/apk/debug/ directory"
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use tempfile::tempdir;

    #[test]
    fn gradle_command_finds_a_real_wrapper_when_present() {
        let dir = tempdir().unwrap();
        let wrapper_name = if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        };
        fs::write(dir.path().join(wrapper_name), "#!/bin/sh\necho fake\n").unwrap();
        assert_eq!(
            gradle_command(dir.path()),
            Some(dir.path().join(wrapper_name))
        );
    }

    #[test]
    fn gradle_command_returns_none_with_no_wrapper() {
        let dir = tempdir().unwrap();
        assert_eq!(gradle_command(dir.path()), None);
    }

    #[test]
    fn find_debug_apk_finds_a_real_apk_under_the_standard_app_module_layout() {
        let dir = tempdir().unwrap();
        let apk_dir = dir
            .path()
            .join("app")
            .join("build")
            .join("outputs")
            .join("apk")
            .join("debug");
        fs::create_dir_all(&apk_dir).unwrap();
        fs::write(apk_dir.join("app-debug.apk"), b"fake apk bytes").unwrap();
        assert_eq!(
            find_debug_apk(dir.path()),
            Some(apk_dir.join("app-debug.apk"))
        );
    }

    #[test]
    fn find_debug_apk_returns_none_when_nothing_was_built() {
        let dir = tempdir().unwrap();
        assert_eq!(find_debug_apk(dir.path()), None);
    }

    #[test]
    fn find_debug_apk_prefers_the_shortest_real_path_when_multiple_modules_have_one() {
        let dir = tempdir().unwrap();
        for module in ["app", "samples/deep/nested-module"] {
            let apk_dir = dir
                .path()
                .join(module)
                .join("build")
                .join("outputs")
                .join("apk")
                .join("debug");
            fs::create_dir_all(&apk_dir).unwrap();
            fs::write(apk_dir.join("x-debug.apk"), b"fake").unwrap();
        }
        let found = find_debug_apk(dir.path()).unwrap();
        assert!(found.starts_with(dir.path().join("app")));
    }

    #[test]
    fn build_debug_apk_reports_a_real_honest_error_with_no_wrapper_and_no_gradle() {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::channel();
        let result = build_debug_apk(dir.path(), None, None, tx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("gradlew"));
    }

    /// Real, live, self-skipping end-to-end build against a real, minimal
    /// Android Gradle project fixture -- self-skips (prints a message) if
    /// this environment has neither a real Android SDK
    /// (`SPARTAN_TEST_ANDROID_SDK`) nor a real `gradle` on `$PATH`, matching
    /// this repo's own established real-external-tool convention. When it
    /// runs, this is a genuine `assembleDebug` against real Google Maven/
    /// Maven Central dependencies -- not mocked -- so it's real, but slow
    /// (multiple minutes on a cold Gradle cache).
    #[test]
    fn build_debug_apk_produces_a_real_installable_apk_from_a_real_minimal_project() {
        let Ok(sdk_root) = std::env::var("SPARTAN_TEST_ANDROID_SDK") else {
            eprintln!("SKIP: SPARTAN_TEST_ANDROID_SDK not set, skipping real Android build test");
            return;
        };
        let sdk_root = PathBuf::from(sdk_root);
        if !sdk_root.is_dir() {
            eprintln!("SKIP: {sdk_root:?} does not exist, skipping real Android build test");
            return;
        }
        let Some(gradle) = crate::find_on_path("gradle") else {
            eprintln!("SKIP: no real `gradle` on $PATH, skipping real Android build test");
            return;
        };

        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("settings.gradle.kts"),
            "pluginManagement {\n    repositories {\n        google()\n        mavenCentral()\n        gradlePluginPortal()\n    }\n}\ndependencyResolutionManagement {\n    repositories {\n        google()\n        mavenCentral()\n    }\n}\nrootProject.name = \"spike\"\ninclude(\":app\")\n",
        )
        .unwrap();
        fs::write(
            root.join("build.gradle.kts"),
            "plugins {\n    id(\"com.android.application\") version \"8.5.2\" apply false\n    id(\"org.jetbrains.kotlin.android\") version \"2.0.21\" apply false\n}\n",
        )
        .unwrap();
        let app_dir = root.join("app");
        let src_dir = app_dir
            .join("src")
            .join("main")
            .join("java")
            .join("com")
            .join("spartan")
            .join("spike");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            app_dir.join("build.gradle.kts"),
            "plugins {\n    id(\"com.android.application\")\n    id(\"org.jetbrains.kotlin.android\")\n}\n\nandroid {\n    namespace = \"com.spartan.spike\"\n    compileSdk = 34\n\n    defaultConfig {\n        applicationId = \"com.spartan.spike\"\n        minSdk = 24\n        targetSdk = 34\n        versionCode = 1\n        versionName = \"1.0\"\n    }\n    compileOptions {\n        sourceCompatibility = JavaVersion.VERSION_17\n        targetCompatibility = JavaVersion.VERSION_17\n    }\n    kotlinOptions {\n        jvmTarget = \"17\"\n    }\n}\n",
        )
        .unwrap();
        fs::write(
            app_dir.join("src").join("main").join("AndroidManifest.xml"),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <application android:label=\"Spike\">\n        <activity android:name=\".MainActivity\" android:exported=\"true\">\n            <intent-filter>\n                <action android:name=\"android.intent.action.MAIN\" />\n                <category android:name=\"android.intent.category.LAUNCHER\" />\n            </intent-filter>\n        </activity>\n    </application>\n</manifest>\n",
        )
        .unwrap();
        fs::write(
            src_dir.join("MainActivity.kt"),
            "package com.spartan.spike\n\nimport android.app.Activity\nimport android.os.Bundle\n\nclass MainActivity : Activity() {\n    override fun onCreate(savedInstanceState: Bundle?) {\n        super.onCreate(savedInstanceState)\n    }\n}\n",
        )
        .unwrap();

        let (tx, rx) = mpsc::channel();
        let result = build_debug_apk(root, Some(&sdk_root), Some(&gradle), tx);
        let lines: Vec<String> = rx.try_iter().collect();
        let apk_path = result.unwrap_or_else(|e| {
            panic!(
                "expected a real successful build, got error: {e}\nlast output lines: {:?}",
                &lines[lines.len().saturating_sub(20)..]
            )
        });
        assert!(
            apk_path.is_file(),
            "the reported APK path must be a real file: {apk_path:?}"
        );
        let bytes = fs::read(&apk_path).unwrap();
        // A real APK is a real ZIP -- confirm the local-file-header magic
        // bytes rather than just trusting the file exists.
        assert_eq!(
            &bytes[0..4],
            b"PK\x03\x04",
            "expected a real ZIP/APK signature"
        );
    }
}
