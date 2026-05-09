//! wgpu pipeline + atlas upload + draw.

use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::cache::GlyphCache;
use crate::vertex::TextVertex;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    matrix: [[f32; 4]; 4],
    viewport: [f32; 4],
}

/// One uploaded glyph atlas (curve + band textures) bound as group 1.
pub struct TextAtlas {
    #[allow(dead_code)]
    curve_tex: wgpu::Texture,
    #[allow(dead_code)]
    band_tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

impl TextAtlas {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        cache: &GlyphCache,
    ) -> Self {
        let (cw, ch) = cache.curve_size();
        let (bw, bh) = cache.band_size();

        let mut curve_padded = vec![[0.0f32; 4]; (cw * ch) as usize];
        for (i, t) in cache.curve_data().iter().enumerate() {
            if i < curve_padded.len() { curve_padded[i] = *t; }
        }
        let curve_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text curve texture"),
            size: wgpu::Extent3d { width: cw, height: ch, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &curve_tex, mip_level: 0,
                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&curve_padded),
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(cw * 16), rows_per_image: Some(ch) },
            wgpu::Extent3d { width: cw, height: ch, depth_or_array_layers: 1 },
        );

        let mut band_padded = vec![[0u32; 4]; (bw * bh) as usize];
        for (i, t) in cache.band_data().iter().enumerate() {
            if i < band_padded.len() { band_padded[i] = *t; }
        }
        let band_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text band texture"),
            size: wgpu::Extent3d { width: bw, height: bh, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &band_tex, mip_level: 0,
                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&band_padded),
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(bw * 16), rows_per_image: Some(bh) },
            wgpu::Extent3d { width: bw, height: bh, depth_or_array_layers: 1 },
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&curve_tex.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&band_tex.create_view(&Default::default())),
                },
            ],
            label: Some("text atlas bind group"),
        });

        Self { curve_tex, band_tex, bind_group }
    }
}

/// Renders prepared vertex buffers against an atlas.
pub struct TextRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    pub atlas_layout: wgpu::BindGroupLayout,
}

impl TextRenderer {
    pub fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some("text uniform layout"),
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
            label: Some("text atlas layout"),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text uniform buffer"),
            contents: bytemuck::bytes_of(&Params {
                matrix: Mat4::IDENTITY.to_cols_array_2d(),
                viewport: [config.width as f32, config.height as f32, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
            label: Some("text uniform bind group"),
        });

        let vert = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text vertex shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vertex.wgsl").into()),
        });
        let frag = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text fragment shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pixel.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &atlas_layout],
            push_constant_ranges: &[],
        });

        let attrs = [
            wgpu::VertexAttribute { offset: 0,  shader_location: 0, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 16, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 32, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 48, shader_location: 3, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 64, shader_location: 4, format: wgpu::VertexFormat::Float32x4 },
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vert, entry_point: "main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TextVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag, entry_point: "main",
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
            uniform_buffer,
            uniform_bind_group,
            atlas_layout,
        }
    }

    /// Build a vertex buffer from a vertex slice.
    pub fn build_vertices(device: &wgpu::Device, vertices: &[TextVertex]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text vertex buffer"),
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
        viewport: (u32, u32),
        clear: Option<wgpu::Color>,
    ) {
        let params = Params {
            matrix: matrix.to_cols_array_2d(),
            viewport: [viewport.0 as f32, viewport.1 as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&params));

        let load_op = match clear {
            Some(c) => wgpu::LoadOp::Clear(c),
            None => wgpu::LoadOp::Load,
        };
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view, resolve_target: None,
                ops: wgpu::Operations { load: load_op, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if vertex_count > 0 {
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.uniform_bind_group, &[]);
            rpass.set_bind_group(1, &atlas.bind_group, &[]);
            rpass.set_vertex_buffer(0, vertex_buffer.slice(..));
            rpass.draw(0..vertex_count, 0..1);
        }
    }
}
