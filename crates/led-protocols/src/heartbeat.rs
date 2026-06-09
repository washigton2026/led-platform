//! Keep-alive heartbeat — the rule that prevents your rig from going dark.
//!
//! ## The invariant
//! Controllers (WLED, FPP, Falcon) enter safe mode after ~2.5 s of silence. Something
//! valid must keep flowing whether the show is playing, paused, or stopped. A **zeroed**
//! frame is NOT a valid heartbeat — it blacks out the rig. The heartbeat fires at 800 ms
//! (safely below the 2.5 s safe-mode threshold with comfortable margin) and only sends if
//! a real frame has been registered via `update()`.
//!
//! ## Health status
//! `health(last_sent_ms, now_ms)` translates silence duration into an actionable status for
//! the UI — yellow warning before controllers drop out, red critical after they likely have.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use led_core::UniverseData;

/// Gap thresholds that mirror the LUMYX_GOSL hardware rules.
pub const WARN_GAP_MS:  u64 = 2_000;
pub const CRIT_GAP_MS:  u64 = 2_500;
/// Our heartbeat interval: below the warning threshold with comfortable margin.
pub const HEARTBEAT_MS: u64 = 800;

// ─── Health status ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Frames flowing normally (gap < 2 s).
    Ok,
    /// 2.0–2.5 s gap — "safe mode risk" yellow banner.
    Warning,
    /// > 2.5 s gap — controller has likely dropped out, red banner + reconnect button.
    Critical,
}

/// Classify time-since-last-send into a `HealthStatus`.
///
/// `last_sent_ms` and `now_ms` are in the same epoch (e.g. `Instant::elapsed().as_millis()`).
#[inline]
pub fn health(last_sent_ms: u64, now_ms: u64) -> HealthStatus {
    match now_ms.saturating_sub(last_sent_ms) {
        g if g < WARN_GAP_MS => HealthStatus::Ok,
        g if g < CRIT_GAP_MS => HealthStatus::Warning,
        _                    => HealthStatus::Critical,
    }
}

// ─── Heartbeat ────────────────────────────────────────────────────────────────

/// Keeps the rig alive by resending the last **valid** frame at `HEARTBEAT_MS` intervals.
///
/// Usage:
/// ```ignore
/// let hb = Heartbeat::new();
/// hb.update(&my_universe_data);                // call from render path each frame
/// let _guard = hb.start(HEARTBEAT_MS, sender); // spawns the keep-alive task
/// ```
pub struct Heartbeat {
    last_valid: Arc<RwLock<Option<Arc<Vec<UniverseData>>>>>,
}

impl Heartbeat {
    pub fn new() -> Self {
        Self { last_valid: Arc::new(RwLock::new(None)) }
    }

    /// Update the stored frame (called from the render path, every frame).
    /// This is the ONLY way the heartbeat ever gets something to send.
    pub fn update(&self, universes: &[UniverseData]) {
        let arc = Arc::new(universes.to_vec());
        *self.last_valid.write().unwrap() = Some(arc);
    }

    /// Spawn the keep-alive task. `send_fn` is called every `interval_ms` if a frame exists.
    /// Aborts when the returned `JoinHandle` is dropped. Takes `&self` so `update()` can
    /// still be called on the same handle after the task is running.
    pub fn start<F>(&self, interval_ms: u64, send_fn: F) -> tokio::task::JoinHandle<()>
    where
        F: Fn(&[UniverseData]) + Send + 'static,
    {
        let last_valid = Arc::clone(&self.last_valid);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let frame = last_valid.read().unwrap().clone();
                if let Some(ref universes) = frame {
                    send_fn(universes); // NEVER a zeroed frame — only what update() gave us
                }
                // If no frame has been registered yet, we stay silent (don't send zeros).
            }
        })
    }
}

impl Default for Heartbeat {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_thresholds_match_gosl_rules() {
        assert_eq!(health(1000, 1000), HealthStatus::Ok,      "0 ms gap");
        assert_eq!(health(0, 1999),    HealthStatus::Ok,      "1999 ms gap");
        assert_eq!(health(0, 2000),    HealthStatus::Warning,  "2000 ms gap = Warning");
        assert_eq!(health(0, 2499),    HealthStatus::Warning,  "2499 ms gap = Warning");
        assert_eq!(health(0, 2500),    HealthStatus::Critical, "2500 ms gap = Critical");
        assert_eq!(health(0, 9999),    HealthStatus::Critical, "long gap = Critical");
    }

    #[test]
    fn health_does_not_panic_on_backwards_clock() {
        // now < last_sent (clock jumped back) — saturating_sub returns 0 → Ok
        assert_eq!(health(5000, 1000), HealthStatus::Ok);
    }
}
