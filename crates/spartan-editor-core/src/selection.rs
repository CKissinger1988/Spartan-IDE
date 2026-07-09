use crate::cursor::ScreenSize;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct SelectionVertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// The exact rust/terracotta accent color (linear-space pre-converted, same
/// reasoning as `cursor.wgsl`'s solid caret color) text selection and the
/// active-tab highlight both use -- kept as a shared constant now that a
/// third, differently-colored real caller (the unsaved-changes modal's dim
/// overlay, §75.23) exists, rather than leaving the color hardcoded
/// per-vertex at every call site.
pub const ACCENT_HIGHLIGHT: [f32; 4] = [0.5524, 0.0563, 0.0242, 0.35];

/// The same rust/terracotta accent at full opacity (no `0.35` translucency)
/// -- the "Spartan Coding futuristic" half of §75.54's user-requested visual
/// pass, real and buildable within this renderer's actual capability today
/// (solid-color quads only, no gradient/glow shader). A plain flat
/// Antigravity-style highlight alone reads as pure minimalism; a thin,
/// solid, saturated accent strip under the active tab is this renderer's
/// real, honest version of a "signature glow" -- sharp and deliberate
/// rather than soft-diffused, which would need real shader work this pass
/// doesn't attempt.
pub const ACCENT_SOLID: [f32; 4] = [0.5524, 0.0563, 0.0242, 1.0];

/// Real pixel thickness of the active-tab accent underline strip below --
/// thin enough to read as a deliberate accent line, not a thick bar,
/// matching this project's own established "declutter" discipline
/// (§36.4.10) for any new chrome.
pub const ACCENT_UNDERLINE_PX: f32 = 2.0;

/// Pixel-space rect for one quad (top-left `x, y`, given `width`/`height`,
/// linear-space `color`) -- originally one per visual line a text selection
/// spans (a multi-line selection isn't a single rectangle the way the caret
/// is), now also reused for the active-tab highlight and the unsaved-changes
/// modal's dim overlay (§75.23), each with their own color.
pub struct SelectionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: [f32; 4],
}

const INITIAL_CAPACITY_QUADS: usize = 64;

/// Real text-selection highlight rendering (§75.18) -- a sibling to
/// `cursor::CursorRenderer`, deliberately its own type rather than
/// generalizing that one to "N quads": the caret is always exactly one
/// solid, opaque quad recomputed every frame, while a selection needs a
/// *variable* count (0 when nothing is selected, one per visual line
/// otherwise) of semi-transparent quads that must render *before* the glyph
/// pass so text stays legible on top of the highlight -- the opposite
/// ordering from the caret, which renders on top of the text. Its own
/// pipeline/shader (`selection.wgsl`) for the same reason `cursor.rs` has
/// its own: a plain filled quad has no reason to go through glyphon's
/// text-specific shader. Color is a per-vertex attribute, not baked into the
/// shader, since three real call sites now want three different colors
/// (text selection and the active-tab highlight share `ACCENT_HIGHLIGHT`;
/// the unsaved-changes modal's dim overlay, §75.23, does not) -- one
/// generic renderer rather than near-duplicate pipelines per color.
pub struct SelectionRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_buffer_capacity_quads: usize,
    quad_count: usize,
}

impl SelectionRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("selection shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("selection.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("selection pipeline layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("selection pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SelectionVertex>() as wgpu::BufferAddress,
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
            label: Some("selection vertex buffer"),
            size: (std::mem::size_of::<SelectionVertex>() * 6 * capacity_quads)
                as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Recomputes every selection-highlight quad from real pixel-space rects
    /// and uploads them, growing the vertex buffer (by doubling, via
    /// `next_power_of_two`) if the caller ever needs more quads than it
    /// currently holds -- real, not hypothetical: a large selection across a
    /// full-height viewport could plausibly exceed the initial capacity.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rects: &[SelectionRect],
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
            let left = to_ndc_x(rect.x);
            let right = to_ndc_x(rect.x + rect.width);
            let top = to_ndc_y(rect.y);
            let bottom = to_ndc_y(rect.y + rect.height);
            let color = rect.color;
            vertices.extend_from_slice(&[
                SelectionVertex {
                    position: [left, top],
                    color,
                },
                SelectionVertex {
                    position: [left, bottom],
                    color,
                },
                SelectionVertex {
                    position: [right, top],
                    color,
                },
                SelectionVertex {
                    position: [right, top],
                    color,
                },
                SelectionVertex {
                    position: [left, bottom],
                    color,
                },
                SelectionVertex {
                    position: [right, bottom],
                    color,
                },
            ]);
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
