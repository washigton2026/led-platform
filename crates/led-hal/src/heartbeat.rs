//! Keep-alive. Controllers enter safe mode after ~2.5 s of silence, so *something valid*
//! must keep flowing. The rule that matters: resend the last **valid** frame — never a
//! zeroed frame, which would black out the rig.
//!
//! [`Heartbeat::beat`] is the synchronous primitive (call it from any timer).
//! [`Heartbeat::spawn`] runs it on an independent thread at a fixed interval, *regardless*
//! of sequencer play/pause state — the real ≥1 Hz keep-alive.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use led_core::{LogicalFrame, OutputError, ProtocolOutput};

#[derive(Default)]
pub struct Heartbeat {
    last_valid: Mutex<Option<LogicalFrame>>,
}

impl Heartbeat {
    pub fn new() -> Self {
        Self { last_valid: Mutex::new(None) }
    }

    /// Record a frame the sequencer actually produced. Only real frames land here.
    pub fn record(&self, frame: &LogicalFrame) {
        *self.last_valid.lock().unwrap() = Some(frame.clone());
    }

    /// Emit a keep-alive. Resends the last valid frame if there is one (`Ok(true)`), or
    /// sends nothing (`Ok(false)`) — it NEVER fabricates a zeroed/blackout frame.
    pub fn beat(&self, out: &dyn ProtocolOutput) -> Result<bool, OutputError> {
        match &*self.last_valid.lock().unwrap() {
            Some(frame) => {
                out.send_frame(frame)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Start an independent thread that beats every `interval`, until the returned handle is
    /// stopped/dropped. Independent of any sequencer state.
    pub fn spawn(
        self: Arc<Self>,
        out: Arc<dyn ProtocolOutput>,
        interval: Duration,
    ) -> HeartbeatHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                std::thread::sleep(interval);
                // A transport hiccup must not kill the heartbeat; just keep going.
                let _ = self.beat(&*out);
            }
        });
        HeartbeatHandle { stop, handle: Some(handle) }
    }
}

/// Stops the heartbeat thread on `stop()` or when dropped.
pub struct HeartbeatHandle {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl HeartbeatHandle {
    pub fn stop(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for HeartbeatHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}
