struct VertexInput {
    @location(0) position: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> @builtin(position) vec4<f32> {
    return vec4<f32>(in.position, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Spartan's accent rust/terracotta hue at low alpha (same linear-space
    // pre-conversion as cursor.wgsl's own solid caret color, for the same
    // reason: this fragment writes to an sRGB surface) -- a real selection
    // highlight using the project's own accent identity rather than a
    // generic gray/blue placeholder, matching cursor.wgsl's precedent.
    return vec4<f32>(0.5524, 0.0563, 0.0242, 0.35);
}
