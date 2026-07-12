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
//!
//! As of §75.42, the real Canvas -> Code direction is wired too: clicking
//! a real rendered tree row selects it and shows a real inline edit form
//! (key/value + "Set Prop"/"Set Style" buttons); submitting posts a real
//! structured `CanvasEdit` back over IPC, which `main.rs` picks up via
//! `take_pending_edit()` and applies through `gui_bridge::
//! spawn_apply_edit_request` against the real live buffer. Still no
//! click-to-select-on-a-visual-canvas (there is no visual canvas) -- this
//! is a real, structural, text-tree-driven edit UI, not WYSIWYG.
//!
//! As of §75.52, there **is** a real visual canvas: a real `<iframe>`
//! showing the real, live-rendered output of `gui_bridge::
//! spawn_bundle_request`'s real esbuild bundle, set via `srcdoc` (so its
//! own DOM/CSS/JS execution is genuinely isolated from this outer page's
//! own DOM, the standard, correct way to embed arbitrary rendered
//! content rather than injecting it into the same document). The
//! structural text-tree editor above stays as-is -- this is an addition,
//! not a replacement -- so Canvas -> Code editing keeps working exactly
//! as before while the iframe gives real visual confirmation of the
//! result.
//!
//! As of §75.53, the visual canvas is real-ly clickable: `gui-builder`'s
//! own bundler now annotates every rendered element with a real
//! `data-spartan-id` attribute (the exact id the structural tree already
//! uses) and posts `{type:'spartan-canvas-click', nodeId}` across the
//! sandbox boundary on click; this page's own `message` listener routes
//! that through the same `selectNode` the tree's own row clicks already
//! call, so clicking an element directly in the live render opens the
//! identical edit form a tree-row click would.

use serde::Deserialize;
use std::cell::RefCell;
use std::rc::Rc;
use winit::window::Window;
use wry::{Rect, WebView, WebViewBuilder};

const HTML: &str = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="background:#0B0B0D;color:#E9E7E4;font-family:monospace;padding:2em;margin:0;">
  <h2 style="color:#2E7DFF;margin-top:0;">Design Mode &mdash; GUI Builder Canvas</h2>
  <p id="fileInfo" style="color:#A6A5A2;">Loading active file info&hellip;</p>
  <p style="color:#84838A;max-width:640px;line-height:1.5;">
    This is a real embedded WebView (not a mock), proven live and bidirectional
    (see the IPC status line below). The tree below is real structural data
    from a real AST parse of the active file &mdash; it is not yet a live
    visual canvas (no bundler/dev-server/HMR wired in, architecture-spec
    &sect;6.2 step 1's remaining scope).
  </p>
  <p id="ipcStatus" style="color:#6E6D73;">IPC bridge: connecting&hellip;</p>
  <div id="livePreviewWrap" style="margin-top:1em;">
    <p id="livePreviewStatus" style="color:#84838A;font-size:0.9em;margin-bottom:0.3em;"></p>
    <iframe id="livePreview" title="Live component preview"
      style="width:100%;height:320px;border:1px solid #333;background:#fff;display:none;"
      sandbox="allow-scripts"></iframe>
  </div>
  <div id="componentTree" style="margin-top:1em;white-space:pre;font-size:0.95em;"></div>
  <div id="editPanel" style="display:none;margin-top:1em;padding:0.75em;border:1px solid #333;max-width:520px;">
    <div id="editPanelTitle" style="color:#E9E7E4;margin-bottom:0.5em;font-size:0.9em;"></div>
    <div>
      <label style="color:#A6A5A2;">Key
        <input id="editKey" type="text" style="width:8em;margin-left:0.3em;" />
      </label>
      <label style="color:#A6A5A2;margin-left:0.75em;">Value
        <input id="editValue" type="text" style="width:14em;margin-left:0.3em;" />
      </label>
    </div>
    <div style="margin-top:0.5em;">
      <button onclick="submitEdit('PropChange')">Set Prop</button>
      <button onclick="submitEdit('StyleChange')" style="margin-left:0.4em;">Set Style</button>
    </div>
    <div id="editStatus" style="color:#84838A;margin-top:0.4em;font-size:0.9em;"></div>
  </div>
  <script>
    var selectedNodeId = null;

    function updateFileInfo(path, isComponent) {
      document.getElementById('fileInfo').innerText =
        'Active file: ' + path + (isComponent ? ' (looks like a component file)' : ' (not a JS/TS/JSX/TSX file)');
    }
    function ackReady() {
      document.getElementById('ipcStatus').innerText = 'IPC bridge: connected (real round-trip confirmed)';
    }
    function hideEditPanel() {
      selectedNodeId = null;
      document.getElementById('editPanel').style.display = 'none';
    }
    function componentTreeLoading() {
      document.getElementById('componentTree').textContent = 'Parsing real component tree…';
      hideEditPanel();
    }
    function componentTreeError(message) {
      document.getElementById('componentTree').textContent = 'Component tree unavailable: ' + message;
      hideEditPanel();
    }
    function componentTreeNotApplicable() {
      document.getElementById('componentTree').textContent = '';
      hideEditPanel();
    }
    function bundleLoading() {
      document.getElementById('livePreviewStatus').textContent = 'Rendering live preview…';
      document.getElementById('livePreview').style.display = 'none';
    }
    function bundleNotApplicable() {
      document.getElementById('livePreviewStatus').textContent = '';
      document.getElementById('livePreview').style.display = 'none';
    }
    function showBundle(code) {
      const doc = '<!DOCTYPE html><html><head><meta charset="utf-8"></head>' +
        '<body style="margin:0;padding:8px;font-family:sans-serif;">' +
        '<div id="spartan-root"></div><script>' + code + '<' + '/script>' +
        '</body></html>';
      const frame = document.getElementById('livePreview');
      frame.srcdoc = doc;
      frame.style.display = 'block';
      document.getElementById('livePreviewStatus').textContent =
        'Live preview (real esbuild bundle, sandboxed iframe)';
    }
    function bundleError(message) {
      document.getElementById('livePreview').style.display = 'none';
      document.getElementById('livePreviewStatus').textContent = 'Live preview error: ' + message;
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
    function escapeHtml(s) {
      return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    }
    function selectNode(nodeId, tagName) {
      selectedNodeId = nodeId;
      document.getElementById('editPanel').style.display = 'block';
      document.getElementById('editPanelTitle').innerText = 'Selected: <' + tagName + '> (id ' + nodeId + ')';
      document.getElementById('editStatus').innerText = '';
    }
    function submitEdit(kind) {
      if (!selectedNodeId) return;
      const key = document.getElementById('editKey').value;
      const value = document.getElementById('editValue').value;
      if (!key) {
        document.getElementById('editStatus').innerText = 'Key is required.';
        return;
      }
      const edit = kind === 'StyleChange'
        ? {kind: 'StyleChange', nodeId: selectedNodeId, property: key, value: value}
        : {kind: 'PropChange', nodeId: selectedNodeId, prop: key, value: value};
      document.getElementById('editStatus').innerText = 'Applying…';
      window.ipc.postMessage(JSON.stringify({type: 'edit', edit: edit}));
    }
    function editApplied() {
      document.getElementById('editStatus').innerText = 'Applied — real source updated, component tree refreshed.';
    }
    function editError(message) {
      document.getElementById('editStatus').innerText = 'Error: ' + message;
    }
    function renderNode(node, depth, lines) {
      const indent = '&nbsp;&nbsp;'.repeat(depth);
      const text = node.textContent ? ' &ldquo;' + escapeHtml(node.textContent) + '&rdquo;' : '';
      const label = '&lt;' + escapeHtml(node.tagName) + escapeHtml(propsSummaryOf(node.props)) + '&gt;' + text;
      const safeId = node.id.replace(/'/g, "\\'");
      const safeTag = node.tagName.replace(/'/g, "\\'");
      lines.push(
        '<div style="cursor:pointer;padding:1px 2px;" onclick="selectNode(\'' + safeId + '\', \'' + safeTag + '\')">' +
        indent + label +
        '</div>'
      );
      node.children.forEach(function (child) { renderNode(child, depth + 1, lines); });
    }
    var lastTreeRoots = [];
    function updateComponentTree(data) {
      lastTreeRoots = data.roots;
      const lines = [];
      data.roots.forEach(function (root) { renderNode(root, 0, lines); });
      document.getElementById('componentTree').innerHTML =
        lines.length > 0 ? lines.join('') : '(no JSX elements found)';
      hideEditPanel();
    }
    function findNodeById(nodes, id) {
      for (const node of nodes) {
        if (node.id === id) return node;
        const found = findNodeById(node.children, id);
        if (found) return found;
      }
      return null;
    }
    // Real §75.53 click-to-select relay: the sandboxed live-preview
    // iframe (no "allow-same-origin") can only reach this outer page via
    // postMessage -- received here and routed through the exact same
    // `selectNode` the structural tree's own row clicks already use, so
    // a canvas click and a tree-row click for the same element produce
    // an identical selection.
    window.addEventListener('message', function (event) {
      const msg = event.data;
      if (!msg || msg.type !== 'spartan-canvas-click') return;
      const node = findNodeById(lastTreeRoots, msg.nodeId);
      selectNode(msg.nodeId, node ? node.tagName : 'unknown');
    });
    window.ipc.postMessage(JSON.stringify({type: 'ready'}));
  </script>
</body>
</html>"#;

#[derive(Deserialize)]
#[serde(tag = "type")]
enum IpcMessage {
    #[serde(rename = "ready")]
    Ready,
    /// A real, structured `CanvasEdit` (§75.42) the JS side's edit form
    /// posted -- `edit` is passed through as raw JSON rather than a typed
    /// Rust struct, since its exact shape (`PropChange`/`StyleChange`,
    /// each with different fields) already exactly matches
    /// `gui-builder`'s own `CanvasEdit` union and is going straight back
    /// out to that same CLI as a JSON string; re-typing it here would just
    /// be a second, redundant definition to keep in sync.
    #[serde(rename = "edit")]
    Edit { edit: serde_json::Value },
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
    /// The most recent real `CanvasEdit` JSON (§75.42) the JS side's edit
    /// form posted, if any hasn't been consumed yet -- `main.rs`'s
    /// `AboutToWait` polls this via `take_pending_edit()`, the same
    /// "IPC handler can only stash data, not act on it directly" pattern
    /// `component_tree_request` already established, since applying an
    /// edit needs access to the live `OpenFile`/`Document` state the IPC
    /// closure itself has no way to reach.
    pending_edit: Rc<RefCell<Option<String>>>,
}

impl WebviewBridge {
    pub fn new(window: &Window, bounds: Rect) -> Self {
        let ready_acked = Rc::new(RefCell::new(false));
        let pending_edit = Rc::new(RefCell::new(None));
        let webview_slot: Rc<RefCell<Option<WebView>>> = Rc::new(RefCell::new(None));

        let webview_slot_handler = webview_slot.clone();
        let ready_acked_handler = ready_acked.clone();
        let pending_edit_handler = pending_edit.clone();

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
                    Ok(IpcMessage::Edit { edit }) => {
                        *pending_edit_handler.borrow_mut() = Some(edit.to_string());
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
            pending_edit,
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

    /// Real §75.52 live-preview loading state, shown immediately on spawn
    /// (the real esbuild bundle subprocess hasn't returned yet).
    pub fn push_bundle_loading(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script("bundleLoading();");
        }
    }

    /// Hides the live preview entirely (not a component file, or no real
    /// project root) -- same "empty is the ordinary state" rule
    /// `push_component_tree_not_applicable` already established.
    pub fn push_bundle_not_applicable(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script("bundleNotApplicable();");
        }
    }

    /// Renders the real, live esbuild bundle (§75.52) into the sandboxed
    /// `<iframe>`. `code` is real, arbitrary JS text (not JSON) -- passed
    /// through `serde_json::to_string` to get a real, correctly escaped JS
    /// string literal (JSON string encoding is a valid subset of JS string
    /// literal syntax), never spliced raw the way the already-JSON tree
    /// payload is.
    pub fn push_bundle(&self, code: &str) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let encoded = serde_json::to_string(code).unwrap_or_else(|_| "\"\"".to_string());
            let _ = wv.evaluate_script(&format!("showBundle({encoded});"));
        }
    }

    /// Real, human-readable live-preview failure (a real missing
    /// dependency in the target project, a real syntax error, subprocess
    /// spawn failure) -- shown instead of silently leaving a stale
    /// previous render on screen.
    pub fn push_bundle_error(&self, message: &str) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let encoded = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_string());
            let _ = wv.evaluate_script(&format!("bundleError({encoded});"));
        }
    }

    /// Consumes (at most once) the most recent real `CanvasEdit` JSON the
    /// JS side's edit form posted, if any -- polled non-blockingly from
    /// `AboutToWait`, same pattern `gui_bridge`'s own subprocess results
    /// use. Returns `None` on every call once already taken, until the JS
    /// side posts a new one.
    pub fn take_pending_edit(&self) -> Option<String> {
        self.pending_edit.borrow_mut().take()
    }

    /// Real confirmation the edit was applied and the live buffer updated
    /// -- called after a real `gui_bridge::spawn_apply_edit_request`
    /// resolves `Ok`.
    pub fn push_edit_applied(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script("editApplied();");
        }
    }

    /// Real, human-readable failure message for a `CanvasEdit` that
    /// couldn't be applied (unknown node id, unsupported edit shape,
    /// subprocess failure) -- shown in the edit panel rather than silently
    /// discarded.
    pub fn push_edit_error(&self, message: &str) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let escaped = message
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', " ");
            let _ = wv.evaluate_script(&format!("editError('{escaped}');"));
        }
    }
}
