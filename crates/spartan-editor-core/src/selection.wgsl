struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    // Color travels per-vertex now, not hardcoded here (§75.23) -- three
    // real callers (text selection, the active-tab highlight, the
    // unsaved-changes modal's dim overlay) want three different colors, and
    // all three are already linear-space pre-converted by the Rust side
    // (`selection::ACCENT_HIGHLIGHT`, `main.rs`'s `MODAL_DIM_COLOR`), the
    // same reasoning `cursor.wgsl`'s solid caret color used when the color
    // was still hardcoded here.
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
