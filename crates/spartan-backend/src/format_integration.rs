//! Real "Format Document" support -- shells out to each language's own
//! real formatter binary (`languages.toml`'s `formatter` field, real and
//! present since §20.1 but never wired to any real caller until now),
//! piping the *live in-memory buffer* through stdin/stdout rather than
//! the file on disk -- the same "operate on the live buffer, not disk"
//! discipline §75.42 established, so formatting an unsaved edit works
//! and a formatted result always applies through the normal `edit` IPC
//! path, getting the same undo/dirty tracking as any other edit.
//!
//! **A real, deliberate, per-formatter dispatch, not a blind "run the
//! configured command"**: `languages.toml`'s own `formatter` entries are
//! written for a project-wide "format this file" CLI invocation (a real
//! file path argument), not a stdin/stdout filter -- `rustfmt`/`black`/
//! `gofmt` all support reading from stdin and writing to stdout, but need
//! different flags (or none at all) to do it, and `prettier` specifically
//! needs `--stdin-filepath <path>` to know which parser to pick,
//! information the static registry entry doesn't carry and can't. This
//! resolves each real, known formatter program to its correct real
//! stdin/stdout invocation, the same "adapt the new caller instead of
//! risking the shared registry" precedent `dap_integration::
//! resolve_dap_command` already set for Python's `debugpy` invocation.
//! `ktlint` (Kotlin, real `--stdin --format` support, confirmed live
//! against a real installed `ktlint` 1.8.0 before writing this) is real
//! and wired; `google-java-format` (Java, real `-` stdin/stdout support,
//! confirmed live against a real installed `google-java-format` 1.24.0
//! before writing this) closes the real, previously-total "Java has no
//! formatter configured at all" gap by giving `languages.toml`'s Java
//! entry a real `[language.formatter]` for the first time. `dotnet
//! format` (C#) genuinely doesn't fit this model -- it's a real,
//! project-wide command with no stdin/stdout filter mode at all, not a
//! flag this crate merely hasn't found yet -- and stays a real, honest,
//! named gap: `resolve_formatter_command` returns `None` for it, and
//! callers report that plainly rather than guessing at an invocation.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use spartan_languages::CommandSpec;

/// Resolves a language's own configured formatter into a real
/// stdin-in/stdout-out invocation, or `None` if that program has no known
/// filter-mode invocation (see this module's own doc comment).
pub(crate) fn resolve_formatter_command(
    configured: &CommandSpec,
    path: &Path,
) -> Option<(String, Vec<String>)> {
    match configured.program.as_str() {
        "rustfmt" => Some(("rustfmt".to_string(), Vec::new())),
        "black" => Some(("black".to_string(), vec!["-q".to_string(), "-".to_string()])),
        "gofmt" => Some(("gofmt".to_string(), Vec::new())),
        "prettier" => Some((
            "prettier".to_string(),
            vec![
                "--stdin-filepath".to_string(),
                path.to_string_lossy().into_owned(),
            ],
        )),
        // Real, confirmed live against ktlint 1.8.0: `--stdin` reads the
        // buffer, `--format`/`-F` autocorrects, `--stdin-path` gives it
        // the real file path (matching prettier's own `--stdin-filepath`
        // reasoning -- ktlint's `.editorconfig` resolution is path-keyed),
        // and `--log-level=none` keeps its own JVM/ktlint diagnostic
        // banner off stdout (confirmed via a real run that it always
        // lands on stderr regardless of this flag, but silencing it here
        // avoids depending on that never changing).
        "ktlint" => Some((
            "ktlint".to_string(),
            vec![
                "--stdin".to_string(),
                format!("--stdin-path={}", path.to_string_lossy()),
                "--format".to_string(),
                "--log-level=none".to_string(),
            ],
        )),
        // Real, confirmed live against google-java-format 1.24.0: a bare
        // `-` argument reads stdin and writes the formatted result to
        // stdout. Closes languages.toml's own real "Java has no formatter
        // configured at all" gap (see this module's own doc comment).
        "google-java-format" => Some(("google-java-format".to_string(), vec!["-".to_string()])),
        _ => None,
    }
}

/// Runs a real formatter subprocess, piping `source` in via stdin and
/// capturing stdout. Distinguishes "the tool isn't installed" (a real
/// `io::Error` spawning it) from "the tool ran but rejected the input"
/// (non-zero exit, real stderr content) so a caller can report something
/// more useful than a bare "format failed."
pub(crate) fn run_formatter(
    program: &str,
    args: &[String],
    source: &str,
) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn formatter `{program}`: {e}"))?;

    // Real deadlock avoidance: writing to a child's stdin while its
    // stdout pipe fills up can block forever if done on the same thread
    // its stdout is later read from -- write on a dedicated thread
    // instead, the standard `std::process` recipe for this exact hazard.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let source_owned = source.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(source_owned.as_bytes());
    });

    let output = child
        .wait_with_output()
        .map_err(|e| format!("formatter `{program}` failed to run: {e}"))?;
    let _ = writer.join();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "formatter `{program}` exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("formatter `{program}` produced invalid UTF-8 output: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_formatter_command_knows_the_six_real_stdin_stdout_capable_formatters() {
        for program in [
            "rustfmt",
            "black",
            "gofmt",
            "prettier",
            "ktlint",
            "google-java-format",
        ] {
            let spec = CommandSpec {
                program: program.to_string(),
                args: Vec::new(),
            };
            assert!(
                resolve_formatter_command(&spec, Path::new("x.txt")).is_some(),
                "expected {program} to resolve"
            );
        }
    }

    #[test]
    fn resolve_formatter_command_honestly_returns_none_for_dotnet_format() {
        let spec = CommandSpec {
            program: "dotnet".to_string(),
            args: vec!["format".to_string()],
        };
        assert!(resolve_formatter_command(&spec, Path::new("x.txt")).is_none());
    }

    #[test]
    fn ktlint_invocation_carries_the_real_stdin_path_it_wants() {
        let spec = CommandSpec {
            program: "ktlint".to_string(),
            args: Vec::new(),
        };
        let (program, args) = resolve_formatter_command(&spec, Path::new("/tmp/thing.kt")).unwrap();
        assert_eq!(program, "ktlint");
        assert_eq!(
            args,
            vec![
                "--stdin",
                "--stdin-path=/tmp/thing.kt",
                "--format",
                "--log-level=none"
            ]
        );
    }

    #[test]
    fn prettier_invocation_carries_the_real_stdin_filepath_it_needs() {
        let spec = CommandSpec {
            program: "prettier".to_string(),
            args: Vec::new(),
        };
        let (program, args) = resolve_formatter_command(&spec, Path::new("/tmp/thing.ts")).unwrap();
        assert_eq!(program, "prettier");
        assert_eq!(args, vec!["--stdin-filepath", "/tmp/thing.ts"]);
    }

    #[test]
    fn run_formatter_on_a_real_unknown_binary_errors_honestly() {
        let err = run_formatter("spartan-definitely-not-a-real-binary", &[], "x").unwrap_err();
        assert!(err.contains("failed to spawn"));
    }

    #[test]
    fn run_formatter_reformats_via_a_real_installed_rustfmt_if_present() {
        // Self-skipping, matching this repo's own established convention
        // for real-external-tool tests: only asserts something if the
        // tool is genuinely present.
        if std::process::Command::new("rustfmt")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("SKIP: rustfmt not installed");
            return;
        }
        let formatted = run_formatter("rustfmt", &[], "fn main( ) { let x=1 ; }").unwrap();
        assert!(formatted.contains("fn main() {"), "got: {formatted}");
    }

    #[test]
    fn run_formatter_reports_a_real_syntax_error_from_a_real_installed_black_if_present() {
        if std::process::Command::new("black")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("SKIP: black not installed");
            return;
        }
        let err =
            run_formatter("black", &["-q".to_string(), "-".to_string()], "def(:").unwrap_err();
        assert!(err.contains("exited with"), "got: {err}");
    }

    #[test]
    fn run_formatter_reformats_via_a_real_installed_ktlint_if_present() {
        if std::process::Command::new("ktlint")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("SKIP: ktlint not installed");
            return;
        }
        let spec = CommandSpec {
            program: "ktlint".to_string(),
            args: Vec::new(),
        };
        // Real, live-caught finding: ktlint's own `standard:filename` rule
        // requires a PascalCase file name -- `/tmp/x.kt` genuinely fails
        // real ktlint, not a bug in this code's own invocation. `Main.kt`
        // is a real, conventional Kotlin entry-point file name.
        let (program, args) = resolve_formatter_command(&spec, Path::new("/tmp/Main.kt")).unwrap();
        let formatted = run_formatter(&program, &args, "fun main(){println(\"hi\")}").unwrap();
        assert!(formatted.contains("fun main() {"), "got: {formatted}");
    }

    #[test]
    fn run_formatter_reformats_via_a_real_installed_google_java_format_if_present() {
        if std::process::Command::new("google-java-format")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("SKIP: google-java-format not installed");
            return;
        }
        let spec = CommandSpec {
            program: "google-java-format".to_string(),
            args: Vec::new(),
        };
        let (program, args) = resolve_formatter_command(&spec, Path::new("/tmp/X.java")).unwrap();
        let formatted = run_formatter(
            &program,
            &args,
            "public class X{public static void main(String[] a){System.out.println(\"hi\");}}",
        )
        .unwrap();
        assert!(formatted.contains("public class X {"), "got: {formatted}");
    }
}
