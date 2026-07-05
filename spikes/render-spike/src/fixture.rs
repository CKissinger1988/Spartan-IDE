/// Verbatim port of `rope-spike`'s `synthetic_file()` (§47.1) -- identical
/// line shape and content, so this spike's and the CPU-half spike's reports
/// measure the exact same corpus, not just similarly-sized ones.
pub fn synthetic_file(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 40);
    for i in 0..lines {
        s.push_str(&format!(
            "fn function_{i}(a: u32, b: u32) -> u32 {{ a.wrapping_add(b) * {i} }}\n"
        ));
    }
    s
}
