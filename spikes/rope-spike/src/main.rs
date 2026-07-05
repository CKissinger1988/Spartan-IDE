use rand::Rng;
use ropey::Rope;
use std::time::Instant;

fn synthetic_file(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 40);
    for i in 0..lines {
        s.push_str(&format!(
            "fn function_{i}(a: u32, b: u32) -> u32 {{ a.wrapping_add(b) * {i} }}\n"
        ));
    }
    s
}

fn percentiles(mut v: Vec<f64>, label: &str) {
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

fn bench_rope_typing(rope: &mut Rope, iters: usize) -> Vec<f64> {
    let mut rng = rand::thread_rng();
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let pos = rng.gen_range(0..rope.len_chars().max(1));
        let t0 = Instant::now();
        rope.insert(pos, "x");
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times
}

fn bench_flat_typing(buf: &mut String, iters: usize) -> Vec<f64> {
    let mut rng = rand::thread_rng();
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let byte_pos = rng.gen_range(0..buf.len().max(1));
        let mut p = byte_pos;
        while p > 0 && !buf.is_char_boundary(p) {
            p -= 1;
        }
        let t0 = Instant::now();
        buf.insert(p, 'x');
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times
}

fn bench_rope_paste(rope: &mut Rope, block: &str, iters: usize) -> Vec<f64> {
    let mut rng = rand::thread_rng();
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let pos = rng.gen_range(0..rope.len_chars().max(1));
        let t0 = Instant::now();
        rope.insert(pos, block);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times
}

fn bench_snapshot(rope: &Rope, iters: usize) -> Vec<f64> {
    let mut times = Vec::with_capacity(iters);
    let mut keep_alive = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t0 = Instant::now();
        let snap = rope.clone();
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
        keep_alive.push(snap);
    }
    times
}

fn main() {
    let lines = 50_000;
    println!("=== Spike 0.1 -- Rope vs Flat Buffer, CPU-only ===");
    println!(
        "Environment: {} logical core(s), release build, NO GPU/renderer involved.\n\
         This measures ONLY the text-buffer half of section 2.1/39.1. It does NOT measure\n\
         glyph rendering, damage-region rasterization, or true input-to-photon latency --\n\
         that half requires a GPU + display, unavailable in this sandbox.\n",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );

    let content = synthetic_file(lines);
    println!(
        "Synthetic file: {lines} lines, {} bytes ({:.2} MB)\n",
        content.len(),
        content.len() as f64 / 1_000_000.0
    );

    let paste_block: String = (0..2000).map(|_| 'y').collect();
    let iters = 2000;

    let mut rope = Rope::from_str(&content);
    let rope_typing = bench_rope_typing(&mut rope, iters);
    let rope_paste = bench_rope_paste(&mut rope, &paste_block, 200);
    let rope_snapshot = bench_snapshot(&rope, 500);

    let mut flat = content.clone();
    let flat_typing = bench_flat_typing(&mut flat, iters);

    println!("-- Typing (single-char insert, {iters} ops) --");
    percentiles(rope_typing.clone(), "rope");
    percentiles(flat_typing.clone(), "flat String");

    println!("\n-- Large paste (2KB block, 200 ops) --");
    percentiles(rope_paste, "rope");

    println!("\n-- Snapshot / clone (branching-undo proxy, 500 ops) --");
    percentiles(rope_snapshot, "rope.clone()");

    let p99_of = |v: &Vec<f64>| -> f64 {
        let mut v2 = v.clone();
        v2.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v2[((v2.len() as f64 - 1.0) * 0.99) as usize]
    };
    let rope_p99 = p99_of(&rope_typing);
    let flat_p99 = p99_of(&flat_typing);

    println!("\n=== Verdict (data-structure layer only) ===");
    println!(
        "rope p99 = {:.4}ms vs flat-buffer p99 = {:.4}ms  ({:.1}x)",
        rope_p99,
        flat_p99,
        flat_p99 / rope_p99.max(0.0001)
    );
    if rope_p99 < 1.0 {
        println!(
            "PASS (data-structure layer): rope p99 insert cost leaves ample headroom under\n\
             the 5ms full-pipeline budget (section 39.1) for the rendering half to consume on\n\
             real GPU hardware. This does NOT by itself close Spike 0.1 -- the GPU/glyph half\n\
             must still be run on real hardware with a display."
        );
    } else {
        println!(
            "WATCH: rope p99 already consumes a large fraction of the 5ms full-pipeline budget\n\
             before rendering is even added. Re-profile before greenlighting Spike 0.1."
        );
    }
}
