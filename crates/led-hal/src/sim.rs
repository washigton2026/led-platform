//! A virtual device: accepts physical frames and mirrors them for inspection. This is how
//! the whole stack above the HAL is exercised with zero hardware. `send_physical` is
//! allocation-free (it copies into a pre-sized mirror), so it is safe on the hot path.
//!
//! It also implements [`IDevice`] so the lifecycle/management plane is exercised too.

use std::sync::{Arc, Mutex};

use led_core::{
    DeviceConfig, DeviceDriver, DeviceId, DeviceStatus, IDevice, OutputError, UniverseData,
};

struct SimState {
    connected: bool,
    frames_sent: u64,
    config: DeviceConfig,
    firmware_updates: u64,
    mirror: Vec<UniverseData>, // pre-sized copy of the last received universes
}

pub struct SimulatorDevice {
    id: DeviceId,
    inner: Mutex<SimState>,
}

impl SimulatorDevice {
    /// Create a simulator that owns exactly the given universe numbers (use
    /// `layout.device_universes(id)`).
    pub fn new(id: DeviceId, universes: &[u16]) -> Arc<Self> {
        let mirror = universes
            .iter()
            .map(|&u| UniverseData { universe: u, data: vec![0u8; led_core::UNIVERSE_SIZE] })
            .collect();
        Arc::new(Self {
            id,
            inner: Mutex::new(SimState {
                connected: true,
                frames_sent: 0,
                config: DeviceConfig::default(),
                firmware_updates: 0,
                mirror,
            }),
        })
    }

    pub fn frames_sent(&self) -> u64 {
        self.inner.lock().unwrap().frames_sent
    }

    pub fn firmware_updates(&self) -> u64 {
        self.inner.lock().unwrap().firmware_updates
    }

    pub fn config(&self) -> DeviceConfig {
        self.inner.lock().unwrap().config.clone()
    }

    /// Read one received channel value (for assertions).
    pub fn channel(&self, universe: u16, ch: usize) -> Option<u8> {
        let s = self.inner.lock().unwrap();
        s.mirror
            .iter()
            .find(|u| u.universe == universe)
            .and_then(|u| u.data.get(ch).copied())
    }
}

impl DeviceDriver for SimulatorDevice {
    fn id(&self) -> DeviceId {
        self.id
    }

    fn send_physical(&self, universes: &[UniverseData]) -> Result<(), OutputError> {
        let mut s = self.inner.lock().unwrap();
        s.frames_sent += 1;
        for incoming in universes {
            // copy_from_slice keeps capacity — no allocation on the hot path.
            if let Some(slot) = s.mirror.iter_mut().find(|u| u.universe == incoming.universe) {
                slot.data.copy_from_slice(&incoming.data);
            }
        }
        Ok(())
    }

    fn status(&self) -> DeviceStatus {
        let s = self.inner.lock().unwrap();
        DeviceStatus { connected: s.connected, frames_sent: s.frames_sent, last_send_ms: 0 }
    }
}

impl IDevice for SimulatorDevice {
    fn connect(&self) -> Result<(), OutputError> {
        self.inner.lock().unwrap().connected = true;
        Ok(())
    }

    fn disconnect(&self) {
        self.inner.lock().unwrap().connected = false;
    }

    fn configure(&self, cfg: &DeviceConfig) -> Result<(), OutputError> {
        self.inner.lock().unwrap().config = cfg.clone();
        Ok(())
    }

    fn reboot(&self) -> Result<(), OutputError> {
        let mut s = self.inner.lock().unwrap();
        s.frames_sent = 0;
        Ok(())
    }

    fn update_firmware(&self, image: &[u8]) -> Result<(), OutputError> {
        let mut s = self.inner.lock().unwrap();
        // Safety rule (led-hal/references/firmware.md): never flash a live device.
        if s.connected {
            return Err(OutputError::Transport(
                "refusing firmware update: device is live; disconnect it first".into(),
            ));
        }
        if image.is_empty() {
            return Err(OutputError::Transport("empty firmware image".into()));
        }
        s.firmware_updates += 1;
        Ok(())
    }
}
