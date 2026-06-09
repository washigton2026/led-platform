//! # led-protocols — wire edge of the LUMYX LED platform
//!
//! Turns `UniverseData` (physical-space channel bytes, mapped by the HAL) into packets
//! on the wire. Lives at the final output stage; the logical→physical mapping has already
//! happened upstream.
//!
//! ## Modules
//!
//! | Module | What it does |
//! |---|---|
//! | [`packet`] | E1.31 sACN byte layout — `build_data_packet`, wire accessors |
//! | [`device`] | Synchronous `SacnDevice` ([`DeviceDriver`]) — unicast + multicast |
//! | [`artnet`] | `ArtPoll`/`ArtPollReply` source-conflict detection |
//! | [`sender`] | Async parallel sender — one persistent tokio task per universe |
//! | [`heartbeat`] | 800 ms keep-alive; `HealthStatus`; never sends zeros |
//! | [`pool`] | Pre-allocated 638-byte buffer pool for zero-alloc hot paths |
//!
//! ## Non-negotiable rules (SKILL.md / LUMYX_GOSL)
//! - Sequence numbers are **per-universe**, wrapping — never a shared global counter.
//! - One universe per UDP datagram.
//! - Keep-alive fires at ≤ 800 ms regardless of sequencer state. A zeroed frame is not a heartbeat.
//! - Source conflict checked at startup; refuses to send on overlap, naming the other IP.
//! - WiFi is **unsupported** for live shows (cabled only).
//! - Zero allocations on the send path (pre-sized buffers).

pub mod artnet;
pub mod device;
pub mod heartbeat;
pub mod packet;
pub mod pool;
pub mod sender;

pub use artnet::{find_conflicts, ArtPollReply, ConflictReport};
pub use device::{multicast_addr, SacnDevice};
pub use heartbeat::{health, HealthStatus, Heartbeat, HEARTBEAT_MS};
pub use pool::BufferPool;
pub use sender::{FrameSlice, ParallelSender, UniverseState};
