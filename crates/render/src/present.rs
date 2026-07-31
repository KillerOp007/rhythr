//! Presenting rendered frames straight to a window surface — the live
//! path of the Analyze window. The frame is still rendered into the
//! calibrated offscreen `color_tex` (identical pixels to every export);
//! this module only adds the final hop: a blit onto the swapchain image,
//! letterboxed to preserve aspect, followed by `present()`.
//!
//! Color management: the render pipelines write sRGB-ENCODED bytes into
//! an `Rgba8Unorm` target (that is what the PNG export ships verbatim).
//! A non-sRGB surface format therefore passes bytes through 1:1; an sRGB
//! surface format would re-encode, so the blit shader linearizes first
//! when (and only when) the surface view is sRGB.

use crate::renderer::Renderer;
use crate::Error;

const BLIT_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // Fullscreen triangle: covers the viewport with 3 vertices.
    var out: VsOut;
    let x = f32(i32(i / 2u)) * 4.0 - 1.0;
    let y = f32(i32(i % 2u)) * 4.0 - 1.0;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var frame_tex: texture_2d<f32>;
@group(0) @binding(1) var frame_samp: sampler;

// x: 1.0 => the target view is sRGB and re-encodes: hand it linear.
// (A vec4, not a struct with vec3 padding — that would align to 32 B
// while the CPU side uploads 16.)
@group(0) @binding(2) var<uniform> params: vec4<f32>;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(frame_tex, frame_samp, in.uv);
    if (params.x > 0.5) {
        return vec4<f32>(srgb_to_linear(c.rgb), 1.0);
    }
    return vec4<f32>(c.rgb, 1.0);
}
"#;

/// Blits the offscreen frame onto a window surface. Owns the surface and
/// its configuration; rebuild via [`Presenter::resize`] on window resizes.
pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params_buf: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

fn pick_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    // Prefer a non-sRGB format: our bytes are already sRGB-encoded and
    // then pass through untouched, exactly like the PNG/video exports.
    caps.formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .unwrap_or(caps.formats[0])
}

impl Presenter {
    /// `surface` must have been created from the target window (on the
    /// main thread — macOS/Metal panics otherwise) with the same
    /// `wgpu::Instance` the renderer was built against.
    pub fn new(
        renderer: &Renderer,
        surface: wgpu::Surface<'static>,
        win_w: u32,
        win_h: u32,
    ) -> Result<Presenter, Error> {
        let device = renderer.device();
        let caps = surface.get_capabilities(renderer.adapter());
        if caps.formats.is_empty() {
            return Err(Error::Device("surface has no supported formats".into()));
        }
        let format = pick_format(&caps);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: win_w.max(1),
            height: win_h.max(1),
            // Fifo is available everywhere and blocks in get_current_texture
            // until a vblank slot frees up — natural vsync pacing.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit-layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit-pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blit-params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let linearize = if format.is_srgb() { 1.0f32 } else { 0.0 };
        renderer
            .queue()
            .write_buffer(&params_buf, 0, bytemuck::bytes_of(&[linearize, 0.0, 0.0, 0.0f32]));
        let bind = Self::make_bind(device, &layout, renderer, &sampler, &params_buf);
        Ok(Presenter {
            surface,
            config,
            pipeline,
            layout,
            sampler,
            params_buf,
            bind,
        })
    }

    fn make_bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        renderer: &Renderer,
        sampler: &wgpu::Sampler,
        params_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let view = renderer.color_view();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit-bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        })
    }

    /// Window size changed: reconfigure the swapchain. Call
    /// [`Presenter::rebind`] after the RENDER size changed too.
    pub fn resize(&mut self, renderer: &Renderer, win_w: u32, win_h: u32) {
        self.config.width = win_w.max(1);
        self.config.height = win_h.max(1);
        self.surface.configure(renderer.device(), &self.config);
    }

    /// The offscreen frame texture was recreated (render size changed) —
    /// the bind group must point at the new texture.
    pub fn rebind(&mut self, renderer: &Renderer) {
        self.bind = Self::make_bind(
            renderer.device(),
            &self.layout,
            renderer,
            &self.sampler,
            &self.params_buf,
        );
    }

    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Blits the current offscreen frame to the swapchain, letterboxed to
    /// the frame's aspect, and presents. Blocks until the compositor has
    /// a slot (Fifo) — this IS the frame pacing. Returns false when the
    /// frame was skipped (occluded/outdated): no vsync block happened, so
    /// the caller must pace itself or it spins at an uncapped rate.
    pub fn present_frame(&self, renderer: &Renderer) -> Result<bool, Error> {
        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match self.surface.get_current_texture() {
            Cst::Success(f) | Cst::Suboptimal(f) => f,
            Cst::Outdated | Cst::Lost => {
                // A resize is in flight; reconfigure and skip this frame.
                self.surface.configure(renderer.device(), &self.config);
                return Ok(false);
            }
            Cst::Timeout | Cst::Occluded => return Ok(false),
            Cst::Validation => {
                return Err(Error::Device("surface validation failed".into()));
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let (fw, fh) = renderer.dimensions();
        let (sw, sh) = (self.config.width as f32, self.config.height as f32);
        let scale = (sw / fw as f32).min(sh / fh as f32);
        let vw = (fw as f32 * scale).max(1.0);
        let vh = (fh as f32 * scale).max(1.0);
        let vx = (sw - vw) * 0.5;
        let vy = (sh - vh) * 0.5;

        let mut enc = renderer
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blit") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.027,
                            b: 0.04,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
            pass.draw(0..3, 0..1);
        }
        renderer.queue().submit([enc.finish()]);
        renderer.queue().present(frame);
        Ok(true)
    }

    /// Where the frame lands inside the window (letterbox rect, physical
    /// px) — the overlay layer aligns itself to this.
    pub fn frame_rect(&self, renderer: &Renderer) -> (f32, f32, f32, f32) {
        let (fw, fh) = renderer.dimensions();
        let (sw, sh) = (self.config.width as f32, self.config.height as f32);
        let scale = (sw / fw as f32).min(sh / fh as f32);
        let vw = fw as f32 * scale;
        let vh = fh as f32 * scale;
        ((sw - vw) * 0.5, (sh - vh) * 0.5, vw, vh)
    }
}
