use std::collections::VecDeque;
use std::time::Instant;

/// Verbatim port of `rope-spike`'s `percentiles()` (same format string, same
/// p50/p95/p99/max/n fields) so this spike's report and §47.1's CPU-half
/// report are directly comparable, not just similarly-shaped.
pub fn percentiles(mut v: Vec<f64>, label: &str) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q) as usize];
    println!(
        "  {label:<28} p50={:>8.4}ms  p95={:>8.4}ms  p99={:>8.4}ms  max={:>8.4}ms  n={}",
        p(0.50),
        p(0.95),
        p(0.99),
        v[v.len() - 1],
        v.len()
    );
}

/// Measures real input-to-photon latency: wall-clock time from a
/// `KeyboardInput` event that actually changed the `Document` to the frame
/// that reflects that edit actually being presented to the surface.
///
/// One caveat, stated here rather than left implicit: if several key events
/// arrive before the next `RedrawRequested` (e.g. fast/injected typing
/// outrunning the display's present rate), only the *first* of that batch
/// starts the clock -- so a burst is measured as one input-to-photon latency
/// for the whole batch, not N separate ones. That's the right behavior for
/// what a user actually perceives (time until the screen catches up), but it
/// means sample count can be lower than keystroke count under load.
pub struct LatencyTracker {
    samples: VecDeque<f64>,
    capacity: usize,
    pending_since: Option<Instant>,
    /// Monotonically increasing count of samples ever recorded -- unlike
    /// `samples.len()`, this doesn't stop growing once the ring buffer fills,
    /// so callers can use it to trigger "every N *new* samples" logic (e.g. a
    /// periodic report) for runs longer than `capacity`.
    total_recorded: usize,
}

impl LatencyTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
            pending_since: None,
            total_recorded: 0,
        }
    }

    /// Call when a `KeyboardInput` event just produced a real document edit.
    pub fn note_key_event(&mut self) {
        if self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
    }

    /// Call right after a frame reflecting the pending edit(s) is presented.
    pub fn note_frame_presented(&mut self) {
        if let Some(t0) = self.pending_since.take() {
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            if self.samples.len() == self.capacity {
                self.samples.pop_front();
            }
            self.samples.push_back(elapsed_ms);
            self.total_recorded += 1;
        }
    }

    pub fn total_recorded(&self) -> usize {
        self.total_recorded
    }

    pub fn report(&self, label: &str) {
        if self.samples.is_empty() {
            println!("  {label:<28} (no samples yet)");
            return;
        }
        percentiles(self.samples.iter().copied().collect(), label);
    }
}
