# GPU compute effects — the executor design

The `compute` module ships a portable per-pixel kernel model. The same kernel runs two ways:

- **CPU executor** (`ComputeEffect<K>`, today): maps the kernel over all pixels; deterministic,
  allocation-free, tested. Plugs into the render→send pipeline like any `Effect`.
- **GPU executor** (wgpu, behind a `gpu` cargo feature; hardware-gated): dispatches the
  *same* kernel as a WGSL `@compute @workgroup_size(64)` shader (e.g. `PLASMA_WGSL`).

Authoring the kernel once as a pure per-pixel function (and once as WGSL that computes the
identical value) is what makes "move this effect to the GPU" wiring rather than a rewrite —
and lets the CPU reference be the GPU shader's test oracle.

## Why feature-gate the GPU path

`wgpu` is a large dependency and needs a real adapter (Metal/Vulkan/DX). The default build
stays std-only and the default `cargo test --workspace` stays fast and 100% green. Real-GPU
execution and its test live behind `--features gpu` and skip gracefully when no adapter is
present — a GPU absence is an environment limitation, never a failed gate.

```toml
# led-pixel-engine/Cargo.toml (when enabling the GPU path)
[features]
gpu = ["dep:wgpu", "dep:pollster", "dep:bytemuck"]

[dependencies]
wgpu     = { version = "...", optional = true }
pollster = { version = "...", optional = true }   # block_on for setup
bytemuck = { version = "...", optional = true }   # POD <-> bytes
```

## Executor sketch (`#[cfg(feature = "gpu")]`)

```rust
// 1. Adapter — skip the GPU path (not the test) if none is available.
let instance = wgpu::Instance::default();
let adapter = pollster::block_on(instance.request_adapter(&Default::default()));
let Some(adapter) = adapter else { return /* no GPU here */ };
let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None))?;

// 2. Module from the SAME source the CPU reference mirrors.
let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("plasma"),
    source: wgpu::ShaderSource::Wgsl(PLASMA_WGSL.into()),
});

// 3. Buffers: params (uniform), positions (storage, read), out_rgb (storage, read_write) +
//    a MAP_READ staging buffer for readback. Pre-allocate ONCE; reuse every frame.
// 4. Bind group + compute pipeline (entry point "main").
// 5. Per frame: write params/time; dispatch ceil(count / 64) workgroups; copy out_rgb to
//    staging; map_async + poll; unpack u32 (0xRRGGBB) into PixelColor.
```

## Budget rationale (master §6)

CPU frame for ~50k px targets ≤1 ms; past that, GPU compute targets ≤0.5 ms for ~100k px and
the knob to tune is `workgroup_size(64)`. The handoff to the device is unchanged — the GPU
executor produces the same `LogicalFrame` the triple buffer carries to the HAL.

## Test strategy

- **Always green:** the CPU reference (`ComputeEffect<Plasma>`) is tested for determinism and
  known values — this also validates the algorithm the WGSL encodes.
- **`--features gpu` (hardware):** dispatch `PLASMA_WGSL`, read back, and assert GPU output
  equals the CPU reference within a tiny tolerance (rounding). Skipped if no adapter.

## Checklist (when wiring the GPU executor)

- [ ] `gpu` feature optional; default build/test stays std-only and green
- [ ] No adapter ⇒ skip the GPU path, never fail the test
- [ ] All GPU buffers pre-allocated once; per-frame path has no new allocations
- [ ] GPU output validated against the CPU reference (the WGSL's oracle)
- [ ] `workgroup_size(64)`; dispatch `ceil(count/64)`; guard `i >= count` in the shader
