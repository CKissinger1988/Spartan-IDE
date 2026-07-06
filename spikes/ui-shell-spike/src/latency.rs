/// Verbatim port of `rope-spike`'s `percentiles()` (via `render-spike`'s own
/// copy, §47.1/§47.9) so every spike in this workspace reports latency in
/// the same, directly-comparable format.
pub fn percentiles(mut v: Vec<f64>, label: &str) {
    if v.is_empty() {
        println!("  {label:<28} (no samples yet)");
        return;
    }
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
