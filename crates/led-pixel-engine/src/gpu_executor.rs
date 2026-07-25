//! Real wgpu GPU executor for [`ComputeKernel`]s.
//!
//! Activated only with `--features gpu`. Skips gracefully when no adapter is
//! available — a missing GPU is an environment limitation, never a test failure.
//!
//! The CPU [`ComputeEffect`](super::compute::ComputeEffect) is always the reference
//! and test oracle; this module dispatches the same WGSL kernel to the GPU and
//! verifies parity in tests.

use bytemuck::{Pod, Zeroable};
use led_core::PixelColor;

use crate::effect::Vec3;

// ── GPU context ────────────────────────────────────────────────────────────────

/// One-time GPU initialisation result. `None` means no adapter — caller must
/// fall back to the CPU path silently (no panic, no test failure).
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue:  wgpu::Queue,
}

impl GpuContext {
    /// Try to acquire an adapter. Returns `None` on headless / no-GPU systems.
    pub fn try_init() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::None,
            compatible_surface:     None,       // headless — no window
            force_fallback_adapter: false,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label:    Some("lumyx-gpu"),
                required_features: wgpu::Features::empty(),
                required_limits:   wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;
        Some(GpuContext { device, queue })
    }
}

// ── Uniform layout (mirrors WGSL `Params`) ────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    scale:  f32,
    speed:  f32,
    time_s: f32,
    count:  u32,
}

// ── GpuPlasmaExecutor ─────────────────────────────────────────────────────────

/// GPU executor for the Plasma kernel. Pre-allocates all buffers for `capacity`
/// pixels; re-used every frame (zero per-frame allocation).
pub struct GpuPlasmaExecutor {
    device:        wgpu::Device,
    queue:         wgpu::Queue,
    pipeline:      wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params_buf:    wgpu::Buffer,        // uniform  — Params (16 bytes)
    pos_buf:       wgpu::Buffer,        // storage read — [vec3<f32>] padded to vec4
    out_buf:       wgpu::Buffer,        // storage read_write — [u32]
    staging_buf:   wgpu::Buffer,        // MAP_READ staging
    capacity:      usize,
    scale:         f32,
    speed:         f32,
}

impl GpuPlasmaExecutor {
    /// Compile the pipeline and allocate buffers for `capacity` pixels.
    /// Returns `None` when the system has no usable GPU adapter.
    pub fn try_new(capacity: usize, scale: f32, speed: f32) -> Option<Self> {
        let GpuContext { device, queue } = GpuContext::try_init()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("plasma_wgsl"),
            source: wgpu::ShaderSource::Wgsl(crate::compute::PLASMA_WGSL.into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label:   Some("plasma_bgl"),
                entries: &[
                    // binding 0 — params uniform
                    wgpu::BindGroupLayoutEntry {
                        binding:    0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty:         wgpu::BindingType::Buffer {
                            ty:                 wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size:   None,
                        },
                        count: None,
                    },
                    // binding 1 — positions (read-only storage)
                    wgpu::BindGroupLayoutEntry {
                        binding:    1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty:         wgpu::BindingType::Buffer {
                            ty:                 wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size:   None,
                        },
                        count: None,
                    },
                    // binding 2 — out_rgb (read_write storage)
                    wgpu::BindGroupLayoutEntry {
                        binding:    2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty:         wgpu::BindingType::Buffer {
                            ty:                 wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size:   None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:                Some("plasma_pl"),
                bind_group_layouts:   &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:       Some("plasma_pipeline"),
            layout:      Some(&pipeline_layout),
            module:      &shader,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Uniform: 16 bytes (Params)
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("params"),
            size:               std::mem::size_of::<Params>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Positions: each vec3<f32> padded to 16 bytes (vec4 alignment in WGSL)
        let pos_stride = 16usize; // vec3 padded to vec4 in storage buffers
        let pos_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("positions"),
            size:               (capacity * pos_stride) as u64,
            usage:              wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Output: one u32 (0xRRGGBB) per pixel
        let out_size = (capacity * 4) as u64;
        let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("out_rgb"),
            size:               out_size,
            usage:              wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("staging"),
            size:               out_size,
            usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Some(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            params_buf,
            pos_buf,
            out_buf,
            staging_buf,
            capacity,
            scale,
            speed,
        })
    }

    /// Render `positions` at `time_ms` into `out`. Falls back silently if the
    /// pixel count exceeds `capacity` (caller should size correctly at init).
    pub fn render(&self, time_ms: u64, positions: &[Vec3], out: &mut [PixelColor]) {
        let n = positions.len().min(self.capacity);
        if n == 0 { return; }

        // 1. Upload params uniform
        let params = Params {
            scale:  self.scale,
            speed:  self.speed,
            time_s: time_ms as f32 / 1000.0,
            count:  n as u32,
        };
        self.queue.write_buffer(&self.params_buf, 0, bytemuck::bytes_of(&params));

        // 2. Upload positions — pad each vec3 to 16 bytes
        let mut pos_bytes = vec![0u8; n * 16];
        for (i, p) in positions[..n].iter().enumerate() {
            let base = i * 16;
            pos_bytes[base..base + 4].copy_from_slice(&p.x.to_le_bytes());
            pos_bytes[base + 4..base + 8].copy_from_slice(&p.y.to_le_bytes());
            pos_bytes[base + 8..base + 12].copy_from_slice(&p.z.to_le_bytes());
            // bytes [12..16] = padding zero (already zeroed)
        }
        self.queue.write_buffer(&self.pos_buf, 0, &pos_bytes);

        // 3. Build bind group and dispatch
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:  Some("plasma_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.pos_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.out_buf.as_entire_binding() },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("plasma_enc"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label:              Some("plasma_pass"),
                timestamp_writes:   None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(n.div_ceil(64) as u32, 1, 1);
        }
        // Copy output → staging
        encoder.copy_buffer_to_buffer(
            &self.out_buf, 0,
            &self.staging_buf, 0,
            (n * 4) as u64,
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // 4. Readback (blocking)
        let slice = self.staging_buf.slice(..(n * 4) as u64);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.device.poll(wgpu::Maintain::Wait);
        let _ = rx.recv();

        let data = slice.get_mapped_range();
        let packed: &[u32] = bytemuck::cast_slice(&data);
        for (i, &rgb) in packed[..n].iter().enumerate() {
            out[i] = PixelColor::rgb(
                ((rgb >> 16) & 0xFF) as u8,
                ((rgb >>  8) & 0xFF) as u8,
                ( rgb        & 0xFF) as u8,
            );
        }
        drop(data);
        self.staging_buf.unmap();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::{ComputeEffect, Plasma};
    use crate::effect::{Effect, Vec3};

    /// Skip helper: returns true when no GPU adapter is available.
    fn no_gpu() -> bool {
        GpuContext::try_init().is_none()
    }

    #[test]
    fn gpu_executor_init_does_not_hang() {
        // Verifies TD-004 fix: wgpu 22.x no longer blocks indefinitely on
        // Metal headless. On systems without a GPU, try_init() returns None
        // quickly — it does NOT hang. The test passes either way.
        let _ = GpuContext::try_init(); // must return (Some or None) within seconds
    }

    #[test]
    fn gpu_plasma_parity_with_cpu() {
        if no_gpu() {
            eprintln!("skip: no GPU adapter — parity test requires hardware");
            return;
        }

        let scale = 0.5_f32;
        let speed = 1.0_f32;
        let positions = vec![
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(0.5, 0.5, 0.0),
        ];
        let n = positions.len();
        let time_ms = 0u64;

        // CPU reference
        let cpu_fx = ComputeEffect::new(Plasma { scale, speed });
        let mut cpu_out = vec![PixelColor::default(); n];
        cpu_fx.render(time_ms, &positions, &mut cpu_out);

        // GPU executor
        let gpu = GpuPlasmaExecutor::try_new(n, scale, speed)
            .expect("GPU was available but executor init failed");
        let mut gpu_out = vec![PixelColor::default(); n];
        gpu.render(time_ms, &positions, &mut gpu_out);

        // Parity: allow ±1 per channel for f32 rounding differences
        for (i, (c, g)) in cpu_out.iter().zip(gpu_out.iter()).enumerate() {
            let dr = (c.r as i16 - g.r as i16).abs();
            let dg = (c.g as i16 - g.g as i16).abs();
            let db = (c.b as i16 - g.b as i16).abs();
            assert!(
                dr <= 1 && dg <= 1 && db <= 1,
                "pixel {i}: CPU={c:?} GPU={g:?} delta=({dr},{dg},{db})"
            );
        }
    }

    #[test]
    fn gpu_plasma_deterministic() {
        if no_gpu() {
            eprintln!("skip: no GPU adapter");
            return;
        }
        let positions: Vec<Vec3> = (0..32).map(|i| Vec3::new(i as f32 * 0.1, 0.0, 0.0)).collect();
        let n = positions.len();
        let gpu = GpuPlasmaExecutor::try_new(n, 0.5, 1.0).unwrap();

        let mut a = vec![PixelColor::default(); n];
        let mut b = vec![PixelColor::default(); n];
        gpu.render(500, &positions, &mut a);
        gpu.render(500, &positions, &mut b);
        assert_eq!(a, b, "same time_ms ⇒ identical GPU output");
    }
}
