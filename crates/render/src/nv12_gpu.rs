//! The NV12 conversion done on the GPU instead of the CPU.
//!
//! Measured at 3840x2160/120, a frame cost 0.64 ms to build on the GPU and
//! then 10.84 ms to convert on the CPU — the renderer spent 97% of its time
//! waiting for a colour-space conversion. Doing it in a compute pass before
//! the readback moves that work onto hardware that is idle anyway and, as a
//! second effect, cuts what crosses the bus from 4 bytes per pixel to 1.5.
//!
//! The shader is a transcription of [`crate::nv12`] and the two are held to
//! producing identical bytes by a test that runs a frame through both.

use crate::renderer::READBACK_SLOTS;

/// Whether a frame size can go through the compute path. NV12 already needs
/// even dimensions; the shader additionally packs four horizontal pixels into
/// one 32-bit word, so the width must be a multiple of four for a write never
/// to straddle two rows. Everything else falls back to the CPU conversion.
pub fn gpu_supported(width: u32, height: u32) -> bool {
    crate::nv12::nv12_supported(width as usize, height as usize) && width % 4 == 0
}

/// Compute pipeline plus the per-slot buffers it writes into. Held by the
/// renderer only while NV12 readback is switched on.
pub struct Nv12Path {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    dims_buf: wgpu::Buffer,
    /// One storage buffer per readback slot: with three frames in flight a
    /// shared one would be overwritten while an older frame is still being
    /// copied out of it.
    storage: Vec<wgpu::Buffer>,
    binds: Vec<wgpu::BindGroup>,
    /// Bytes of one NV12 frame at the current size.
    pub len: u64,
}

impl Nv12Path {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        color_view: &wgpu::TextureView,
    ) -> Option<Nv12Path> {
        if !gpu_supported(width, height) {
            return None;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12"),
            source: wgpu::ShaderSource::Wgsl(include_str!("nv12.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("nv12"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let dims_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12-dims"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut me = Nv12Path {
            pipeline,
            layout,
            dims_buf,
            storage: Vec::new(),
            binds: Vec::new(),
            len: 0,
        };
        me.resize(device, width, height, color_view);
        Some(me)
    }

    /// Rebuilds the size-dependent half: the storage buffers and the bind
    /// groups, which name a colour view that a resize has replaced.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        color_view: &wgpu::TextureView,
    ) {
        self.len = crate::nv12::nv12_len(width as usize, height as usize) as u64;
        self.storage = (0..READBACK_SLOTS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nv12-storage"),
                    size: self.len,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            })
            .collect();
        self.binds = self
            .storage
            .iter()
            .map(|buf| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("nv12-bind"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(color_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.dims_buf.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
    }

    pub fn upload_dims(&self, queue: &wgpu::Queue, width: u32, height: u32) {
        queue.write_buffer(&self.dims_buf, 0, bytemuck::bytes_of(&[width, height, 0, 0]));
    }

    /// Appends the conversion and the copy into the mappable readback buffer
    /// to `encoder`, replacing what would otherwise be a texture-to-buffer
    /// copy of the full RGBA frame.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        readback: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) {
        let slot = slot % READBACK_SLOTS;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("nv12"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.binds[slot], &[]);
            // One invocation per 4x2 pixel block, in 8x8 workgroups.
            pass.dispatch_workgroups((width / 4).div_ceil(8), (height / 2).div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&self.storage[slot], 0, readback, 0, self.len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs an RGBA image through the compute shader and hands back the NV12
    /// bytes. Returns None when the machine has no usable adapter, so the
    /// test skips instead of failing on a headless box without a GPU.
    fn gpu_convert(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                    ..Default::default()
                })
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("nv12-test"),
                    ..Default::default()
                })
                .await
                .ok()?;

            // Stands in for the render target: same format, so the shader
            // reads exactly the bytes written here.
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("nv12-test-src"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
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
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );

            let view = tex.create_view(&Default::default());
            let path = Nv12Path::new(&device, width, height, &view)?;
            path.upload_dims(&queue, width, height);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("nv12-test-readback"),
                size: path.len,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            path.encode(&mut encoder, 0, &readback, width, height);
            let idx = queue.submit(Some(encoder.finish()));

            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(idx),
                    timeout: None,
                })
                .ok()?;
            let mapped = slice.get_mapped_range().ok()?;
            let out = mapped.to_vec();
            drop(mapped);
            readback.unmap();
            Some(out)
        })
    }

    /// A picture with every primary, both extremes, greys and a lot of
    /// arbitrary colour, so a wrong coefficient, a wrong shift or a swapped
    /// U/V shows up. The CPU converter's numbers were verified against
    /// ffmpeg's own output, so matching it exactly is what keeps the GPU
    /// path's colours right.
    fn test_image(width: usize, height: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let px = match i % 8 {
                    0 => [255, 0, 0],
                    1 => [0, 255, 0],
                    2 => [0, 0, 255],
                    3 => [255, 255, 255],
                    4 => [0, 0, 0],
                    5 => [128, 128, 128],
                    6 => [(x * 7 % 256) as u8, (y * 5 % 256) as u8, (i * 3 % 256) as u8],
                    _ => [(i % 251) as u8, (i * 13 % 241) as u8, (i * 29 % 239) as u8],
                };
                v.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        v
    }

    #[test]
    fn gpu_matches_cpu_byte_for_byte() {
        let (w, h) = (64usize, 32usize);
        let rgba = test_image(w, h);
        let Some(gpu) = gpu_convert(&rgba, w as u32, h as u32) else {
            eprintln!("no usable GPU adapter — skipping");
            return;
        };
        let mut cpu = vec![0u8; crate::nv12::nv12_len(w, h)];
        assert!(crate::nv12::rgba_to_nv12(&rgba, w, h, &mut cpu));
        assert_eq!(gpu.len(), cpu.len(), "frame size");

        let y_plane = w * h;
        let bad_y = (0..y_plane).find(|&i| gpu[i] != cpu[i]);
        assert!(
            bad_y.is_none(),
            "luma differs at {:?}: gpu {:?} cpu {:?}",
            bad_y,
            bad_y.map(|i| gpu[i]),
            bad_y.map(|i| cpu[i])
        );
        let bad_uv = (y_plane..cpu.len()).find(|&i| gpu[i] != cpu[i]);
        assert!(
            bad_uv.is_none(),
            "chroma differs at {:?}: gpu {:?} cpu {:?}",
            bad_uv.map(|i| i - y_plane),
            bad_uv.map(|i| gpu[i]),
            bad_uv.map(|i| cpu[i])
        );
    }

    /// Sizes the shader cannot address must be refused rather than rendered
    /// wrong — the video loop falls back to the CPU converter for those.
    #[test]
    fn unaddressable_sizes_are_refused() {
        assert!(gpu_supported(1920, 1080));
        assert!(gpu_supported(3840, 2160));
        assert!(gpu_supported(1080, 1920));
        assert!(!gpu_supported(1918, 1080), "width not a multiple of four");
        assert!(!gpu_supported(1920, 1079), "odd height has no chroma row");
    }
}
