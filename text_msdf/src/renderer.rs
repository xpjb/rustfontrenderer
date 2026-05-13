//! MSDF wgpu pipeline (single atlas texture + median shader).

use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::atlas::EmbeddedAtlas;
use crate::vertex::TextVertex;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    matrix: [[f32; 4]; 4],
    /// `x` = atlas SDF span in texels, `y`/`z` = atlas size, `w` = `em_to_px`.
    px_meta: [f32; 4],
}

pub struct TextAtlas {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
}

impl TextAtlas {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        atlas: &EmbeddedAtlas,
    ) -> Self {
        let h = &atlas.file.header;
        let extent = wgpu::Extent3d {
            width: h.atlas_w,
            height: h.atlas_h,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text_msdf atlas"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * h.atlas_w),
                rows_per_image: Some(h.atlas_h),
            },
            extent,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text_msdf atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
            label: Some("text_msdf atlas bind group"),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            texture,
            bind_group,
            width: h.atlas_w,
            height: h.atlas_h,
        }
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

pub struct TextRenderer {
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    pub atlas_layout: wgpu::BindGroupLayout,
    distance_range_px: f32,
    em_to_px: f32,
}

impl TextRenderer {
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        distance_range_px: f32,
        em_to_px: u32,
    ) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text_msdf globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text_msdf atlas layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text_msdf globals"),
            contents: bytemuck::bytes_of(&Globals {
                matrix: Mat4::IDENTITY.to_cols_array_2d(),
                px_meta: [distance_range_px, 1.0, 1.0, em_to_px as f32],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
            label: Some("text_msdf globals bind group"),
        });

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text_msdf vs"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vertex.wgsl").into()),
        });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text_msdf fs"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/pixel_full.wgsl")).into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text_msdf pipeline layout"),
            bind_group_layouts: &[&globals_layout, &atlas_layout],
            push_constant_ranges: &[],
        });

        let attrs = [
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 8,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 3,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: 36,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 52,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 68,
                shader_location: 6,
                format: wgpu::VertexFormat::Float32,
            },
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text_msdf pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vert,
                entry_point: "main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TextVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag,
                entry_point: "main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            pipeline,
            globals_buffer,
            globals_bind_group,
            atlas_layout,
            distance_range_px,
            em_to_px: em_to_px as f32,
        }
    }

    pub fn build_vertices(device: &wgpu::Device, vertices: &[TextVertex]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text_msdf vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        atlas: &TextAtlas,
        vertex_buffer: &wgpu::Buffer,
        vertex_count: u32,
        matrix: Mat4,
        _viewport: (u32, u32),
        clear: Option<wgpu::Color>,
    ) {
        let globals = Globals {
            matrix: matrix.to_cols_array_2d(),
            px_meta: [
                self.distance_range_px,
                atlas.width.max(1) as f32,
                atlas.height.max(1) as f32,
                self.em_to_px,
            ],
        };
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        let load_op = match clear {
            Some(c) => wgpu::LoadOp::Clear(c),
            None => wgpu::LoadOp::Load,
        };
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text_msdf pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if vertex_count > 0 {
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.globals_bind_group, &[]);
            rpass.set_bind_group(1, atlas.bind_group(), &[]);
            rpass.set_vertex_buffer(0, vertex_buffer.slice(..));
            rpass.draw(0..vertex_count, 0..1);
        }
    }
}
