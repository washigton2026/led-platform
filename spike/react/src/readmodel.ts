// Fixture matching the REAL contract: crates/led-readmodel/src/lib.rs (ReadModel::to_json).
// The panel consumes this so both prototypes render the same, comparable data.

export type Health = 'ok' | 'warning' | 'critical'

export interface DeviceView {
  id: number
  connected: boolean
  frames_sent: number
  last_send_ms: number
}

export interface ReadModel {
  health: Health
  devices: DeviceView[]
  metrics: { frames: number; drops: number; beats: number; p50_us: number; p99_us: number }
  discovery: { responded: string[]; missing: string[] } | null
}

export const fixture = (health: Health): ReadModel => ({
  health,
  devices: [
    { id: 0, connected: true, frames_sent: 42, last_send_ms: 0 },
    { id: 1, connected: health !== 'critical', frames_sent: 41, last_send_ms: 800 },
  ],
  metrics: { frames: 100, drops: 3, beats: 5, p50_us: 120, p99_us: 4100 },
  discovery: null,
})
