//! Headless wgpu renderer: projects note meshes and the cursor with a real
//! perspective camera and reads the frame back as RGBA pixels.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::config::SkinConfig;
use crate::mesh::{Mesh, Vertex};
use crate::scene::SceneParams;
use crate::Error;
use rhythia_formats::{map::Map, rhr::Replay};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    glow_tint: [f32; 4],
    /// x = note corner radius, y = note outline width,
    /// z = border mode (0 full / 1 corners), w = border corner radius.
    params: [f32; 4],
    /// Which imported skin textures are bound: x=note y=border z=cursor.
    tex_flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    kind: f32,
    _pad: [f32; 3],
}

impl Instance {
    fn note(model: Mat4, color: [f32; 4]) -> Self {
        Instance {
            model: model.to_cols_array_2d(),
            color,
            kind: 0.0,
            _pad: [0.0; 3],
        }
    }
    fn dot(model: Mat4, color: [f32; 4]) -> Self {
        Instance {
            model: model.to_cols_array_2d(),
            color,
            kind: 1.0,
            _pad: [0.0; 3],
        }
    }
    fn border(model: Mat4, color: [f32; 4]) -> Self {
        Instance {
            model: model.to_cols_array_2d(),
            color,
            kind: 2.0,
            _pad: [0.0; 3],
        }
    }
    fn solid(model: Mat4, color: [f32; 4]) -> Self {
        Instance {
            model: model.to_cols_array_2d(),
            color,
            kind: 3.0,
            _pad: [0.0; 3],
        }
    }
}

/// A drawable mesh uploaded to the GPU.
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// GPU-resident skin textures (note/border/cursor) for one imported skin,
/// plus flags for which are real vs. dummy. Prepared once via
/// [`Renderer::prepare_skin`] and reused across every frame.
pub struct SkinTextures {
    bind_group: wgpu::BindGroup,
    tex_flags: [f32; 4],
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    pipeline: wgpu::RenderPipeline,
    /// Depth-ignoring variant for the trail/cursor overlay instances.
    pipeline_overlay: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    color_tex: wgpu::Texture,
    depth_view: wgpu::TextureView,
    /// The shared unit quad every shape is drawn on, uploaded once.
    quad: GpuMesh,
    /// Layout for the imported-skin texture bind group (group 1).
    skin_layout: wgpu::BindGroupLayout,
    /// A 1×1 transparent texture used for absent skin textures.
    dummy_tex: wgpu::Texture,
    skin_sampler: wgpu::Sampler,
    /// HUD overlay: pipeline, its bind group (screen size + glyph atlas), and
    /// the CPU-side atlas used to lay text out.
    hud_pipeline: wgpu::RenderPipeline,
    hud_bind_group: wgpu::BindGroup,
    hud_atlas: crate::hud::FontAtlas,
    /// Kept to build results-screen bind groups with real cover textures.
    hud_layout: wgpu::BindGroupLayout,
    hud_sampler: wgpu::Sampler,
    hud_screen_buf: wgpu::Buffer,
    hud_atlas_view: wgpu::TextureView,
    /// Per-frame buffers, created once and rewritten every frame — creating
    /// fresh GPU buffers per frame accumulates allocations over long renders
    /// and eventually stalls the machine.
    inst_buf: wgpu::Buffer,
    hud_vbuf: wgpu::Buffer,
    /// Readback slots so the GPU can render ahead while the CPU reads older
    /// frames (see `submit_frame`/`with_slot_pixels`).
    readback_bufs: [wgpu::Buffer; READBACK_SLOTS],
    readback_submissions: std::cell::RefCell<[Option<wgpu::SubmissionIndex>; READBACK_SLOTS]>,
}

/// Number of in-flight readback buffers; the video loop reads a frame only
/// once `READBACK_SLOTS - 1` newer frames have been submitted behind it.
pub const READBACK_SLOTS: usize = 3;

/// The four ambient background layers (tunnel, chevrons, rays, moving grid)
/// are DELIBERATELY DISABLED (user decision, 2026-07-14): reconstructions
/// from footage never got close enough to the game — the tunnel look was
/// off, rays rendered invisible, the grid showed only its ceiling plane and
/// the chevrons didn't match — and iterating on rarely-used eye candy was
/// stalling the project. The implementations stay below so support can be
/// revived by flipping this constant; recalibrate each against fresh footage
/// (see docs/ROADMAP.md §6) before shipping it.
const AMBIENT_EFFECTS_SUPPORTED: bool = false;

/// Capacity of the persistent instance buffer (instances per frame).
const INSTANCE_CAP: usize = 16 * 1024;
/// Capacity of the persistent HUD vertex buffer (vertices per frame).
const HUD_VERT_CAP: usize = 64 * 1024;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl Renderer {
    pub fn new(width: u32, height: u32, hud_font: Option<&[u8]>) -> Result<Renderer, Error> {
        pollster::block_on(Self::new_async(width, height, hud_font))
    }

    async fn new_async(
        width: u32,
        height: u32,
        hud_font: Option<&[u8]>,
    ) -> Result<Renderer, Error> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|_| Error::NoAdapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("rhythia"),
                ..Default::default()
            })
            .await
            .map_err(|e| Error::Device(e.to_string()))?;

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        // Skin-texture bind group (group 1): 3 textures + 1 sampler.
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let skin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin-layout"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("note-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("note.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout), Some(&skin_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        };
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
                5 => Float32x4, 6 => Float32
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("note-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                // Cull back faces so the thin extruded meshes don't blend
                // their front and back layers into tessellation artifacts.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Same shader without depth write/test: the cursor trail's densely
        // overlapping translucent stamps must all blend — with the depth
        // test on, stamps quantising to the same depth reject each other's
        // overlap and the trail beads up.
        let pipeline_overlay = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_layout), Some(instance_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_tex.create_view(&Default::default());
        let quad = upload_mesh(&device, &Mesh::quad());

        // 1×1 transparent texture stands in for any skin texture a pack
        // doesn't ship; the shader's tex_flags then keep it on the SDF path.
        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy-tex"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            dummy.as_image_copy(),
            &[0, 0, 0, 0],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let skin_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("skin-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        // --- HUD overlay pipeline ---------------------------------------
        let hud_atlas = crate::hud::FontAtlas::new(hud_font);
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hud-atlas"),
            size: wgpu::Extent3d {
                width: hud_atlas.width as u32,
                height: hud_atlas.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            atlas_tex.as_image_copy(),
            &hud_atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(hud_atlas.width as u32),
                rows_per_image: Some(hud_atlas.height as u32),
            },
            wgpu::Extent3d {
                width: hud_atlas.width as u32,
                height: hud_atlas.height as u32,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_tex.create_view(&Default::default());
        // Screen size uniform (fixed for this renderer's dimensions).
        let screen_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &screen_buf,
            0,
            bytemuck::bytes_of(&[width as f32, height as f32, 0.0f32, 0.0f32]),
        );
        let hud_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hud-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let hud_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(3),
                tex_entry(4),
            ],
        });
        let dummy_view = dummy.create_view(&Default::default());
        let hud_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud-bind"),
            layout: &hud_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: screen_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&hud_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&dummy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&dummy_view),
                },
            ],
        });
        let hud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hud.wgsl").into()),
        });
        let hud_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud-pipeline-layout"),
            bind_group_layouts: &[Some(&hud_layout)],
            immediate_size: 0,
        });
        let hud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud-pipeline"),
            layout: Some(&hud_pl_layout),
            vertex: wgpu::VertexState {
                module: &hud_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::hud::HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &hud_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (INSTANCE_CAP * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud-verts"),
            size: (HUD_VERT_CAP * std::mem::size_of::<crate::hud::HudVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let mk_readback = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (padded_row * height) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let readback_bufs = [mk_readback(), mk_readback(), mk_readback()];

        Ok(Renderer {
            device,
            queue,
            width,
            height,
            pipeline,
            pipeline_overlay,
            globals_buf,
            bind_group,
            color_tex,
            depth_view,
            quad,
            skin_layout,
            dummy_tex: dummy,
            skin_sampler,
            hud_pipeline,
            hud_bind_group,
            hud_atlas,
            hud_layout,
            hud_sampler,
            hud_screen_buf: screen_buf,
            hud_atlas_view: atlas_view,
            inst_buf,
            hud_vbuf,
            readback_bufs,
            readback_submissions: std::cell::RefCell::new([None, None, None]),
        })
    }

    /// The render target's (width, height) in pixels.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Uploads a skin's bundled textures (from an imported `.rhs`) to the GPU
    /// once, ready to render many frames. Absent textures use a dummy and
    /// keep the shader on the procedural path.
    pub fn prepare_skin(&self, config: &SkinConfig) -> SkinTextures {
        let note = config
            .note_texture
            .as_deref()
            .and_then(|b| self.upload_png(b));
        let border = config
            .border_texture
            .as_deref()
            .and_then(|b| self.upload_png(b));
        let cursor = config
            .cursor_texture
            .as_deref()
            .and_then(|b| self.upload_png(b));
        // Custom background layers, composited once on the CPU into a
        // frame-sized image (static under camera motion — a documented
        // approximation for parallax/spin).
        let background = compose_background(config, self.width, self.height)
            .map(|(rgba, w, h)| self.upload_rgba(&rgba, w, h));

        let tex_flags = [
            note.is_some() as u32 as f32,
            border.is_some() as u32 as f32,
            cursor.is_some() as u32 as f32,
            background.is_some() as u32 as f32,
        ];
        let view = |t: &Option<wgpu::Texture>| {
            t.as_ref()
                .unwrap_or(&self.dummy_tex)
                .create_view(&Default::default())
        };
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skin-bg"),
            layout: &self.skin_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(&note)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(&border)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view(&cursor)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.skin_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&view(&background)),
                },
            ],
        });
        SkinTextures {
            bind_group,
            tex_flags,
        }
    }

    /// Decodes image bytes (PNG/JPEG/WebP/BMP, any colour type) and uploads
    /// them as an sRGB texture.
    fn upload_png(&self, bytes: &[u8]) -> Option<wgpu::Texture> {
        let (rgba, w, h) = decode_image_rgba(bytes)?;
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skin-tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            tex.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Some(tex)
    }

    /// Renders one still frame at `song_time_ms` and returns RGBA8 pixels
    /// (row-major, width*height*4 bytes). The player's `config` drives the
    /// note shape, border style, cursor, trail and colours.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn render_still(
        &self,
        params: &SceneParams,
        config: &SkinConfig,
        skin: &SkinTextures,
        replay: &Replay,
        map: &Map,
        song_time_ms: f64,
        hud_state: Option<&crate::hud::HudState>,
    ) -> Result<Vec<u8>, Error> {
        self.render_still_with_ghost(params, config, skin, replay, map, song_time_ms, hud_state, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_still_with_ghost(
        &self,
        params: &SceneParams,
        config: &SkinConfig,
        skin: &SkinTextures,
        replay: &Replay,
        map: &Map,
        song_time_ms: f64,
        hud_state: Option<&crate::hud::HudState>,
        ghost: Option<&crate::hud::GhostInput>,
    ) -> Result<Vec<u8>, Error> {
        self.submit_frame_with_ghost(
            params,
            config,
            skin,
            replay,
            map,
            song_time_ms,
            hud_state,
            ghost,
            0,
        )?;
        self.with_slot_pixels(0, |px| px.to_vec())
    }

    /// Renders one frame and queues its copy into readback slot `slot`
    /// WITHOUT waiting — pair with [`Self::with_slot_pixels`]. Submitting
    /// frame N and then reading slot N-1 overlaps GPU rendering with the
    /// CPU-side readback/encode of the previous frame.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn submit_frame(
        &self,
        params: &SceneParams,
        config: &SkinConfig,
        skin: &SkinTextures,
        replay: &Replay,
        map: &Map,
        song_time_ms: f64,
        hud_state: Option<&crate::hud::HudState>,
        slot: usize,
    ) -> Result<(), Error> {
        self.submit_frame_with_ghost(
            params,
            config,
            skin,
            replay,
            map,
            song_time_ms,
            hud_state,
            None,
            slot,
        )
    }

    /// Like [`Self::submit_frame`], with an optional second replay: the
    /// frame splits into two side-by-side views — the player's run on the
    /// left, the ghost's on the right — each with its own full HUD and the
    /// ghost's cursor/trail in its distinct colour.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_frame_with_ghost(
        &self,
        params: &SceneParams,
        config: &SkinConfig,
        skin: &SkinTextures,
        replay: &Replay,
        map: &Map,
        song_time_ms: f64,
        hud_state: Option<&crate::hud::HudState>,
        ghost: Option<&crate::hud::GhostInput>,
        slot: usize,
    ) -> Result<(), Error> {
        match ghost {
            None => self.submit_side(
                params,
                config,
                skin,
                replay,
                map,
                song_time_ms,
                hud_state,
                (0, self.width),
                true,
                Some(slot),
            ),
            Some(g) => {
                let half = self.width / 2;
                self.submit_side(
                    params,
                    config,
                    skin,
                    replay,
                    map,
                    song_time_ms,
                    hud_state,
                    (0, half),
                    true,
                    None,
                )?;
                let mut ghost_cfg = config.clone();
                ghost_cfg.cursor_color = g.color;
                ghost_cfg.cursor_trail_color = g.color;
                ghost_cfg.cursor_trail_gradient.clear();
                ghost_cfg.cursor_trail_inherit = true;
                // Meters may sit elsewhere on the ghost's side.
                for m in [&mut ghost_cfg.hud.error_meter, &mut ghost_cfg.hud.aim_meter] {
                    if let Some(gx) = m.ghost_x {
                        m.x = gx;
                    }
                    if let Some(gy) = m.ghost_y {
                        m.y = gy;
                    }
                }
                // The ghost plays on its own field: its map already carries
                // its mods, and its border widens with its grid.
                let mut ghost_params = *params;
                ghost_params.grid_scale = g.grid_scale;
                self.submit_side(
                    &ghost_params,
                    &ghost_cfg,
                    skin,
                    &g.replay,
                    &g.map,
                    song_time_ms,
                    hud_state.map(|_| &g.state),
                    (half, self.width - half),
                    false,
                    Some(slot),
                )
            }
        }
    }

    /// Renders one view into the given horizontal viewport slice: the full
    /// scene plus its HUD, optionally clearing first and queueing the
    /// framebuffer readback.
    #[allow(clippy::too_many_arguments)]
    fn submit_side(
        &self,
        params: &SceneParams,
        config: &SkinConfig,
        skin: &SkinTextures,
        replay: &Replay,
        map: &Map,
        song_time_ms: f64,
        hud_state: Option<&crate::hud::HudState>,
        viewport: (u32, u32),
        clear: bool,
        readback_slot: Option<usize>,
    ) -> Result<(), Error> {
        let (vp_x, vp_w) = viewport;
        let aspect = vp_w as f32 / self.height as f32;
        let cursor = replay.cursor_at(song_time_ms);
        let view_proj = params.view_proj(aspect, cursor);

        let (corner, outline) = config.note_shape.sdf_params();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            glow_tint: [0.45, 0.7, 1.1, 0.5],
            params: [
                corner,
                outline,
                if config.border_is_corners() { 1.0 } else { 0.0 },
                0.30,
            ],
            tex_flags: skin.tex_flags,
        };
        self.queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        // Every shape is a quad on the hit plane / at its approach depth,
        // collected with a z sort key and drawn back-to-front so alpha
        // blends correctly (depth testing still resolves note occlusion).
        let mut items: Vec<(f32, Instance)> = Vec::new();
        // Custom skin background: a fullscreen quad at the far plane, drawn
        // before everything (kind 4 bypasses the camera in the shader).
        if skin.tex_flags[3] > 0.5 {
            items.push((
                f32::NEG_INFINITY,
                Instance {
                    model: Mat4::IDENTITY.to_cols_array_2d(),
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: 4.0,
                    _pad: [0.0; 3],
                },
            ));
        }

        // Playfield border, just outside the ±1 grid, flat at the hit plane.
        let ph = params.playfield_half();
        let border_model = Mat4::from_translation(glam::Vec3::new(0.0, 0.0, -0.02))
            * Mat4::from_scale(glam::Vec3::new(ph, ph, 1.0));
        let [br, bg, bb] = srgb_to_linear(config.border_color);
        items.push((
            -0.02,
            Instance::border(border_model, [br, bg, bb, config.border_opacity]),
        ));

        // Ambient tunnel: nested square outlines behind everything, each
        // layer twisted against the last, slowly rotating (a full turn takes
        // about a minute) with a per-layer brightness pulse — reconstructed
        // from a kymograph of the user's footage. With AccentFromHitNote the
        // colour follows the most recently hit note.
        if AMBIENT_EFFECTS_SUPPORTED
            && config.ambient.tunnel_enabled
            && config.ambient.tunnel_opacity > 0.0
        {
            let t = song_time_ms / 1000.0;
            let accent = if config.ambient.accent_from_hit_note {
                last_hit_color(config, hud_state, song_time_ms).unwrap_or(config.ambient.accent)
            } else {
                config.ambient.accent
            };
            let [ar, ag, ab] = srgb_to_linear(accent);
            let layers = 10;
            for i in 0..layers {
                let s = 0.7f32 * 1.33f32.powi(i);
                let theta = i as f32 * 0.42
                    + (t as f32) * 0.16 * (1.0 + i as f32 * 0.09)
                    + i as f32 * i as f32 * 0.11;
                // Slow per-layer brightness pulse (bright phases of ~8 s).
                let pulse = 0.5 + 0.5 * ((t as f32) * 0.785 + i as f32 * 2.4).sin();
                let alpha = crate::config::srgb8_to_linear(
                    [255, 255, 255],
                    config.ambient.tunnel_opacity * (0.35 + 0.65 * pulse),
                )[3];
                let color = [ar, ag, ab, alpha];
                for k in 0..4u32 {
                    // Irregularity, as in the footage: bar lengths vary and
                    // some edges are missing entirely.
                    let h = hash01(i as u32 * 7 + k * 131);
                    if h > 0.85 {
                        continue;
                    }
                    let half_len = s * (0.55 + 0.45 * hash01(i as u32 * 13 + k * 57));
                    let off = s * (hash01(i as u32 * 29 + k * 17) - 0.5) * 0.5;
                    let half_th = s * 0.028;
                    let a = theta + k as f32 * std::f32::consts::FRAC_PI_2;
                    let model = Mat4::from_rotation_z(a)
                        * Mat4::from_translation(glam::Vec3::new(off, s, -0.06))
                        * Mat4::from_scale(glam::Vec3::new(half_len, half_th, 1.0));
                    items.push((-0.06 - i as f32 * 1e-4, Instance::solid(model, color)));
                }
            }
        }

        // Ambient chevrons ("moving chevron outline layer"): arrow outlines
        // that spawn far behind the playfield and fly toward and past the
        // camera, in two columns left/right of the centre with tips pointing
        // inward. Colour is the fixed background accent — the hit-note tint
        // does not apply to this layer (per the user).
        if AMBIENT_EFFECTS_SUPPORTED
            && config.ambient.chevron_enabled
            && config.ambient.chevron_opacity > 0.0
        {
            let t = song_time_ms as f32 / 1000.0;
            let px = 2.27 / 540.0; // world units per 1080p pixel at z≈0
            let amb = &config.ambient;
            let [ar, ag, ab] = srgb_to_linear(amb.accent);
            let alpha = crate::config::srgb8_to_linear([255; 3], amb.chevron_opacity)[3];
            let color = [ar, ag, ab, alpha];
            let gap_half = (amb.chevron_gap * px * 0.5).max(0.3);
            let arm_th = (amb.chevron_width * px * 0.5).max(0.004);
            let z_spacing = 5.0f32;
            let z_far = -30.0f32;
            let count = 7;
            let speed = 3.0 * amb.chevron_speed; // world units/s toward camera
            let scroll = (t * speed) % z_spacing;
            for side in [-1.0f32, 1.0] {
                for j in 0..count {
                    let z = z_far + j as f32 * z_spacing + scroll;
                    if z > 1.5 {
                        continue;
                    }
                    let size = if j % 2 == 0 {
                        amb.chevron_large
                    } else {
                        amb.chevron_small
                    };
                    let arm = size * px;
                    // Apex on the column's inner edge, pointing inward.
                    let x = side * gap_half;
                    for updown in [-1.0f32, 1.0] {
                        let a = updown * std::f32::consts::FRAC_PI_4;
                        let model = Mat4::from_translation(glam::Vec3::new(x, 0.0, z))
                            * Mat4::from_rotation_z(a)
                            * Mat4::from_translation(glam::Vec3::new(side * arm * 0.5, 0.0, 0.0))
                            * Mat4::from_scale(glam::Vec3::new(arm * 0.5, arm_th, 1.0));
                        items.push((z, Instance::solid(model, color)));
                    }
                }
            }
        }

        // Ambient rays ("depth rays that streak toward the camera"): thin
        // streaks aligned along the depth axis flying from far behind toward
        // and past the camera — on screen they read as radial speed lines.
        if AMBIENT_EFFECTS_SUPPORTED
            && config.ambient.rays_enabled
            && config.ambient.rays_opacity > 0.0
        {
            let amb = &config.ambient;
            let t = song_time_ms as f32 / 1000.0;
            let px = 2.27 / 540.0;
            let [ar, ag, ab] = srgb_to_linear(amb.accent);
            let alpha = crate::config::srgb8_to_linear(
                [255; 3],
                (amb.rays_opacity * (0.5 + amb.rays_intensity)).min(1.0),
            )[3];
            let rays = 26u32;
            let z_span = 40.0f32;
            let speed = 9.0f32; // world units/s toward the camera
            for i in 0..rays {
                let ang = hash01(i * 733) * std::f32::consts::TAU;
                let r = 1.6 + 3.2 * hash01((i * 733) ^ 0xabcd);
                let len = 2.0 + 4.0 * hash01((i * 733) ^ 0x1234);
                let th = (amb.rays_width * px * 0.35).max(0.003);
                // Each ray wraps through the depth span at its own offset.
                let z = -((hash01((i * 733) ^ 0x77) * z_span - t * speed).rem_euclid(z_span)) + 2.0;
                let model = Mat4::from_rotation_z(ang)
                    * Mat4::from_translation(glam::Vec3::new(r, 0.0, z))
                    * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
                    * Mat4::from_scale(glam::Vec3::new(len * 0.5, th, 1.0));
                items.push((z, Instance::solid(model, [ar, ag, ab, alpha])));
            }
        }

        // Ambient grid: a synthwave floor and ceiling — perspective planes
        // with static lines running away from the camera and cross lines
        // scrolling toward it (kymograph: ~one line per second).
        if AMBIENT_EFFECTS_SUPPORTED
            && config.ambient.grid_enabled
            && config.ambient.grid_opacity > 0.0
        {
            let amb = &config.ambient;
            let t = song_time_ms as f32 / 1000.0;
            let px = 2.27 / 540.0;
            let [ar, ag, ab] = srgb_to_linear(amb.accent);
            let plane_y = (amb.grid_center_gap * 0.5 * px).max(0.5); // half-gap
            let cell = (amb.grid_cell_size * px * 2.0).max(0.3);
            let depth_cells = 24;
            let half_w = cell * 10.0;
            let base_alpha = crate::config::srgb8_to_linear([255; 3], amb.grid_opacity)[3] * 0.55;
            let th = 0.024;
            let scroll = (t * amb.grid_speed * cell) % cell;
            for side in [-1.0f32, 1.0] {
                let y = side * plane_y;
                // Cross lines scrolling toward the camera.
                for k in 1..depth_cells {
                    let z = -(k as f32 * cell - scroll) - 0.2;
                    let fade = (1.0 - k as f32 / depth_cells as f32)
                        .powf(amb.grid_fade_falloff)
                        .max(0.0);
                    let model = Mat4::from_translation(glam::Vec3::new(0.0, y, z))
                        * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
                        * Mat4::from_scale(glam::Vec3::new(half_w, th, 1.0));
                    items.push((z, Instance::solid(model, [ar, ag, ab, base_alpha * fade])));
                }
                // Length lines running away from the camera (static), split
                // into segments so they can fade with distance.
                for xk in -10i32..=10 {
                    let x = xk as f32 * cell;
                    let segs = 6;
                    for sgm in 0..segs {
                        let z0 = -0.2 - (depth_cells as f32 * cell) * sgm as f32 / segs as f32;
                        let z1 =
                            -0.2 - (depth_cells as f32 * cell) * (sgm + 1) as f32 / segs as f32;
                        let fade = (1.0 - sgm as f32 / segs as f32).powf(amb.grid_fade_falloff);
                        let model = Mat4::from_translation(glam::Vec3::new(x, y, (z0 + z1) * 0.5))
                            * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2)
                            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)
                            * Mat4::from_scale(glam::Vec3::new((z0 - z1).abs() * 0.5, th, 1.0));
                        items.push((
                            (z0 + z1) * 0.5,
                            Instance::solid(model, [ar, ag, ab, base_alpha * fade]),
                        ));
                    }
                }
            }
        }

        // Playfield grid: the 3×3 cell separator lines at ±0.5.
        if config.playfield_grid && config.playfield_grid_opacity > 0.0 {
            let [gr, gg, gb] = srgb_to_linear(config.playfield_grid_color);
            let gcol = [gr, gg, gb, config.playfield_grid_opacity];
            let half_t = (config.playfield_grid_thickness / 1440.0).max(0.0005);
            for s in [-0.5f32, 0.5] {
                let v = Mat4::from_translation(glam::Vec3::new(s, 0.0, -0.018))
                    * Mat4::from_scale(glam::Vec3::new(half_t, ph, 1.0));
                let h = Mat4::from_translation(glam::Vec3::new(0.0, s, -0.018))
                    * Mat4::from_scale(glam::Vec3::new(ph, half_t, 1.0));
                items.push((-0.018, Instance::solid(v, gcol)));
                items.push((-0.018, Instance::solid(h, gcol)));
            }
        }

        // Notes. With PushBack, missed notes fly on past the hit plane and
        // fade out instead of vanishing at it.
        let hits = hud_state.map(|s| s.results());
        for (i, note) in map.notes.iter().enumerate() {
            let note_t = note.time_ms as f64;
            let mut depth_opacity: Option<(f32, f32)> = params
                .note_depth(note_t, song_time_ms)
                .map(|d| (d, params.note_opacity(d)));
            if depth_opacity.is_none() && config.push_back {
                let behind = ((note_t - song_time_ms) / 1000.0) as f32 * params.approach_rate;
                let missed = hits.map(|h| !h[i].hit).unwrap_or(false);
                if missed && (-2.5..0.0).contains(&behind) {
                    depth_opacity = Some((behind, (1.0 + behind / 2.5).max(0.0)));
                }
            }
            if let Some((depth, opacity)) = depth_opacity {
                let model = params.note_model(note.x, note.y, depth);
                // Real colours from an imported colorset take priority; else
                // the named-palette approximation.
                let rgb = if config.colorset.is_empty() {
                    colorset_color(&config.colorset_name, i)
                } else {
                    config.colorset[i % config.colorset.len()]
                };
                let [r, g, b] = srgb_to_linear(rgb);
                items.push((-depth, Instance::note(model, [r, g, b, opacity])));
            }
        }

        // Cursor (+ optional trail) render after everything with the
        // depth-free overlay pipeline, in list order.
        let mut overlay: Vec<Instance> = Vec::new();
        self.push_cursor(&mut overlay, config, replay, song_time_ms);

        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut instances: Vec<Instance> = items.into_iter().map(|(_, inst)| inst).collect();
        let main_count = instances.len().min(INSTANCE_CAP);
        instances.extend(overlay);
        instances.truncate(INSTANCE_CAP);
        if instances.is_empty() {
            // Never draw from an unwritten buffer region.
            instances.push(Instance::note(Mat4::from_scale(glam::Vec3::ZERO), [0.0; 4]));
        }
        self.queue
            .write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&instances));

        let view = self.color_tex.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // Background colour from the config (default black);
                        // the right half of a split frame loads the left.
                        load: if clear {
                            wgpu::LoadOp::Clear({
                                let [r, g, b] = srgb_to_linear(config.background_color);
                                wgpu::Color {
                                    r: r as f64,
                                    g: g as f64,
                                    b: b as f64,
                                    a: 1.0,
                                }
                            })
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(
                vp_x as f32,
                0.0,
                vp_w as f32,
                self.height as f32,
                0.0,
                1.0,
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &skin.bind_group, &[]);
            pass.set_vertex_buffer(0, self.quad.vertices.slice(..));
            pass.set_vertex_buffer(1, self.inst_buf.slice(..));
            pass.set_index_buffer(self.quad.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.quad.index_count, 0, 0..main_count as u32);
            if instances.len() > main_count {
                pass.set_pipeline(&self.pipeline_overlay);
                pass.draw_indexed(
                    0..self.quad.index_count,
                    0,
                    main_count as u32..instances.len() as u32,
                );
            }
        }

        // HUD overlay: a second pass that loads the rendered scene and draws
        // the flat 2D stat panels/bars/ring/title on top (no depth).
        let hud_verts = hud_state.filter(|_| !config.disable_gui).map(|state| {
            let stats = state.stats_at(map, replay, song_time_ms);
            let field = self.playfield_screen(&view_proj, params.playfield_half(), vp_w);
            // Project freshly missed notes' cells to screen for the X marks.
            let miss_marks: Vec<(f32, f32, f64)> = state
                .recent_misses(map, song_time_ms)
                .into_iter()
                .map(|(gx, gy, age)| {
                    let (wx, wy) = crate::scene::grid_to_world(gx, gy);
                    let c = view_proj * glam::Vec4::new(wx, wy, 0.0, 1.0);
                    let ndc = c.truncate() / c.w;
                    (
                        (ndc.x * 0.5 + 0.5) * vp_w as f32,
                        (0.5 - ndc.y * 0.5) * self.height as f32,
                        age,
                    )
                })
                .collect();
            crate::hud::build_hud(
                &self.hud_atlas,
                config,
                state,
                &stats,
                replay,
                map,
                song_time_ms,
                &field,
                &miss_marks,
                vp_w,
                self.height,
            )
        });
        if let Some(mut verts) = hud_verts.filter(|v| !v.is_empty()) {
            if vp_x > 0 {
                for v in &mut verts {
                    v.pos[0] += vp_x as f32;
                }
            }
            verts.truncate(HUD_VERT_CAP - HUD_VERT_CAP % 3);
            self.queue
                .write_buffer(&self.hud_vbuf, 0, bytemuck::cast_slice(&verts));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hud_pipeline);
            pass.set_bind_group(0, &self.hud_bind_group, &[]);
            pass.set_vertex_buffer(0, self.hud_vbuf.slice(..));
            pass.draw(0..verts.len() as u32, 0..1);
        }

        // Queue the framebuffer copy into the slot's readback buffer as part
        // of the same submission, and remember its index for a targeted
        // wait. A side without readback (the left half of a split frame)
        // just submits its work.
        if let Some(slot) = readback_slot {
            let padded = (self.width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.color_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.readback_bufs[slot % READBACK_SLOTS],
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(self.height),
                    },
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );
            let idx = self.queue.submit(Some(encoder.finish()));
            self.readback_submissions.borrow_mut()[slot % READBACK_SLOTS] = Some(idx);
        } else {
            self.queue.submit(Some(encoder.finish()));
        }
        Ok(())
    }

    /// Maps readback slot `slot`, waits only for its own submission, and
    /// hands the tightly-packed RGBA pixels to `f`. Zero-copy when the row
    /// stride needs no padding (true for common widths like 1920/2560).
    pub fn with_slot_pixels<R>(&self, slot: usize, f: impl FnOnce(&[u8]) -> R) -> Result<R, Error> {
        let unpadded = self.width * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = &self.readback_bufs[slot % READBACK_SLOTS];
        let submission = self.readback_submissions.borrow_mut()[slot % READBACK_SLOTS].take();

        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: submission,
                timeout: None,
            })
            .map_err(|e| Error::Device(e.to_string()))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| Error::Device(e.to_string()))?;
        let result = if padded == unpadded {
            f(&mapped[..(unpadded * self.height) as usize])
        } else {
            let mut pixels = Vec::with_capacity((unpadded * self.height) as usize);
            for row in 0..self.height {
                let start = (row * padded) as usize;
                pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
            }
            f(&pixels)
        };
        drop(mapped);
        buf.unmap();
        Ok(result)
    }

    /// Renders the results screen (after a finish or fail): blurred cover
    /// background, cover, title block, big grade, statistics, health graph
    /// and mods — the game's screen minus its interactive buttons.
    pub fn render_results(
        &self,
        replay: &Replay,
        map: &Map,
        hud_state: &crate::hud::HudState,
        config: &SkinConfig,
        ghost: Option<&crate::hud::GhostInput>,
    ) -> Result<Vec<u8>, Error> {
        let (w, h) = (self.width as f32, self.height as f32);
        // Final stats. A failed run ends at its fail time — notes after it
        // were never attempted and must not count as misses. The window
        // margin lets the killing miss itself (note at ~fail time) register.
        let final_stats = |r: &Replay, state: &crate::hud::HudState| {
            let stats_end = if r.failed() {
                r.fail_time_ms as f64 + rhythia_sim::hitreg::DEFAULT_WINDOW_MS + 1.0
            } else {
                f64::MAX
            };
            let mut st = state.stats_at(map, r, stats_end);
            // The results screen shows the stored final score; the running
            // 100-per-combo sum can drift a few thousandths from it.
            if !r.failed() {
                st.score = r.total_score;
            }
            st
        };
        // One side per player: the whole layout renders once at full width,
        // or twice at half width for a ghost race (two scores, one screen).
        let mut sides: Vec<(&Replay, crate::hud::HudStats, f32, f32)> = Vec::new();
        match ghost {
            None => sides.push((replay, final_stats(replay, hud_state), 0.0, w)),
            Some(g) => {
                sides.push((replay, final_stats(replay, hud_state), 0.0, w * 0.5));
                sides.push((&g.replay, final_stats(&g.replay, &g.state), w * 0.5, w * 0.5));
            }
        }

        // Cover textures: full resolution + a tiny average-pooled copy that
        // linear sampling stretches into a cheap blur. A dark grey stands in
        // when the map has no cover.
        let cover_rgba =
            map.cover
                .as_deref()
                .and_then(decode_image_rgba)
                .unwrap_or((vec![28, 28, 32, 255], 1, 1));
        let (pixels, cw, ch) = cover_rgba;
        let cover_tex = self.upload_rgba(&pixels, cw, ch);
        let (bp, bw, bh) = average_pool(&pixels, cw, ch, 20);
        let blur_tex = self.upload_rgba(&bp, bw, bh);
        let cover_view = cover_tex.create_view(&Default::default());
        let blur_view = blur_tex.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("results-bind"),
            layout: &self.hud_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.hud_screen_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.hud_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.hud_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&cover_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&blur_view),
                },
            ],
        });

        // Geometry: dimmed blurred background, cover with a green frame,
        // then the text/graph overlay.
        let mut verts: Vec<crate::hud::HudVertex> = Vec::new();
        let quad = |verts: &mut Vec<crate::hud::HudVertex>,
                    x0: f32,
                    y0: f32,
                    x1: f32,
                    y1: f32,
                    color: [f32; 4],
                    mode: f32| {
            let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            let p = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
            for &[i, j, k] in &[[0usize, 1, 2], [0, 2, 3]] {
                for idx in [i, j, k] {
                    verts.push(crate::hud::HudVertex {
                        pos: p[idx],
                        uv: uv[idx],
                        color,
                        mode,
                        _pad: 0.0,
                    });
                }
            }
        };
        // Background: blurred cover, heavily dimmed.
        quad(&mut verts, 0.0, 0.0, w, h, [0.055, 0.055, 0.06, 1.0], 3.0);
        // Cover with a green frame, top-left as in the game. One map, one
        // cover — a ghost race shares it (with the title block) at full
        // size instead of duplicating it per side.
        let (cx0, cy0, cx1, cy1) = (w * 0.044, h * 0.062, w * 0.214, h * 0.365);
        let f = (h * 0.004).max(2.0);
        quad(
            &mut verts,
            cx0 - f,
            cy0 - f,
            cx1 + f,
            cy1 + f,
            crate::config::srgb8_to_linear([34, 197, 94], 1.0),
            0.0,
        );
        quad(&mut verts, cx0, cy0, cx1, cy1, [1.0, 1.0, 1.0, 1.0], 2.0);
        if ghost.is_some() {
            verts.extend(crate::hud::build_results(
                &self.hud_atlas,
                replay,
                map,
                &sides[0].1,
                self.width,
                self.height,
                true,
                crate::hud::ResultsPart::Header,
            ));
        }
        for (side_replay, side_stats, x_off, w_eff) in &sides {
            let (x_off, w_eff) = (*x_off, *w_eff);
            let icons = active_mod_icons(side_replay, config);
            let part = if ghost.is_some() {
                crate::hud::ResultsPart::Side
            } else {
                crate::hud::ResultsPart::Full
            };
            let side_verts = crate::hud::build_results(
                &self.hud_atlas,
                side_replay,
                map,
                side_stats,
                w_eff as u32,
                self.height,
                !icons.is_empty(),
                part,
            );
            verts.extend(side_verts.into_iter().map(|mut v| {
                v.pos[0] += x_off;
                v
            }));
        }

        let vbuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("results-verts"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let view = self.color_tex.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("results"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hud_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.draw(0..verts.len() as u32, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        let mut pixels = self.read_pixels()?;

        // Mod icons, composited onto the static frame CPU-side (cheaper than
        // extra texture plumbing for a once-per-render image). They sit in
        // each side's Mods box, right of the speed letter, where the text
        // fallback would otherwise be.
        for (side_replay, _, x_off, w_eff) in &sides {
            let icons = active_mod_icons(side_replay, config);
            if icons.is_empty() {
                continue;
            }
            let icon_h = (h * 0.052) as u32;
            // Clear the speed notation, which sits proportionally wider on
            // a half-width side.
            let frac = if ghost.is_some() { 0.63 } else { 0.58 };
            let mut x = (x_off + w_eff * frac) as u32;
            let y = (h * 0.775) as u32 - icon_h / 2;
            for (_, png) in icons {
                if let Some((rgba, iw, ih)) = decode_image_rgba(png) {
                    let (small, sw, sh) = downscale_icon(&rgba, iw, ih, icon_h.max(8));
                    blit_over(&mut pixels, self.width, &small, sw, sh, x, y);
                    x += sw + (h * 0.015) as u32;
                }
            }
        }
        Ok(pixels)
    }

    /// Uploads an RGBA8 image as an sRGB texture.
    fn upload_rgba(&self, pixels: &[u8], w: u32, h: u32) -> wgpu::Texture {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("results-cover"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            tex.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        tex
    }

    /// Screen-space (pixel) box of the playfield border at the hit plane,
    /// used to anchor the HUD. Projects the world origin and the ±`world_half`
    /// edges (the bracket box the game's HUD hangs off).
    fn playfield_screen(
        &self,
        view_proj: &Mat4,
        world_half: f32,
        width_px: u32,
    ) -> crate::hud::Playfield {
        let project = |p: glam::Vec3| -> [f32; 2] {
            let c = *view_proj * glam::Vec4::new(p.x, p.y, p.z, 1.0);
            let ndc = c.truncate() / c.w;
            [
                (ndc.x * 0.5 + 0.5) * width_px as f32,
                (0.5 - ndc.y * 0.5) * self.height as f32,
            ]
        };
        let c = project(glam::Vec3::ZERO);
        let ex = project(glam::Vec3::new(world_half, 0.0, 0.0));
        let ey = project(glam::Vec3::new(0.0, world_half, 0.0));
        // Average the x/y projections so a non-square frame gives one radius.
        let half = ((ex[0] - c[0]).abs() + (c[1] - ey[1]).abs()) * 0.5;
        crate::hud::Playfield {
            cx: c[0],
            cy: c[1],
            half,
        }
    }

    /// Emits the trail stamps (oldest first) and the cursor, in draw order
    /// for the depth-free overlay pipeline.
    fn push_cursor(
        &self,
        items: &mut Vec<Instance>,
        config: &SkinConfig,
        replay: &Replay,
        song_time_ms: f64,
    ) {
        let [cr, cg, cb] = srgb_to_linear(config.cursor_color);
        let size = 0.10 * config.cursor_scale;
        if config.cursor_trail_enabled {
            // The game's trail is a smooth snake: soft stamps laid out by
            // DISTANCE along the recent cursor path (SpacingMultiplier is
            // the stamp interval in cursor widths — 0.05 reads as a solid
            // ribbon, 2.0 as separate dots), cursor-wide at the head and
            // tapering to a tip, with a steep fade so the bright tail is
            // much shorter than the lifetime.
            let span_ms = (config.cursor_trail_fade_secs as f64 * 1000.0).max(1.0);
            let base = if config.cursor_trail_inherit {
                config.cursor_color
            } else {
                config.cursor_trail_color
            };
            let head_w = size * 1.05;
            let stamp_dist = (config.cursor_trail_spacing * head_w * 2.0).max(0.002);
            // Walk the path head→tail in fine time steps, stamping whenever
            // the accumulated distance passes the interval.
            let samples = 512usize;
            let mut stamps = 0usize;
            let mut trail: Vec<Instance> = Vec::new();
            let mut carry = 0.0f32;
            let mut prev = replay.cursor_at(song_time_ms);
            'walk: for i in 1..=samples {
                let frac = i as f32 / samples as f32; // 0 = head, 1 = oldest
                let cur = replay.cursor_at(song_time_ms - span_ms * i as f64 / samples as f64);
                let (dx, dy) = (cur.0 - prev.0, cur.1 - prev.1);
                let seg = (dx * dx + dy * dy).sqrt();
                let mut along = stamp_dist - carry;
                while along <= seg {
                    let u = along / seg.max(1e-6);
                    let (x, y) = (prev.0 + dx * u, prev.1 + dy * u);
                    let rgb = if config.cursor_trail_gradient.is_empty() {
                        base
                    } else {
                        gradient_color(&config.cursor_trail_gradient, frac)
                    };
                    let [tr, tg, tb] = srgb_to_linear(rgb);
                    let alpha = config.cursor_trail_opacity * (1.0 - frac).powf(1.8);
                    let w = if config.cursor_trail_shrink {
                        head_w * (1.0 - 0.85 * frac.powf(1.3))
                    } else {
                        head_w
                    };
                    if w > 1e-3 && alpha > 0.002 {
                        let model = Mat4::from_translation(glam::Vec3::new(x, y, 0.01))
                            * Mat4::from_scale(glam::Vec3::splat(w));
                        trail.push(Instance::dot(model, [tr, tg, tb, alpha]));
                    }
                    stamps += 1;
                    if stamps >= 4000 {
                        break 'walk;
                    }
                    along += stamp_dist;
                }
                carry = seg - (along - stamp_dist);
                prev = cur;
            }
            let _ = stamps;
            // Collected head→tail; draw oldest first so the head blends on
            // top.
            trail.reverse();
            items.extend(trail);
        }
        let (x, y) = replay.cursor_at(song_time_ms);
        let model = Mat4::from_translation(glam::Vec3::new(x, y, 0.02))
            * Mat4::from_rotation_z(config.cursor_rotation_deg.to_radians())
            * Mat4::from_scale(glam::Vec3::splat(size));
        items.push(Instance::dot(model, [cr, cg, cb, config.cursor_opacity]));
    }

    fn read_pixels(&self) -> Result<Vec<u8>, Error> {
        let bytes_per_pixel = 4u32;
        let unpadded = self.width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        let out_buf = &self.readback_bufs[0];

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: out_buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = out_buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| Error::Device(e.to_string()))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|e| Error::Device(e.to_string()))?;
        let mut pixels = Vec::with_capacity((unpadded * self.height) as usize);
        for row in 0..self.height {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        out_buf.unmap();
        Ok(pixels)
    }
}

/// Note colour by index, cycling the (approximated) named colourset. The
/// real `.txt` colorsets live compressed in the game bundle; these are
/// visual stand-ins keyed by name — "Arctic" reads as icy blues/whites.
/// Converts an sRGB colour component/triple to linear. Colours come from the
/// config/colorsets as sRGB; the sRGB render target re-encodes on write, so
/// the shader must receive linear values or everything renders too bright.
/// Cheap deterministic hash → [0,1), for ambient-effect variation (no RNG
/// dependency; must be stable across frames).
fn hash01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Colour (sRGB 0..1) of the most recently hit note before `song_time_ms`,
/// for the `BackgroundAccentFromHitNote` ambient tint.
fn last_hit_color(
    config: &SkinConfig,
    hud_state: Option<&crate::hud::HudState>,
    song_time_ms: f64,
) -> Option<[f32; 3]> {
    let results = hud_state?.results();
    let mut best: Option<(f64, usize)> = None;
    for (i, r) in results.iter().enumerate() {
        if let Some(ht) = r.hit_ms.filter(|&ht| r.hit && ht <= song_time_ms) {
            if best.map(|(bt, _)| ht > bt).unwrap_or(true) {
                best = Some((ht, i));
            }
        }
    }
    let (_, i) = best?;
    Some(if config.colorset.is_empty() {
        colorset_color(&config.colorset_name, i)
    } else {
        config.colorset[i % config.colorset.len()]
    })
}

/// Samples the trail gradient at `t` (0 = newest dot, 1 = oldest), linearly
/// interpolating between the sorted colour stops.
fn gradient_color(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    match stops {
        [] => [1.0, 1.0, 1.0],
        [only] => only.1,
        _ => {
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for w in stops.windows(2) {
                let (p0, c0) = w[0];
                let (p1, c1) = w[1];
                if t <= p1 {
                    let u = ((t - p0) / (p1 - p0).max(1e-6)).clamp(0.0, 1.0);
                    return [
                        c0[0] + (c1[0] - c0[0]) * u,
                        c0[1] + (c1[1] - c0[1]) * u,
                        c0[2] + (c1[2] - c0[2]) * u,
                    ];
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

/// Decodes PNG bytes to (rgba8, width, height); None on any decode error.
/// Composites the skin's `BackgroundImages[]` layers into one frame-sized
/// RGBA image (bottom-up, alpha-over). Screen layers use fit/scale/centre
/// against the frame; world layers project their grid-space rect through
/// the default camera. Rotation is ignored (rare). Returns None without
/// layers.
fn compose_background(config: &SkinConfig, width: u32, height: u32) -> Option<(Vec<u8>, u32, u32)> {
    if config.background_images.is_empty() {
        return None;
    }
    let (fw, fh) = (width as f32, height as f32);
    let mut out = vec![0u8; (width * height * 4) as usize];
    let params = crate::scene::SceneParams::from(config);
    let view_proj = params.view_proj(fw / fh, (0.0, 0.0));
    let project = |x: f32, y: f32| -> (f32, f32) {
        let p = view_proj * glam::Vec4::new(x, y, 0.0, 1.0);
        let ndc = p / p.w.max(1e-6);
        ((ndc.x * 0.5 + 0.5) * fw, (0.5 - ndc.y * 0.5) * fh)
    };

    for layer in &config.background_images {
        let Some((src, sw, sh)) = decode_image_rgba(&layer.bytes) else {
            continue;
        };
        let (sw_f, sh_f) = (sw as f32, sh as f32);
        // Destination rect.
        let (cx, cy, half_w, half_h) = if layer.placement == 1 {
            let (x0, y0) = project(layer.space_x - layer.space_w * 0.5, layer.space_y + layer.space_h * 0.5);
            let (x1, y1) = project(layer.space_x + layer.space_w * 0.5, layer.space_y - layer.space_h * 0.5);
            (
                (x0 + x1) * 0.5,
                (y0 + y1) * 0.5,
                (x1 - x0).abs() * 0.5 * layer.scale_x,
                (y1 - y0).abs() * 0.5 * layer.scale_y,
            )
        } else {
            // Fit against the frame: 0 stretch, 1 contain, 2 cover.
            let (base_w, base_h) = match layer.fit {
                0 => (fw, fh),
                1 => {
                    let k = (fw / sw_f).min(fh / sh_f);
                    (sw_f * k, sh_f * k)
                }
                _ => {
                    let k = (fw / sw_f).max(fh / sh_f);
                    (sw_f * k, sh_f * k)
                }
            };
            (
                layer.center_x * fw,
                layer.center_y * fh,
                base_w * 0.5 * layer.scale_x,
                base_h * 0.5 * layer.scale_y,
            )
        };
        if half_w < 1.0 || half_h < 1.0 {
            continue;
        }
        let (x_min, x_max) = ((cx - half_w).max(0.0) as u32, ((cx + half_w).min(fw)) as u32);
        let (y_min, y_max) = ((cy - half_h).max(0.0) as u32, ((cy + half_h).min(fh)) as u32);
        for dy in y_min..y_max.min(height) {
            for dx in x_min..x_max.min(width) {
                // Destination pixel → source uv (bilinear).
                let mut u = (dx as f32 - (cx - half_w)) / (half_w * 2.0);
                let v = (dy as f32 - (cy - half_h)) / (half_h * 2.0);
                if layer.flip_horizontal {
                    u = 1.0 - u;
                }
                let sx = (u * (sw_f - 1.0)).clamp(0.0, sw_f - 1.0);
                let sy = (v * (sh_f - 1.0)).clamp(0.0, sh_f - 1.0);
                let (x0, y0) = (sx as u32, sy as u32);
                let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
                let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
                let px = |x: u32, y: u32, c: usize| src[((y * sw + x) * 4) as usize + c] as f32;
                let mut rgba = [0f32; 4];
                for (c, v) in rgba.iter_mut().enumerate() {
                    let top = px(x0, y0, c) * (1.0 - tx) + px(x1, y0, c) * tx;
                    let bot = px(x0, y1, c) * (1.0 - tx) + px(x1, y1, c) * tx;
                    *v = top * (1.0 - ty) + bot * ty;
                }
                let a = rgba[3] / 255.0 * layer.tint[3];
                if a <= 0.003 {
                    continue;
                }
                let di = ((dy * width + dx) * 4) as usize;
                for c in 0..3 {
                    let s_v = rgba[c] * layer.tint[c];
                    let d_v = out[di + c] as f32;
                    out[di + c] = (s_v * a + d_v * (1.0 - a)).min(255.0) as u8;
                }
                out[di + 3] = ((a + out[di + 3] as f32 / 255.0 * (1.0 - a)) * 255.0) as u8;
            }
        }
    }
    Some((out, width, height))
}

/// Mod icons matching the replay's active mods (parsed from its JSON list),
/// in declaration order.
fn active_mod_icons<'a>(replay: &Replay, config: &'a SkinConfig) -> Vec<&'a (String, Vec<u8>)> {
    let mods: Vec<String> = serde_json::from_str(&replay.mods).unwrap_or_default();
    mods.iter()
        .filter_map(|m| config.mod_icons.iter().find(|(name, _)| name == m))
        .collect()
}

/// Downscales RGBA to `target` px on the longer side, alpha-aware: colour
/// channels average premultiplied so transparent texels don't bleed dark
/// halos into icons (average_pool is for opaque covers).
fn downscale_icon(rgba: &[u8], w: u32, h: u32, target: u32) -> (Vec<u8>, u32, u32) {
    let scale = (w.max(h)).div_ceil(target).max(1);
    let (ow, oh) = (w.div_ceil(scale).max(1), h.div_ceil(scale).max(1));
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in (oy * scale)..((oy + 1) * scale).min(h) {
                for sx in (ox * scale)..((ox + 1) * scale).min(w) {
                    let i = ((sy * w + sx) * 4) as usize;
                    let pa = rgba[i + 3] as u32;
                    r += rgba[i] as u32 * pa;
                    g += rgba[i + 1] as u32 * pa;
                    b += rgba[i + 2] as u32 * pa;
                    a += pa;
                    n += 1;
                }
            }
            let o = ((oy * ow + ox) * 4) as usize;
            if let (Some(rr), Some(gg), Some(bb)) =
                (r.checked_div(a), g.checked_div(a), b.checked_div(a))
            {
                out[o] = rr as u8;
                out[o + 1] = gg as u8;
                out[o + 2] = bb as u8;
            }
            out[o + 3] = (a / n.max(1)) as u8;
        }
    }
    (out, ow, oh)
}

/// Alpha-blends `src` (RGBA, sw×sh) onto `dst` (RGBA, dw wide) at (x, y).
fn blit_over(dst: &mut [u8], dw: u32, src: &[u8], sw: u32, sh: u32, x: u32, y: u32) {
    for row in 0..sh {
        for col in 0..sw {
            let si = ((row * sw + col) * 4) as usize;
            let a = src[si + 3] as u32;
            if a == 0 {
                continue;
            }
            let di = (((y + row) * dw + (x + col)) * 4) as usize;
            if di + 3 >= dst.len() {
                continue;
            }
            for c in 0..3 {
                let s = src[si + c] as u32;
                let d = dst[di + c] as u32;
                dst[di + c] = ((s * a + d * (255 - a)) / 255) as u8;
            }
            dst[di + 3] = 255;
        }
    }
}

fn decode_image_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    // Map covers in the wild are PNGs of every flavour (indexed, 16-bit)
    // and just as often JPEGs — decode whatever the image crate recognises.
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

/// Box-averages an RGBA image down to at most `target` pixels on its longer
/// side — linear upsampling of the result reads as a cheap blur.
fn average_pool(pixels: &[u8], w: u32, h: u32, target: u32) -> (Vec<u8>, u32, u32) {
    let scale = (w.max(h)).div_ceil(target).max(1);
    let (ow, oh) = ((w / scale).max(1), (h / scale).max(1));
    let mut out = Vec::with_capacity((ow * oh * 4) as usize);
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..scale {
                for dx in 0..scale {
                    let (x, y) = (ox * scale + dx, oy * scale + dy);
                    if x < w && y < h {
                        let i = ((y * w + x) * 4) as usize;
                        r += pixels[i] as u32;
                        g += pixels[i + 1] as u32;
                        b += pixels[i + 2] as u32;
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            out.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]);
        }
    }
    (out, ow, oh)
}

fn srgb_to_linear(c: [f32; 3]) -> [f32; 3] {
    let f = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(c[0]), f(c[1]), f(c[2])]
}

fn colorset_color(name: &str, i: usize) -> [f32; 3] {
    let n = name.to_ascii_lowercase();
    // Arctic verified against the extracted colorset table: exactly
    // [#ffffff, #a2e0ff, #a2e0ff] — white + icy blue, nothing else. (An
    // earlier 5-hue approximation had periwinkle/violet in it, which showed
    // up as wrong purple notes; user report 15.07.2026.)
    let palette: &[[f32; 3]] = if n.contains("arctic") || n.contains("ice") || n.contains("frost") {
        &[
            [1.000, 1.000, 1.000], // #ffffff
            [0.635, 0.878, 1.000], // #a2e0ff
            [0.635, 0.878, 1.000], // #a2e0ff
        ]
    } else {
        &[
            [0.30, 0.85, 1.00],
            [0.45, 0.55, 1.00],
            [0.75, 0.45, 1.00],
            [0.35, 0.95, 0.85],
        ]
    };
    palette[i % palette.len()]
}

/// Uploads a mesh's vertex + index buffers to the GPU.
fn upload_mesh(device: &wgpu::Device, mesh: &Mesh) -> GpuMesh {
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh-verts"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh-indices"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    GpuMesh {
        vertices,
        indices,
        index_count: mesh.indices.len() as u32,
    }
}

/// A trivial extension used only to keep the projection helpers together.
trait CursorAt {
    fn cursor_at(&self, ms: f64) -> (f32, f32);
}

impl CursorAt for Replay {
    fn cursor_at(&self, ms: f64) -> (f32, f32) {
        // Linear interpolation between the bracketing frames.
        let frames = &self.frames;
        if frames.is_empty() {
            return (0.0, 0.0);
        }
        match frames.binary_search_by(|f| f.ms.total_cmp(&ms)) {
            Ok(i) => (frames[i].x, frames[i].y),
            Err(0) => (frames[0].x, frames[0].y),
            Err(i) if i >= frames.len() => {
                let f = frames[frames.len() - 1];
                (f.x, f.y)
            }
            Err(i) => {
                let a = frames[i - 1];
                let b = frames[i];
                let span = (b.ms - a.ms) as f32;
                let t = if span > 0.0 {
                    ((ms - a.ms) as f32) / span
                } else {
                    0.0
                };
                (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
            }
        }
    }
}

#[cfg(test)]
mod decode_tests {
    use super::decode_image_rgba;

    /// Map covers are frequently JPEGs; the decoder must not be PNG-only.
    #[test]
    fn jpeg_cover_decodes() {
        let img = image::RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 16) as u8, 128])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&img)
            .unwrap();
        let (rgba, w, h) = decode_image_rgba(&jpeg).expect("jpeg decodes");
        assert_eq!((w, h), (16, 16));
        assert_eq!(rgba.len(), 16 * 16 * 4);
    }

    /// Indexed-palette PNGs previously returned None.
    #[test]
    fn indexed_png_decodes() {
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut png_bytes), 2, 2);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![255, 0, 0, 0, 255, 0]);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&[0, 1, 1, 0]).unwrap();
        }
        let (rgba, w, h) = decode_image_rgba(&png_bytes).expect("indexed png decodes");
        assert_eq!((w, h), (2, 2));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    }
}
