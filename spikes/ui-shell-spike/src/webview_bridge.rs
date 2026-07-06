use serde::Deserialize;
use std::cell::RefCell;
use std::rc::Rc;
use winit::window::Window;
use wry::{Rect, WebView, WebViewBuilder};

/// The center-stage HTML/JS content: a trivial counter and a button, per
/// §39.4's own stated scope ("one embedded WebView panel showing a trivial
/// counter/state value synced live in both directions"). Real content
/// (`CanvasEdit` events, §6.1-6.2) is explicitly out of scope for this
/// increment -- this proves the bridge mechanism, not the real product
/// surface.
const HTML: &str = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="background:#141416;color:#E9E7E4;font-family:monospace;padding:2em;margin:0;">
  <h2 style="color:#C4432B;">Center Stage &mdash; real embedded WebView</h2>
  <p>Counter (synced Rust &#8596; JS): <span id="counter" style="font-size:1.4em;">0</span></p>
  <button id="incBtn" style="font-family:monospace;font-size:1.1em;padding:0.5em 1em;
    background:#202024;color:#E9E7E4;border:1px solid #35353A;cursor:pointer;">
    Click me (round-trips to Rust and back)
  </button>
  <p id="roundtrip" style="color:#84838A;"></p>
  <script>
    function setCounter(n) {
      document.getElementById('counter').innerText = n;
    }
    // Called by Rust (via evaluate_script) once it has processed a click,
    // closing the loop entirely within JS's own clock (performance.now())
    // so there's no cross-clock skew between Rust's Instant and JS's clock.
    function onRustAck(t0) {
      const elapsed = performance.now() - t0;
      document.getElementById('roundtrip').innerText =
        'Last round-trip: ' + elapsed.toFixed(3) + 'ms';
      window.ipc.postMessage(JSON.stringify({type: 'round_trip', ms: elapsed}));
    }
    document.getElementById('incBtn').addEventListener('click', function () {
      const t0 = performance.now();
      window.ipc.postMessage(JSON.stringify({type: 'click', t0: t0}));
    });
  </script>
</body>
</html>"#;

#[derive(Deserialize)]
#[serde(tag = "type")]
enum IpcMessage {
    #[serde(rename = "click")]
    Click { t0: f64 },
    #[serde(rename = "round_trip")]
    RoundTrip { ms: f64 },
}

/// Owns the child `WebView` occupying the center-stage panel, plus the
/// shared counter and round-trip latency samples the IPC handler and the
/// native keyboard-input path both touch. The `WebView` itself lives behind
/// `Rc<RefCell<Option<...>>>` because `with_ipc_handler` requires a
/// `Fn(Request<String>) + 'static` closure constructed *before* `.build()`
/// returns the `WebView` it needs to call `evaluate_script` on -- a real
/// chicken-and-egg wry API constraint, resolved the same way wry's own
/// examples resolve it elsewhere.
pub struct WebviewBridge {
    webview: Rc<RefCell<Option<WebView>>>,
    pub counter: Rc<RefCell<i64>>,
    pub round_trip_samples: Rc<RefCell<Vec<f64>>>,
}

impl WebviewBridge {
    pub fn new(window: &Window, bounds: Rect) -> Self {
        let counter = Rc::new(RefCell::new(0i64));
        let round_trip_samples = Rc::new(RefCell::new(Vec::new()));
        let webview_slot: Rc<RefCell<Option<WebView>>> = Rc::new(RefCell::new(None));

        let webview_slot_handler = webview_slot.clone();
        let counter_handler = counter.clone();
        let samples_handler = round_trip_samples.clone();

        let webview = WebViewBuilder::new_as_child(window)
            .with_bounds(bounds)
            .with_html(HTML)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let body = req.body().as_str();
                match serde_json::from_str::<IpcMessage>(body) {
                    Ok(IpcMessage::Click { t0 }) => {
                        let new_value = {
                            let mut c = counter_handler.borrow_mut();
                            *c += 1;
                            *c
                        };
                        if let Some(wv) = webview_slot_handler.borrow().as_ref() {
                            let _ = wv.evaluate_script(&format!(
                                "onRustAck({t0}); setCounter({new_value});"
                            ));
                        }
                    }
                    Ok(IpcMessage::RoundTrip { ms }) => {
                        samples_handler.borrow_mut().push(ms);
                    }
                    Err(e) => {
                        eprintln!("[ui-shell-spike] failed to parse IPC message {body:?}: {e}");
                    }
                }
            })
            .build()
            .expect("failed to build child WebView");

        *webview_slot.borrow_mut() = Some(webview);

        Self {
            webview: webview_slot,
            counter,
            round_trip_samples,
        }
    }

    pub fn set_bounds(&self, bounds: Rect) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.set_bounds(bounds);
        }
    }

    /// Pushes a counter value from the native side into the WebView's DOM --
    /// the Rust -> JS half of "synced live in both directions" (the JS ->
    /// Rust half is the IPC click handler above).
    pub fn push_counter_from_native(&self, value: i64) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script(&format!("setCounter({value});"));
        }
    }

    /// Dispatches a real DOM click on the button via script injection --
    /// exercises the exact same `onclick` handler and IPC path a physical
    /// mouse click would, used by the scripted round-trip benchmark (see
    /// `main.rs`) instead of fragile OS-level mouse-coordinate clicking.
    pub fn trigger_synthetic_click(&self) {
        if let Some(wv) = self.webview.borrow().as_ref() {
            let _ = wv.evaluate_script("document.getElementById('incBtn').click();");
        }
    }
}
