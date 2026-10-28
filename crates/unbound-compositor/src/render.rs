use smithay::utils::{Physical, Size};
use unbound_toolkit::{Gpu, GpuError};
use wgpu::util::DeviceExt;

use crate::state::{Frame, Headless};

pub const BACKGROUND: [u8; 3] = [0x20, 0x28, 0x30];

const SHADER: &str = r"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texel: vec2<f32>,
}

@vertex
fn vertex(@location(0) position: vec2<f32>, @location(1) texel: vec2<f32>) -> VertexOutput {
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), texel);
}

@group(0) @binding(0) var frame: texture_2d<f32>;

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = min(vec2<u32>(input.texel), textureDimensions(frame) - 1u);
    return textureLoad(frame, texel, 0);
}
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub size: Size<i32, Physical>,
    pub rgba: Vec<u8>,
}

pub struct Renderer {
    gpu: Gpu,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Renderer {
    pub fn new() -> Result<Self, GpuError> {
        let gpu = Gpu::new()?;
        let device = &gpu.device;

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
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
            label: Some("compose"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compose"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compose"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
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

        Ok(Self {
            gpu,
            layout,
            pipeline,
        })
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }

    pub fn render(&self, compositor: &Headless) -> Result<Snapshot, GpuError> {
        let size = compositor.output_size();
        let extent = extent(size);
        let target = self.gpu.render_target(extent.width, extent.height);
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let quads: Vec<_> = compositor
            .windows()
            .iter()
            .filter_map(|window| Some((window.frame()?, window.location())))
            .map(|(frame, location)| self.upload(frame, location, size))
            .collect();

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("compose"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(background()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (bind_group, vertices) in &quads {
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.draw(0..6, 0..1);
            }
        }

        let rgba = self.gpu.finish_and_read(encoder, &target)?;
        Ok(Snapshot { size, rgba })
    }

    fn upload(
        &self,
        frame: &Frame,
        location: smithay::utils::Point<i32, Physical>,
        output: Size<i32, Physical>,
    ) -> (wgpu::BindGroup, wgpu::Buffer) {
        let extent = extent(frame.size);
        let texture = self.gpu.device.create_texture_with_data(
            &self.gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("window"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &frame.bgra_premultiplied,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("window"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });

        let to_clip_x = |x: i32| 2.0 * x as f32 / output.w as f32 - 1.0;
        let to_clip_y = |y: i32| 1.0 - 2.0 * y as f32 / output.h as f32;
        let (left, top) = (to_clip_x(location.x), to_clip_y(location.y));
        let right = to_clip_x(location.x + frame.size.w);
        let bottom = to_clip_y(location.y + frame.size.h);
        let (width, height) = (frame.size.w as f32, frame.size.h as f32);
        let corners = [
            [left, top, 0.0, 0.0],
            [right, top, width, 0.0],
            [left, bottom, 0.0, height],
            [right, top, width, 0.0],
            [right, bottom, width, height],
            [left, bottom, 0.0, height],
        ];
        let bytes: Vec<u8> = corners
            .iter()
            .flatten()
            .flat_map(|value| value.to_ne_bytes())
            .collect();
        let vertices = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("window"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
        (bind_group, vertices)
    }
}

fn extent(size: Size<i32, Physical>) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.w.unsigned_abs(),
        height: size.h.unsigned_abs(),
        depth_or_array_layers: 1,
    }
}

fn background() -> wgpu::Color {
    let [r, g, b] = BACKGROUND.map(|channel| f64::from(channel) / 255.0);
    wgpu::Color { r, g, b, a: 1.0 }
}
