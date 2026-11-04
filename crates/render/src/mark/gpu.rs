//! The compositor side of the mark: two full-screen draws on the finished
//! frame, one adding the positive half of the pattern, one subtracting the
//! negative half (see `mark.wgsl` for why two).
use super::{pattern_halves, PAT_H, PAT_W};
use std::cell::RefCell;

pub struct MarkPass {
    add: wgpu::RenderPipeline,
    sub: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Present while a pattern is set; the texture lives as long as the bind
    /// group that holds its view.
    bind: RefCell<Option<wgpu::BindGroup>>,
}

impl MarkPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> MarkPass {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mark-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../mark.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mark-layout"),
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
        let pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mark-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, op: wgpu::BlendOperation| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mark-pipeline"),
                layout: Some(&pl_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: op,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::Zero,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = pipeline("fs_add", wgpu::BlendOperation::Add);
        let sub = pipeline("fs_sub", wgpu::BlendOperation::ReverseSubtract);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mark-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        MarkPass { add, sub, layout, sampler, bind: RefCell::new(None) }
    }

    /// Uploads the pattern (or clears it with `None`).
    pub fn set(&self, device: &wgpu::Device, queue: &wgpu::Queue, pattern: Option<&[f32]>) {
        let Some(p) = pattern else {
            *self.bind.borrow_mut() = None;
            return;
        };
        assert_eq!(p.len(), PAT_W * PAT_H, "a pattern is PAT_W by PAT_H values");
        let size = wgpu::Extent3d { width: PAT_W as u32, height: PAT_H as u32, depth_or_array_layers: 1 };
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mark-pattern"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pattern_halves(p),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PAT_W as u32 * 2),
                rows_per_image: Some(PAT_H as u32),
            },
            size,
        );
        let view = tex.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mark-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        *self.bind.borrow_mut() = Some(bind);
    }

    pub fn enabled(&self) -> bool {
        self.bind.borrow().is_some()
    }

    /// The two draws on the finished frame; nothing happens without a pattern.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let bind = self.bind.borrow();
        let Some(bind) = bind.as_ref() else { return };
        for pipeline in [&self.add, &self.sub] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mark"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
