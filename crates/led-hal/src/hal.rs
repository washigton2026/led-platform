//! The HAL facade — the sole implementor of [`ProtocolOutput`].
//!
//! Owns the compiled mapping, the device drivers, and the pre-allocated scratch. Per frame
//! it applies the mapping **once**, then hands each device only the universes it owns.
//!
//! ## NetworkGuard integration
//!
//! [`Hal::with_guard`] accepts any [`NetworkGuard`] implementation. Call `check()` once
//! at show-start (before the first `send_frame`) to enforce the WiFi-forbidden Hardware
//! Rule. In production use [`WifiBlockGuard`]; in tests use [`PermissiveGuard`].
//!
//! The guard is NOT called on every frame — enforcement is a show-start gate, not a
//! per-frame overhead. This keeps the hot path alloc-free and lock-free.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use led_core::{CompiledLayout, DeviceDriver, LogicalFrame, OutputError, ProtocolOutput, UniverseData};

use crate::calibration::Calibration;
use crate::metrics::MetricsEmitter;
use crate::network_guard::{NetworkGuard, NetworkPolicyError, PermissiveGuard};
use crate::observability::SpanCollector;

pub struct Hal {
    layout:   CompiledLayout,
    devices:  Vec<Arc<dyn DeviceDriver>>,
    scratch:  Mutex<Vec<UniverseData>>, // pre-sized once; reused every frame
    guard:    Box<dyn NetworkGuard>,    // called once at show-start, never in hot path
    /// Optional metrics emitter. When `Some`, records latency for every `send_frame`.
    /// Overhead: one `Instant::now()` + one atomic increment per frame — negligible.
    metrics:  Option<Arc<MetricsEmitter>>,
    /// Optional span collector. When `Some`, emits one span per `send_frame`.
    spans:    Option<Arc<SpanCollector>>,
    /// Node ID for distributed tracing (0 = single-node default).
    node_id:  u32,
    /// Per-output calibration (ADR-0019). Empty by default: with no device registered the
    /// fan-out is byte-identical to before, and the branch is per device, never per pixel.
    calibration: Calibration,
    /// Destination for calibrated bytes, sized once at startup (empty when no calibration).
    ///
    /// Why a second buffer instead of correcting `scratch` in place: `CompiledLayout::apply`
    /// only writes the targets covered by `frame.pixels` (it guards with `.get(id)` for short
    /// frames). Correcting in place would re-correct any uncovered target on every frame —
    /// compounding gamma and darkening those pixels frame after frame. Writing into a separate
    /// buffer makes the correction idempotent per frame. Allocated once; never on the hot path.
    cal_scratch: Mutex<Vec<UniverseData>>,
}

impl Hal {
    /// Create a `Hal` with the default [`PermissiveGuard`] (no WiFi enforcement).
    /// Suitable for tests, simulators, and non-hardware environments.
    pub fn new(layout: CompiledLayout, devices: Vec<Arc<dyn DeviceDriver>>) -> Self {
        Self::with_guard(layout, devices, PermissiveGuard)
    }

    /// Create a `Hal` with a custom [`NetworkGuard`].
    ///
    /// Call [`Hal::check_network`] once before starting a live show to enforce
    /// the guard's policy. If the guard returns an error, the show must not start.
    ///
    /// # Example
    /// ```ignore
    /// use led_hal::{Hal, WifiBlockGuard};
    ///
    /// let hal = Hal::with_guard(layout, devices, WifiBlockGuard);
    /// hal.check_network().expect("WiFi must be disabled before show start");
    /// ```
    pub fn with_guard(
        layout:  CompiledLayout,
        devices: Vec<Arc<dyn DeviceDriver>>,
        guard:   impl NetworkGuard + 'static,
    ) -> Self {
        let scratch = layout.make_scratch();
        Self {
            layout,
            devices,
            scratch: Mutex::new(scratch),
            guard: Box::new(guard),
            metrics: None,
            spans: None,
            node_id: 0,
            calibration: Calibration::new(),
            cal_scratch: Mutex::new(Vec::new()),
        }
    }

    /// Attach per-output calibration (ADR-0019). Gamma and brightness are folded into one
    /// 256-entry LUT per device at startup; the frame path then costs a single indexed read
    /// per channel, and **nothing at all** for devices with no calibration registered.
    ///
    /// Values arrive as plain `f32` on purpose: `led-hal` does not depend on
    /// `led-hardware-profile` — wiring a profile's `Calibration` to this is the app's job.
    pub fn with_calibration(mut self, calibration: Calibration) -> Self {
        // Size the calibrated-output buffer here, at startup — never on the frame path.
        if !calibration.is_empty() {
            self.cal_scratch = Mutex::new(self.layout.make_scratch());
        }
        self.calibration = calibration;
        self
    }

    /// The calibration currently attached.
    pub fn calibration(&self) -> &Calibration {
        &self.calibration
    }

    /// Attach a [`MetricsEmitter`] to this HAL. After attaching, every `send_frame`
    /// records its latency and frame count. Attach before the first frame.
    pub fn with_metrics(mut self, emitter: Arc<MetricsEmitter>) -> Self {
        self.metrics = Some(emitter);
        self
    }

    /// Access the attached metrics emitter (if any).
    pub fn metrics(&self) -> Option<&Arc<MetricsEmitter>> {
        self.metrics.as_ref()
    }

    /// Attach a [`SpanCollector`] for distributed tracing.
    /// After attaching, every `send_frame` emits a `"hal.send_frame"` span.
    pub fn with_spans(mut self, spans: Arc<SpanCollector>, node_id: u32) -> Self {
        self.spans   = Some(spans);
        self.node_id = node_id;
        self
    }

    /// Enforce the network policy. Must be called once before the first `send_frame`.
    ///
    /// Returns `Ok(())` if the show may proceed, or `Err(NetworkPolicyError)` if
    /// the guard blocks (e.g. WiFi is active). The caller should log and abort.
    ///
    /// # Hardware Rule
    /// WiFi is forbidden during live shows. Use `WifiBlockGuard` in production.
    /// This call is a no-op with `PermissiveGuard` (tests/simulator).
    pub fn check_network(&self) -> Result<(), NetworkPolicyError> {
        self.guard.check()
    }

    pub fn layout(&self) -> &CompiledLayout {
        &self.layout
    }
}

impl ProtocolOutput for Hal {
    fn send_frame(&self, frame: &LogicalFrame) -> Result<(), OutputError> {
        // Start latency measurement and span if attached.
        let t0 = self.metrics.as_ref().map(|_| Instant::now());
        let span_t0 = self.spans.as_ref().map(|_| Instant::now());

        let mut scratch = self.scratch.lock().expect("scratch poisoned");

        // (1) Apply the ONE mapping, exactly once, into pre-allocated scratch.
        self.layout.apply(frame, &mut scratch);

        // (2) Fan out: each device receives only the universes it owns. No re-mapping.
        //     Per-output calibration (ADR-0019) is applied here, on the device's own
        //     contiguous scratch slice — one branch per device, never per pixel, and skipped
        //     entirely when the device has no calibration.
        for dev in &self.devices {
            let range = self
                .layout
                .device_range(dev.id())
                .ok_or(OutputError::DeviceNotConnected(dev.id()))?;
            match self.calibration.lut(dev.id()) {
                // Uncalibrated device: byte-identical to the pre-ADR-0019 path.
                None => dev.send_physical(&scratch[range])?,
                // Calibrated: correct into the dedicated buffer (idempotent per frame), send that.
                Some(lut) => {
                    let mut cal = self.cal_scratch.lock().expect("cal scratch poisoned");
                    for i in range.clone() {
                        cal[i].data.copy_from_slice(&scratch[i].data);
                        lut.apply_in_place(&mut cal[i].data);
                    }
                    dev.send_physical(&cal[range])?;
                }
            }
        }

        // Record metrics (outside the scratch lock to minimise critical section).
        drop(scratch);
        if let (Some(m), Some(t)) = (&self.metrics, t0) {
            m.record_frame(t.elapsed().as_micros() as u64);
        }
        if let (Some(s), Some(t)) = (&self.spans, span_t0) {
            s.record(crate::observability::Span {
                name:        "hal.send_frame",
                node_id:     self.node_id,
                start_us:    0,
                duration_us: t.elapsed().as_micros() as u64,
                tags:        "layer=hal",
            });
        }

        Ok(())
    }

    fn universe_count(&self) -> u16 {
        self.layout.universe_count()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_guard::{PermissiveGuard, WifiBlockGuard};
    use crate::SimulatorDevice;
    use led_core::{CompiledLayout, DeviceSpec, RgbOrder};

    fn minimal_layout_and_device() -> (CompiledLayout, Arc<SimulatorDevice>) {
        let n = 4usize;
        // SimulatorDevice::new returns Arc<Self>; universe 1 owns n pixels
        let sim = SimulatorDevice::new(0, &[1u16]); // device 0, universe 1
        let layout = CompiledLayout::linear(n, &[DeviceSpec { id: 0, universes: 1 }], RgbOrder::Rgb);
        (layout, sim)
    }

    #[test]
    fn hal_new_uses_permissive_guard() {
        let (layout, sim) = minimal_layout_and_device();
        let hal = Hal::new(layout, vec![sim]);
        assert!(hal.check_network().is_ok(), "Hal::new must use PermissiveGuard");
    }

    #[test]
    fn hal_with_guard_permissive_always_ok() {
        let (layout, sim) = minimal_layout_and_device();
        let hal = Hal::with_guard(layout, vec![sim], PermissiveGuard);
        assert!(hal.check_network().is_ok());
    }

    #[test]
    fn hal_with_guard_wifi_block_does_not_panic() {
        // WifiBlockGuard returns Ok or a typed error — never panics.
        let (layout, sim) = minimal_layout_and_device();
        let hal = Hal::with_guard(layout, vec![sim], WifiBlockGuard);
        let result = hal.check_network();
        match result {
            Ok(()) => { /* no Wi-Fi active or not detectable */ }
            Err(e) => {
                let s = e.to_string();
                assert!(
                    s.contains("CRITICAL") || s.contains("WARNING"),
                    "error must contain CRITICAL or WARNING: {s}"
                );
            }
        }
    }

    #[test]
    fn hal_check_network_is_not_called_per_frame() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc as StdArc;

        struct CountingGuard(StdArc<AtomicU32>);
        impl NetworkGuard for CountingGuard {
            fn check(&self) -> Result<(), NetworkPolicyError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            fn name(&self) -> &'static str { "CountingGuard" }
        }

        let counter = StdArc::new(AtomicU32::new(0));
        let (layout, sim) = minimal_layout_and_device();
        let hal = Hal::with_guard(layout, vec![sim.clone()], CountingGuard(counter.clone()));

        hal.check_network().unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1, "check_network called once");

        // Send 5 frames — guard must NOT be called again
        let frame = led_core::LogicalFrame::new(
            vec![led_core::PixelColor::default(); 4],
            0,
        );
        for _ in 0..5 { let _ = hal.send_frame(&frame); }

        assert_eq!(counter.load(Ordering::SeqCst), 1, "guard must not be called per-frame");
    }
}
