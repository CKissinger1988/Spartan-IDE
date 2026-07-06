use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct PanelVertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// An RGBA color in linear space (already gamma-corrected -- see
/// `srgb_to_linear` in `main.rs`), a pixel-space rect, and a screen size to
/// convert between them. Used for the left rail / auxiliary pane / mode-
/// fade panels -- the native-rendered chrome around the WebView-hosted
/// center stage.
pub struct PanelRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: [f32; 4],
}

/// Draws a small, fixed number of solid-colored rects with one pipeline and
/// one vertex buffer -- deliberately not a general-purpose 2D renderer,
/// just enough to stand in for the three-column skeleton's native chrome
/// panels while the center stage is a real embedded WebView (see
/// `webview_bridge.rs`).
pub struct PanelRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    capacity_rects: usize,
}

impl PanelRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        capacity_rects: usize,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("panel shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("panel.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("panel pipeline layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("panel pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PanelVertex>() as wgpu::BufferAddress,
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
                            format: wgpu::VertexFormat::Float32x4,
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

        // 6 vertices (2 triangles) per rect, rewritten every frame.
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("panel vertex buffer"),
            size: (std::mem::size_of::<PanelVertex>() * 6 * capacity_rects) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertex_buffer,
            capacity_rects,
        }
    }

    /// Uploads up to `capacity_rects` rects (extras are silently dropped --
    /// a spike-level bound, not a general renderer) and returns how many
    /// vertices to draw.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        rects: &[PanelRect],
        screen_w: f32,
        screen_h: f32,
    ) -> u32 {
        let to_ndc_x = |px: f32| (px / screen_w) * 2.0 - 1.0;
        let to_ndc_y = |px: f32| 1.0 - (px / screen_h) * 2.0;

        let mut vertices = Vec::with_capacity(rects.len().min(self.capacity_rects) * 6);
        for rect in rects.iter().take(self.capacity_rects) {
            let left = to_ndc_x(rect.x);
            let right = to_ndc_x(rect.x + rect.width);
            let top = to_ndc_y(rect.y);
            let bottom = to_ndc_y(rect.y + rect.height);
            let c = rect.color;

            vertices.extend_from_slice(&[
                PanelVertex {
                    position: [left, top],
                    color: c,
                },
                PanelVertex {
                    position: [left, bottom],
                    color: c,
                },
                PanelVertex {
                    position: [right, top],
                    color: c,
                },
                PanelVertex {
                    position: [right, top],
                    color: c,
                },
                PanelVertex {
                    position: [left, bottom],
                    color: c,
                },
                PanelVertex {
                    position: [right, bottom],
                    color: c,
                },
            ]);
        }

        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        vertices.len() as u32
    }

    pub fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, vertex_count: u32) {
        if vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}
