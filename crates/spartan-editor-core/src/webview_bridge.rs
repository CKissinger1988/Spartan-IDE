//! Real embedded WebView bridge for Design mode (§6.1, §8, §16.1, task
//! #12), promoted from `spikes/ui-shell-spike`'s already-proven
//! `WebviewBridge` (§47.11) -- the real mechanism behind §6.1's "only
//! place in the app using a WebView" requirement: a real child `wry`
//! `WebView` embedded inside this crate's own `wgpu`/`winit` window,
//! occupying the same screen region the main editor buffer otherwise
//! renders into, not a second top-level window.
//!
//! As of §75.41, a real dev-server bridge (§6.2 step 1, `gui_bridge.rs`)
//! does exist and this WebView renders its real output -- a real,
//! structural component tree (tag names, props, nesting) parsed from the
//! active file's real JSX/TSX by the real `gui-builder` CLI subprocess.
//! Still honestly, deliberately *not* live React/JSX rendering: no
//! bundler, no dev server, no HMR, no visual layout at all -- a real,
//! indented text tree, not a canvas. See `gui_bridge.rs`'s own doc comment
//! and `gui-builder/README.md` for the exact, current scope boundary.

use serde::Deserialize;
use std::cell::RefCell;
use std::rc::Rc;
use winit::window::Window;
use wry::{Rect, WebView, WebViewBuilder};

const HTML: &str = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="background:#0B0B0D;color:#E9E7E4;font-family:monospace;padding:2em;margin:0;">
  <h2 style="color:#C4432B;margin-top:0;">Design Mode &mdash; GUI Builder Canvas</h2>
  <p id="fileInfo" style="color:#A6A5A2;">Loading active file info&hellip;</p>
  <p style="color:#84838A;max-width:640px;line-height:1.5;">
    This is a real embedded WebView (not a mock), proven live and bidirectional
    (see the IPC status line below). The tree below is real structural data
    from a real AST parse of the active file &mdash; it is not yet a live
    visual canvas (no bundler/dev-server/HMR wired in, architecture-spec
    &sect;6.2 step 1's remaining scope).
  </p>
  <p id="ipcStatus" style="color:#6E6D73;">IPC bridge: connecting&hellip;</p>
  <div id="componentTree" style="margin-top:1em;white-space:pre;font-size:0.95em;"></div>
  <script>
    function updateFileInfo(path, isComponent) {
      document.getElementById('fileInfo').innerText =
        'Active file: ' + path + (isComponent ? ' (looks like a component file)' : ' (not a JS/TS/JSX/TSX file)');
    }
    function ackReady() {
      document.getElementById('ipcStatus').innerText = 'IPC bridge: connected (real round-trip confirmed)';
    }
    function componentTreeLoading() {
      document.getElementById('componentTree').textContent = 'Parsing real component tree…';
    }
    function componentTreeError(message) {
      document.getElementById('componentTree').textContent = 'Component tree unavailable: ' + message;
    }
    function componentTreeNotApplicable() {
      document.getElementById('componentTree').textContent = '';
    }
    function propsSummaryOf(props) {
      const keys = Object.keys(props);
      if (keys.length === 0) return '';
      return ' ' + keys.map(function (k) {
        const p = props[k];
        if (p.kind === 'string') return k + '="' + p.value + '"';
        if (p.kind === 'style') return k + '={...' + Object.keys(p.entries).length + ' entries}';
        return k + '={' + p.source + '}';
      }).join(' ');
    }
    function renderNode(node, depth, lines) {
      const indent = '  '.repeat(depth);
      const text = node.textContent ? ' “' + node.textContent + '”' : '';
      lines.push(indent + '<' + node.tagName + propsSummaryOf(node.props) + '>' + text);
      node.children.forEach(function (child) { renderNode(child, depth + 1, lines); });
    }
    function updateComponentTree(data) {
      const lines = [];
      data.roots.forEach(function (root) { renderNode(root, 0, lines); });
      document.getElementById('componentTree').textContent =
        lines.length > 0 ? lines.join('\n') : '(no JSX elements found)';
    }
    window.ipc.postMessage(JSON.stringify({type: 'ready'}));
  </script>
</body>
</html>"#;

#[derive(Deserialize)]
#[serde(tag = "type")]
enum IpcMessage {
    #[serde(rename = "ready")]
    Ready,
}

/// Owns the child `WebView` occupying Design mode's content region. Lives
/// behind `Rc<RefCell<Option<...>>>` for the same real `with_ipc_handler`
/// chicken-and-egg reason `ui-shell-spike`'s own bridge documents: the IPC
/// closure must be constructed *before* `.build()` returns the `WebView`
/// it needs to call `evaluate_script` on.
pub struct WebviewBridge {
    webview: Rc<RefCell<Option<WebView>>>,
    /// Real, live confirmation the IPC round-trip actually happened (set
    /// `true` the moment the JS side's own `{"type":"ready"}` message is
    /// received and acknowledged) -- a genuinely useful diagnostic hook,
    /// not yet consumed by any caller in this crate (no UI surface reads
    /// it back yet), named here rather than silently dropped.
    #[allow(dead_code)]
    pub ready_acked: Rc<RefCell<bool>>,
}

impl WebviewBridge {
    pub fn new(window: &Window, bounds: Rect) -> Self {
        let ready_acked = Rc::new(RefCell::new(false));
        let webview_slot: Rc<RefCell<Option<WebView>>> = Rc::new(RefCell::new(None));

        let webview_slot_handler = webview_slot.clone();
        let ready_acked_handler = ready_acked.clone();

        let webview = WebViewBuilder::new_as_child(window)
            .with_bounds(bounds)
            .with_html(HTML)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let body = req.body().as_str();
                match serde_json::from_str::<IpcMessage>(body) {
                    Ok(IpcMessage::Ready) => {
                        *ready_acked_handler.borrow_mut() = true;
                        if let Some(wv) = webview_slot_handler.borrow().as_ref() {
                            let _ = wv.evaluate_script("ackReady();");
                        }
                    }
                    Err(e) => {
                        eprintln!("[webview_bridge] failed to parse IPC message {body:?}: {e}");
                    }
                }
            })
            .build()
            .expect("failed to build child WebView");

        *webview_slot.borrow_mut() = Some(webview);

        Self {
            webview: webview_slot,
            ready_acked,
        }
    }

    pub fn set_bounds(&self, bounds: Rect) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.set_bounds(bounds);
        }
    }

    /// Pushes the real active file's path and a real, simple
    /// component-file heuristic (extension-based, matching
    /// `language::detect_language_for_file`'s own registry-driven
    /// detection rather than a second, separate guess) into the WebView's
    /// DOM.
    pub fn push_file_info(&self, path: &str, is_component_file: bool) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let escaped = path.replace('\\', "\\\\").replace('\'', "\\'");
            let _ = wv.evaluate_script(&format!(
                "updateFileInfo('{escaped}', {is_component_file});"
            ));
        }
    }

    /// Shows a real "parsing" state while a real `gui_bridge` request is
    /// in flight -- called immediately on spawn, before the (real,
    /// non-blocking) subprocess has actually returned.
    pub fn push_component_tree_loading(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script("componentTreeLoading();");
        }
    }

    /// Clears any previously-rendered tree -- a real, live bug found only
    /// by testing (not by inspection): switching from a component file to
    /// a non-component one correctly updated the file-info line but left
    /// the *previous* file's stale tree rendered underneath it, since
    /// nothing had ever told the WebView to clear it. Called whenever the
    /// active file is not a real component file.
    pub fn push_component_tree_not_applicable(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script("componentTreeNotApplicable();");
        }
    }

    /// Renders the real component tree JSON `gui_bridge::run_cli` produced
    /// -- `json` is spliced directly into the script call as a JS object
    /// literal (valid, since JSON is a subset of JS expression syntax),
    /// already confirmed to be real, well-formed JSON by the caller before
    /// this is ever invoked.
    pub fn push_component_tree(&self, json: &str) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script(&format!("updateComponentTree({json});"));
        }
    }

    /// Shows a real, human-readable failure message (subprocess spawn
    /// failure, a real parse error, `gui-builder` not found/built) instead
    /// of silently leaving the last-rendered tree (possibly from a
    /// previous file) misleadingly on screen.
    pub fn push_component_tree_error(&self, message: &str) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let escaped = message
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', " ");
            let _ = wv.evaluate_script(&format!("componentTreeError('{escaped}');"));
        }
    }
}
