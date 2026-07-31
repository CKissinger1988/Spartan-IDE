//! Real PTY spawning for Console + Sessions (§75.64, closing the §75.62
//! audit's own named gap: "Console/Sessions share one real blocker --
//! reusing Leo's async `Event` mechanism for streaming PTY output").
//! Ports the same real, already-tested `portable-pty` spawn shape
//! `spartan-editor-core::terminal.rs` established (§75.56/§75.57), moved
//! to this crate's own `Event`-over-stdout streaming instead of an
//! in-process `mpsc` a render loop polls once per frame.
//!
//! Deliberately does **not** strip ANSI escape sequences the way the
//! original wgpu shell's own `terminal.rs` had to (that renderer has no
//! per-cell color grid) -- raw bytes are sent through as-is, real,
//! verbatim, since the Electron shell can drive a real terminal emulator
//! (`xterm.js`) client-side that actually understands them, a genuine
//! fidelity improvement over the wgpu shell's own necessarily-limited
//! plain-text rendering, not a regression.

use std::io::{Read, Write};
use std::sync::mpsc::Sender;
use std::thread;

use crate::Event;

/// Closes the "UTF-8 chunk-boundary reassembly" gap this module's own
/// `spawn_pty` doc comment previously named as a real, un-fixed limitation
/// (`docs/FUTURE_FEATURES.md`'s "Terminal & sessions" table): a multi-byte
/// UTF-8 sequence split exactly across two real `reader.read` calls used
/// to produce a spurious `U+FFFD` replacement character at the boundary,
/// since `String::from_utf8_lossy` was called independently on each raw
/// chunk with no memory of a still-incomplete sequence left dangling at
/// the end of the previous one.
///
/// `std::str::from_utf8`'s own `Utf8Error` already distinguishes the two
/// real cases that matter here: `error_len() == None` means the byte
/// slice simply ran out while still partway through an otherwise-valid
/// multi-byte sequence (exactly the read-boundary case this struct
/// exists to fix -- buffer the dangling tail and wait for the rest to
/// arrive on a later read); `error_len() == Some(n)` means the bytes are
/// genuinely invalid UTF-8, not a chunking artifact, so those are still
/// lossy-decoded immediately rather than buffered forever. A real UTF-8
/// leading byte never claims more than 3 continuation bytes, so the
/// "incomplete" tail this struct ever holds is naturally bounded at 3
/// bytes -- no unbounded-growth guard is needed.
#[derive(Default)]
struct Utf8Reassembler {
    pending: Vec<u8>,
}

impl Utf8Reassembler {
    fn push(&mut self, new_bytes: &[u8]) -> String {
        self.pending.extend_from_slice(new_bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(s) => {
                let result = s.to_string();
                self.pending.clear();
                result
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                let valid_part = std::str::from_utf8(&self.pending[..valid_up_to])
                    .unwrap()
                    .to_string();
                match e.error_len() {
                    Some(_) => {
                        // Genuinely invalid bytes, not a boundary artifact
                        // -- lossy-decode them too rather than buffering
                        // bad data forever, then continue.
                        let lossy_tail =
                            String::from_utf8_lossy(&self.pending[valid_up_to..]).into_owned();
                        self.pending.clear();
                        valid_part + &lossy_tail
                    }
                    None => {
                        // Ran out of bytes mid-sequence -- keep only the
                        // real dangling tail, waiting for the rest.
                        self.pending = self.pending[valid_up_to..].to_vec();
                        valid_part
                    }
                }
            }
        }
    }

    /// Real, deliberate termination-path flush: if the real reader loop
    /// hits EOF (`Ok(0)`) or a real read error while a genuinely incomplete
    /// multi-byte sequence is still buffered (waiting on a read that will
    /// now never arrive), that tail must not simply vanish -- lossy-decode
    /// whatever real bytes are left (the same `U+FFFD`-substitution
    /// convention `push`'s own `Some(_)` branch already uses for genuinely
    /// invalid bytes) rather than silently dropping the process's own last
    /// few real output bytes. Returns an empty string (a real, harmless
    /// no-op for the caller) when nothing was pending.
    fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let result = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        result
    }
}

pub struct PtyHandle {
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    pub fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(data)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        self.master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

/// Real spawn -- `command` of `None` defaults to the real `$SHELL` (or
/// `/bin/sh`), matching `terminal.rs::spawn`'s own real convention for
/// Console; a real named command (`claude`/`codex`/`gemini`/...) is how
/// Sessions reuses this exact same primitive rather than needing a
/// second implementation.
pub fn spawn_pty(
    session_id: u64,
    cwd: &std::path::Path,
    cols: u16,
    rows: u16,
    command: Option<&str>,
    args: &[String],
    out_tx: Sender<String>,
) -> std::io::Result<PtyHandle> {
    let shell_owned;
    let command = match command {
        Some(c) => c,
        None => {
            shell_owned = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            &shell_owned
        }
    };

    let pty_system = portable_pty::native_pty_system();
    let pty_pair = pty_system
        .openpty(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(std::io::Error::other)?;

    let mut cmd = portable_pty::CommandBuilder::new(command);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.cwd(cwd);

    let child = pty_pair
        .slave
        .spawn_command(cmd)
        .map_err(std::io::Error::other)?;

    let mut reader = pty_pair
        .master
        .try_clone_reader()
        .map_err(std::io::Error::other)?;
    let writer = pty_pair
        .master
        .take_writer()
        .map_err(std::io::Error::other)?;

    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut reassembler = Utf8Reassembler::default();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Real incremental UTF-8 reassembly across read
                    // boundaries -- closes the gap this loop's own doc
                    // comment used to name, see `Utf8Reassembler` above.
                    let chunk = reassembler.push(&buf[..n]);
                    if chunk.is_empty() {
                        // A real dangling incomplete sequence with
                        // nothing else to emit yet -- correctly wait for
                        // the next read rather than sending an empty event.
                        continue;
                    }
                    let event = Event {
                        event: "pty_output".to_string(),
                        data: serde_json::json!({ "session_id": session_id, "chunk": chunk }),
                    };
                    if let Ok(line) = serde_json::to_string(&event) {
                        if out_tx.send(line).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        // A genuinely incomplete multi-byte sequence can still be sitting
        // in `reassembler` at real EOF/error time -- the read that would
        // have completed it is never coming, so flush whatever real bytes
        // are left (lossy-decoded, matching `push`'s own convention for
        // genuinely invalid bytes) instead of silently discarding them.
        let tail = reassembler.flush();
        if !tail.is_empty() {
            let event = Event {
                event: "pty_output".to_string(),
                data: serde_json::json!({ "session_id": session_id, "chunk": tail }),
            };
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = out_tx.send(line);
            }
        }
        let event = Event {
            event: "pty_exit".to_string(),
            data: serde_json::json!({ "session_id": session_id }),
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(PtyHandle {
        writer,
        master: pty_pair.master,
        child,
    })
}

#[cfg(test)]
mod utf8_reassembler_tests {
    use super::Utf8Reassembler;

    #[test]
    fn a_plain_ascii_chunk_passes_through_unchanged() {
        let mut r = Utf8Reassembler::default();
        assert_eq!(r.push(b"hello world"), "hello world");
    }

    #[test]
    fn a_multi_byte_char_split_exactly_at_the_boundary_reassembles_correctly() {
        // "é" is U+00E9, encoded as the 2 real bytes 0xC3 0xA9.
        let full = "café".as_bytes().to_vec();
        let split_at = full.len() - 1; // right before the last byte of "é"
        let mut r = Utf8Reassembler::default();
        let first = r.push(&full[..split_at]);
        assert_eq!(
            first, "caf",
            "the dangling incomplete byte must not leak through yet"
        );
        let second = r.push(&full[split_at..]);
        assert_eq!(
            second, "é",
            "the completed sequence must resolve on the next chunk"
        );
    }

    #[test]
    fn a_four_byte_emoji_split_across_three_separate_reads_reassembles_correctly() {
        // A real 4-byte UTF-8 sequence (an emoji), split into three
        // single-byte reads plus a final read -- the worst realistic case.
        let full = "🎉".as_bytes().to_vec();
        assert_eq!(full.len(), 4);
        let mut r = Utf8Reassembler::default();
        assert_eq!(r.push(&full[..1]), "");
        assert_eq!(r.push(&full[1..2]), "");
        assert_eq!(r.push(&full[2..3]), "");
        assert_eq!(r.push(&full[3..4]), "🎉");
    }

    #[test]
    fn genuinely_invalid_bytes_are_lossy_decoded_not_buffered_forever() {
        let mut r = Utf8Reassembler::default();
        // 0xFF is never a valid UTF-8 byte in any position.
        let out = r.push(&[b'a', 0xFF, b'b']);
        // Exact real output, not just "the surrounding real bytes survive":
        // `String::from_utf8_lossy` replaces the one invalid byte with
        // exactly one real U+FFFD replacement character.
        assert_eq!(out, "a\u{FFFD}b");
        assert!(
            r.pending.is_empty(),
            "an invalid byte must not be held onto forever"
        );
    }

    #[test]
    fn a_real_multi_line_chunk_containing_a_split_multi_byte_char_reassembles_correctly() {
        let full = "line one\ncafé line two\n".as_bytes().to_vec();
        let split_at = full.iter().position(|&b| b == 0xA9).unwrap(); // mid "é"
        let mut r = Utf8Reassembler::default();
        let mut out = r.push(&full[..split_at]);
        out.push_str(&r.push(&full[split_at..]));
        assert_eq!(out, "line one\ncafé line two\n");
    }

    #[test]
    fn flush_lossy_decodes_a_real_dangling_tail_left_at_termination() {
        // "é" split so only its leading byte (0xC3) ever arrives -- the
        // read that would deliver the rest (0xA9) never comes because the
        // real process has already exited.
        let full = "café".as_bytes().to_vec();
        let split_at = full.len() - 1;
        let mut r = Utf8Reassembler::default();
        let before_eof = r.push(&full[..split_at]);
        assert_eq!(before_eof, "caf");
        assert!(!r.pending.is_empty(), "the dangling byte must be buffered");

        let flushed = r.flush();
        assert_eq!(
            flushed, "\u{FFFD}",
            "the real dangling byte is lossy-decoded, not dropped"
        );
        assert!(r.pending.is_empty(), "flush must clear the buffer");
    }

    #[test]
    fn flush_on_a_clean_boundary_is_a_real_harmless_no_op() {
        let mut r = Utf8Reassembler::default();
        assert_eq!(r.push(b"hello"), "hello");
        assert_eq!(r.flush(), "", "nothing was pending, so flush emits nothing");
    }
}
