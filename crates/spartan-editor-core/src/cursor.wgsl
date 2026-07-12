struct VertexInput {
    @location(0) position: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
    return vec4<f32>(in.position, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Spartan's blue brand accent hue (#2E7DFF, per docs/architecture-spec.md
    // §75.95 and mobile/src/theme.ts) rather than a generic white caret, matching
    // the same palette identity the rest of the project already uses (see
    // src/theme.rs for this crate's other real color tokens). Values are
    // sRGB(0.180, 0.490, 1.000) pre-converted to linear: this fragment writes
    // to an sRGB surface, which gamma-encodes on write, so a perceptual value
    // passed straight through renders visibly lighter than intended.
    return vec4<f32>(0.0273, 0.2051, 1.0000, 1.0);
}
