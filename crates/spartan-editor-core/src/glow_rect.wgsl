// Real signed-distance-field rounded-rect + soft accent glow (§75.55,
// user-requested "more futuristic" visual pass). `selection.wgsl` only ever
// draws flat, sharp-edged quads -- correct for text-selection highlights and
// modal dim overlays, but incapable of the rounded, glowing chrome a
// "futuristic" IDE aesthetic needs. This is a second, sibling shader rather
// than a modification of `selection.wgsl`, since selection quads must stay
// perfectly sharp-edged (a rounded selection highlight around a run of
// glyphs would look wrong) while this one is for standalone chrome elements
// (buttons, pills, cards) that want a real rounded + optionally glowing
// treatment.

struct VertexInput {
    @location(0) position: vec2<f32>,
    // Pixel-space offset of this vertex from the rect's own center --
    // *not* the expanded (glow-padded) quad's center, so the SDF below
    // always measures distance to the real, unpadded rounded-rect edge
    // even though the rasterized quad itself may extend further out to
    // leave room for the glow halo to actually render.
    @location(1) local: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) color: vec4<f32>,
    // 0.0 = no glow (plain rounded-rect fill); >0.0 = soft outward glow
    // strength, using `color`'s own RGB as the glow tint so a single
    // per-vertex color drives both the fill and its halo -- no separate
    // glow-color attribute needed for this pass's real use cases (an
    // accent-colored glow always matches the accent-colored fill).
    @location(5) glow_strength: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) radius: f32,
    @location(3) color: vec4<f32>,
    @location(4) glow_strength: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.local = in.local;
    out.half_size = in.half_size;
    out.radius = in.radius;
    out.color = in.color;
    out.glow_strength = in.glow_strength;
    return out;
}

// Inigo Quilez's standard rounded-box SDF: negative inside the shape,
// positive outside, zero exactly on the rounded edge -- the real, well-
// established formula this technique is built on, not a novel derivation.
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Real glow falloff radius in pixels -- how far the soft halo reaches
// beyond the shape's own edge. Matches `glow_rect.rs`'s own `GLOW_PADDING_PX`
// (the quad must be rasterized at least this far out, or the glow is
// silently clipped by the quad's own geometry before the shader ever runs).
const GLOW_RANGE_PX: f32 = 14.0;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = sd_rounded_box(in.local, in.half_size, in.radius);

    // Antialiased fill: `fwidth(d)` is the real per-pixel screen-space rate
    // of change of the signed distance, so the edge softens by exactly one
    // real screen pixel regardless of zoom/DPI, rather than a fixed
    // hand-tuned constant that would look wrong at a different pixel density.
    let aa = max(fwidth(d), 0.0001);
    let fill_alpha = 1.0 - smoothstep(-aa, aa, d);

    var out_color = in.color.rgb;
    var out_alpha = in.color.a * fill_alpha;

    if (in.glow_strength > 0.0) {
        // Soft outward falloff for the region *outside* the shape
        // (`max(d, 0.0)`, so pixels inside the shape don't get an extra
        // glow contribution on top of their own fill) using a real
        // Gaussian-shaped decay, not a linear ramp -- reads as a genuine
        // soft glow rather than a hard-edged secondary rectangle.
        let outside_d = max(d, 0.0);
        let sigma = GLOW_RANGE_PX * 0.5;
        let glow_alpha = in.glow_strength * in.color.a
            * exp(-(outside_d * outside_d) / (2.0 * sigma * sigma));
        out_alpha = max(out_alpha, glow_alpha);
    }

    if (out_alpha <= 0.001) {
        discard;
    }
    return vec4<f32>(out_color, out_alpha);
}
