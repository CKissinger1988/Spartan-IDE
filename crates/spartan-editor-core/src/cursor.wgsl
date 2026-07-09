struct VertexInput {
    @location(0) position: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
    return vec4<f32>(in.position, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Spartan's accent rust/terracotta hue (#C4432B, per docs/architecture-spec.md
    // §50.3 and mobile/src/theme.ts) rather than a generic white caret, matching
    // the same palette identity the rest of the project already uses (see
    // src/theme.rs for this crate's other real color tokens). Values are
    // sRGB(0.769, 0.263, 0.169) pre-converted to linear: this fragment writes
    // to an sRGB surface, which gamma-encodes on write, so a perceptual value
    // passed straight through renders visibly lighter than intended.
    return vec4<f32>(0.5524, 0.0563, 0.0242, 1.0);
}
