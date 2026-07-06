use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CursorVertex {
    position: [f32; 2],
}

/// Caret width in pixels -- a thin bar, not a block, matching a conventional
/// text-editor caret rather than a terminal-style block cursor.
pub const CURSOR_WIDTH_PX: f32 = 2.0;

/// Pixel-space rect for the caret quad (top-left `x, y`, given `width`/`height`).
pub struct CursorRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Window size in pixels, for converting `CursorRect` into NDC.
pub struct ScreenSize {
    pub width: f32,
    pub height: f32,
}

/// Promoted from `spikes/render-spike/src/cursor.rs` (§39.1, §47.9) verbatim.
/// Draws a solid caret rect at a pixel position recomputed every frame from
/// the `Document`'s cursor char-index (see `text::TextState::cursor_pixel_pos`).
/// Deliberately its own pipeline/vertex buffer, separate from glyphon's atlas
/// pipeline -- the caret is a plain filled quad, not a glyph, so there's no
/// reason to route it through glyphon's text-specific shader.
pub struct CursorRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
}

impl CursorRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cursor shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cursor.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cursor pipeline layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cursor pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CursorVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    }],
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

        // Room for one quad (2 triangles, 6 vertices); rewritten in place by
        // `update()` every frame rather than recreated.
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cursor vertex buffer"),
            size: (std::mem::size_of::<CursorVertex>() * 6) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertex_buffer,
        }
    }

    /// Recomputes the caret quad from a pixel-space rect and uploads it.
    /// `screen` converts pixel space (origin top-left, Y-down) to wgpu's NDC
    /// (origin center, Y-up) -- the same conversion the rest of the pipeline
    /// leaves to glyphon internally, done by hand here since this quad
    /// bypasses glyphon.
    pub fn update(&self, queue: &wgpu::Queue, rect: CursorRect, screen: ScreenSize) {
        let to_ndc_x = |px: f32| (px / screen.width) * 2.0 - 1.0;
        let to_ndc_y = |px: f32| 1.0 - (px / screen.height) * 2.0;

        let left = to_ndc_x(rect.x);
        let right = to_ndc_x(rect.x + rect.width);
        let top = to_ndc_y(rect.y);
        let bottom = to_ndc_y(rect.y + rect.height);

        let vertices = [
            CursorVertex {
                position: [left, top],
            },
            CursorVertex {
                position: [left, bottom],
            },
            CursorVertex {
                position: [right, top],
            },
            CursorVertex {
                position: [right, top],
            },
            CursorVertex {
                position: [left, bottom],
            },
            CursorVertex {
                position: [right, bottom],
            },
        ];

        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    pub fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }
}
