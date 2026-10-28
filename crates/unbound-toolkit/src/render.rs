use std::collections::HashMap;

use ab_glyph::GlyphId;
use wgpu::util::DeviceExt;

use crate::gpu::{Gpu, GpuError};
use crate::scene::{Item, Rect, Scene};
use crate::text::{Coverage, Font};
use crate::tokens::Color;

const ATLAS_WIDTH: u32 = 1024;

const SHADER: &str = r"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texel: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex(
    @location(0) position: vec2<f32>,
    @location(1) texel: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOutput {
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), texel, color);
}

@group(0) @binding(0) var atlas: texture_2d<f32>;

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = min(vec2<u32>(input.texel), textureDimensions(atlas) - 1u);
    return input.color * textureLoad(atlas, texel, 0).r;
}
";

pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Renderer {
    gpu: Gpu,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

struct AtlasEntry {
    coverage: Coverage,
    origin: (u32, u32),
}

impl Renderer {
    pub fn new(gpu: Gpu) -> Self {
        let device = &gpu.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("toolkit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("toolkit"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("toolkit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: Gpu::TARGET_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            gpu,
            layout,
            pipeline,
        }
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }

    pub fn render(&self, scene: &Scene, font: &Font) -> Result<Snapshot, GpuError> {
        let mut glyphs: HashMap<(GlyphId, u32), AtlasEntry> = HashMap::new();
        let mut shelf = Shelf::default();
        for item in &scene.items {
            if let Item::Text { text, size, .. } = item {
                for glyph in font.layout(text, *size, 0, 0) {
                    let key = (glyph.id, size.to_bits());
                    if glyphs.contains_key(&key) {
                        continue;
                    }
                    if let Some(coverage) = font.coverage(glyph.id, *size) {
                        let origin = shelf.place(coverage.width, coverage.height);
                        glyphs.insert(key, AtlasEntry { coverage, origin });
                    }
                }
            }
        }

        let atlas_height = shelf.height().max(1);
        let mut atlas = vec![0u8; (ATLAS_WIDTH * atlas_height) as usize];
        atlas[0] = u8::MAX;
        for entry in glyphs.values() {
            let (left, top) = entry.origin;
            for (row, source) in entry
                .coverage
                .alpha
                .chunks(entry.coverage.width.max(1) as usize)
                .enumerate()
            {
                let start = ((top + row as u32) * ATLAS_WIDTH + left) as usize;
                atlas[start..start + source.len()].copy_from_slice(source);
            }
        }

        let mut vertices = Vec::new();
        let canvas = (scene.width as f32, scene.height as f32);
        for item in &scene.items {
            match item {
                Item::Fill { rect, color } => {
                    push_quad(&mut vertices, canvas, *rect, (0, 0, 0, 0), *color);
                }
                Item::Text {
                    x,
                    baseline,
                    size,
                    color,
                    text,
                } => {
                    for glyph in font.layout(text, *size, *x, *baseline) {
                        let Some(entry) = glyphs.get(&(glyph.id, size.to_bits())) else {
                            continue;
                        };
                        let coverage = &entry.coverage;
                        let rect = Rect {
                            x: glyph.x + coverage.left,
                            y: glyph.baseline + coverage.top,
                            width: coverage.width,
                            height: coverage.height,
                        };
                        let (u, v) = entry.origin;
                        let texels = (u, v, coverage.width, coverage.height);
                        push_quad(&mut vertices, canvas, rect, texels, *color);
                    }
                }
            }
        }

        let device = &self.gpu.device;
        let atlas_texture = device.create_texture_with_data(
            &self.gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("atlas"),
                size: wgpu::Extent3d {
                    width: ATLAS_WIDTH,
                    height: atlas_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &atlas,
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            }],
        });
        let target = self.gpu.render_target(scene.width, scene.height);
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("toolkit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color(scene.background)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !vertices.is_empty() {
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("quads"),
                    contents: &vertices,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..(vertices.len() / 32) as u32, 0..1);
            }
        }
        let rgba = self.gpu.finish_and_read(encoder, &target)?;
        Ok(Snapshot {
            width: scene.width,
            height: scene.height,
            rgba,
        })
    }
}

#[derive(Default)]
struct Shelf {
    x: u32,
    y: u32,
    row_height: u32,
}

impl Shelf {
    fn place(&mut self, width: u32, height: u32) -> (u32, u32) {
        if self.x == 0 && self.y == 0 && self.row_height == 0 {
            self.x = 1;
            self.row_height = 1;
        }
        if self.x + width > ATLAS_WIDTH {
            self.y += self.row_height;
            self.x = 0;
            self.row_height = 0;
        }
        let origin = (self.x, self.y);
        self.x += width;
        self.row_height = self.row_height.max(height);
        origin
    }

    fn height(&self) -> u32 {
        self.y + self.row_height
    }
}

fn clear_color(color: Color) -> wgpu::Color {
    let channel = |value: u8| f64::from(value) / 255.0;
    wgpu::Color {
        r: channel(color.r),
        g: channel(color.g),
        b: channel(color.b),
        a: 1.0,
    }
}

fn push_quad(
    vertices: &mut Vec<u8>,
    (canvas_width, canvas_height): (f32, f32),
    rect: Rect,
    (u, v, texel_width, texel_height): (u32, u32, u32, u32),
    color: Color,
) {
    let left = 2.0 * rect.x as f32 / canvas_width - 1.0;
    let right = 2.0 * (rect.x as f32 + rect.width as f32) / canvas_width - 1.0;
    let top = 1.0 - 2.0 * rect.y as f32 / canvas_height;
    let bottom = 1.0 - 2.0 * (rect.y as f32 + rect.height as f32) / canvas_height;
    let (u0, v0) = (u as f32, v as f32);
    let (u1, v1) = (u0 + texel_width as f32, v0 + texel_height as f32);
    let rgba = [color.r, color.g, color.b, u8::MAX].map(|value| f32::from(value) / 255.0);
    let corners = [
        [left, top, u0, v0],
        [right, top, u1, v0],
        [left, bottom, u0, v1],
        [right, top, u1, v0],
        [right, bottom, u1, v1],
        [left, bottom, u0, v1],
    ];
    for corner in corners {
        for value in corner.into_iter().chain(rgba) {
            vertices.extend_from_slice(&value.to_ne_bytes());
        }
    }
}
