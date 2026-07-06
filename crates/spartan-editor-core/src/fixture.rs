/// Verbatim port of `rope-spike`'s `synthetic_file()` (via `render-spike`'s
/// own copy, §47.1/§47.9) -- identical line shape and content, so every
/// spike/crate in this workspace measures the exact same corpus.
pub fn synthetic_file(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 40);
    for i in 0..lines {
        s.push_str(&format!(
            "fn function_{i}(a: u32, b: u32) -> u32 {{ a.wrapping_add(b) * {i} }}\n"
        ));
    }
    s
}
