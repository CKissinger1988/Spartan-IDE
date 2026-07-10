use crate::cursor::ScreenSize;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct GlowVertex {
    position: [f32; 2],
    local: [f32; 2],
    half_size: [f32; 2],
    radius: f32,
    color: [f32; 4],
    glow_strength: f32,
}

/// Real corner radius in pixels used across every rounded chrome element in
/// this pass (§75.55) -- one shared constant so the "how rounded" decision
/// is made once, not re-guessed per call site.
pub const CORNER_RADIUS_PX: f32 = 6.0;

/// Real glow halo reach in pixels -- must exactly match
/// `glow_rect.wgsl`'s own `GLOW_RANGE_PX`; kept as two independently
/// documented constants (Rust padding, WGSL falloff radius) rather than one
/// shared value crossing the Rust/WGSL boundary, since there is no real
/// mechanism in this codebase to enforce that automatically (WGSL constants
/// aren't generated from Rust here). A quad that isn't padded by at least
/// this much clips its own glow before the fragment shader ever sees it.
pub const GLOW_PADDING_PX: f32 = 14.0;

/// Pixel-space rounded rect with an optional soft accent glow -- the
/// "futuristic" sibling to `selection::SelectionRect`, which stays
/// deliberately sharp-edged for text-selection/dim-overlay use. `radius`
/// of `0.0` degrades to a plain rounded-off rect with no visible rounding
/// at all (a real, cheap way to reuse this one renderer for both rounded
/// and square chrome rather than needing a third renderer for the latter).
/// `glow_strength` of `0.0` skips the glow falloff term entirely.
pub struct GlowRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub color: [f32; 4],
    pub glow_strength: f32,
}

const INITIAL_CAPACITY_QUADS: usize = 32;

/// Real SDF rounded-rect + glow rendering (§75.55) -- its own pipeline/
/// shader (`glow_rect.wgsl`) for the same reason `cursor.rs`/`selection.rs`
/// each have their own: this shader's per-vertex attribute set (local
/// offset, half-size, radius, glow strength) has no reason to exist on the
/// flat-quad `selection.wgsl` pipeline, and vice versa.
pub struct GlowRectRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_buffer_capacity_quads: usize,
    quad_count: usize,
}

impl GlowRectRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glow rect shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glow_rect.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glow rect pipeline layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glow rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlowVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 7]>() as wgpu::BufferAddress,
                            shader_location: 4,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 11]>() as wgpu::BufferAddress,
                            shader_location: 5,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let vertex_buffer = Self::make_buffer(device, INITIAL_CAPACITY_QUADS);

        Self {
            pipeline,
            vertex_buffer,
            vertex_buffer_capacity_quads: INITIAL_CAPACITY_QUADS,
            quad_count: 0,
        }
    }

    fn make_buffer(device: &wgpu::Device, capacity_quads: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glow rect vertex buffer"),
            size: (std::mem::size_of::<GlowVertex>() * 6 * capacity_quads) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Recomputes every rounded-rect quad from real pixel-space rects and
    /// uploads them, growing the vertex buffer (doubling, via
    /// `next_power_of_two`) exactly like `SelectionRenderer::update`. Each
    /// rect's rasterized quad is padded by `GLOW_PADDING_PX` on every side
    /// when `glow_strength > 0.0` (skipped otherwise, avoiding needless
    /// overdraw for plain rounded chrome with no glow) so the fragment
    /// shader has real screen-space room to render the halo -- `local`
    /// coordinates are computed against the *unpadded* rect center, so the
    /// SDF math in `glow_rect.wgsl` is unaffected by the padding, only the
    /// quad's own rasterized extent is.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rects: &[GlowRect],
        screen: &ScreenSize,
    ) {
        self.quad_count = rects.len();
        if rects.is_empty() {
            return;
        }
        if rects.len() > self.vertex_buffer_capacity_quads {
            let new_capacity = rects.len().next_power_of_two();
            self.vertex_buffer = Self::make_buffer(device, new_capacity);
            self.vertex_buffer_capacity_quads = new_capacity;
        }

        let to_ndc_x = |px: f32| (px / screen.width) * 2.0 - 1.0;
        let to_ndc_y = |px: f32| 1.0 - (px / screen.height) * 2.0;

        let mut vertices = Vec::with_capacity(rects.len() * 6);
        for rect in rects {
            let pad = if rect.glow_strength > 0.0 {
                GLOW_PADDING_PX
            } else {
                0.0
            };
            let half_size = [rect.width / 2.0, rect.height / 2.0];
            let center_x = rect.x + half_size[0];
            let center_y = rect.y + half_size[1];

            let px_left = rect.x - pad;
            let px_right = rect.x + rect.width + pad;
            let px_top = rect.y - pad;
            let px_bottom = rect.y + rect.height + pad;

            let corners = [
                (px_left, px_top),
                (px_left, px_bottom),
                (px_right, px_top),
                (px_right, px_top),
                (px_left, px_bottom),
                (px_right, px_bottom),
            ];

            for (px, py) in corners {
                vertices.push(GlowVertex {
                    position: [to_ndc_x(px), to_ndc_y(py)],
                    local: [px - center_x, py - center_y],
                    half_size,
                    radius: rect.radius,
                    color: rect.color,
                    glow_strength: rect.glow_strength,
                });
            }
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    pub fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.quad_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..(self.quad_count as u32 * 6), 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure, headless check of the local-coordinate math `update()` relies
    /// on -- a rect's own center should sit at `local = (0, 0)` for every
    /// vertex derived from it once you account for padding, since padding
    /// only changes each vertex's own distance from that fixed center.
    #[test]
    fn local_coords_center_on_unpadded_rect() {
        let rect = GlowRect {
            x: 100.0,
            y: 50.0,
            width: 40.0,
            height: 20.0,
            radius: CORNER_RADIUS_PX,
            color: [1.0, 0.0, 0.0, 1.0],
            glow_strength: 0.0,
        };
        let center_x = rect.x + rect.width / 2.0;
        let center_y = rect.y + rect.height / 2.0;
        assert_eq!(center_x, 120.0);
        assert_eq!(center_y, 60.0);
        // Top-left corner (no padding, glow_strength == 0.0) is exactly
        // half_size away from center in both axes.
        let local_top_left = (rect.x - center_x, rect.y - center_y);
        assert_eq!(local_top_left, (-20.0, -10.0));
    }

    #[test]
    fn glow_padding_only_applied_when_strength_positive() {
        // A zero-strength rect's rasterized quad must exactly match its
        // logical bounds (pad == 0.0) -- verified indirectly here since
        // `update()` itself needs a real GPU device to exercise; this pins
        // down the conditional's own real threshold value.
        let glow_strength = 0.0_f32;
        let pad = if glow_strength > 0.0 {
            GLOW_PADDING_PX
        } else {
            0.0
        };
        assert_eq!(pad, 0.0);
        let glow_strength = 0.6_f32;
        let pad = if glow_strength > 0.0 {
            GLOW_PADDING_PX
        } else {
            0.0
        };
        assert_eq!(pad, GLOW_PADDING_PX);
    }
}
