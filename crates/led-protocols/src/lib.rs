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
//! | [`ddp`]    | DDP (Distributed Display Protocol) — WLED advanced mode, pixel streaming |
//!
//! ## Non-negotiable rules (SKILL.md / LUMYX_GOSL)
//! - Sequence numbers are **per-universe** (sACN/Art-Net) or **per-device** (DDP), wrapping — never a shared global counter.
//! - One universe per UDP datagram (sACN/Art-Net).
//! - DDP payload ≤ 1462 bytes per packet; auto-fragment for larger segments.
//! - Keep-alive fires at ≤ 800 ms regardless of sequencer state. A zeroed frame is not a heartbeat.
//! - Source conflict checked at startup; refuses to send on overlap, naming the other IP.
//! - WiFi is **unsupported** for live shows (cabled only).
//! - Zero allocations on the send path (pre-sized buffers).

pub mod artnet;
pub mod ddp;
pub mod device;
pub mod heartbeat;
pub mod packet;
pub mod pool;
pub mod router;
pub mod sender;

pub use artnet::{
    discover_controllers, presence, DiscoveryResult,
    build_art_dmx, find_conflicts, parse_art_dmx, ArtNetDevice, ArtPollReply, ConflictReport,
};
pub use ddp::{build_ddp_packet, build_ddp_packet_bytes, build_ddp_packet_format, max_pixels_per_packet, DDP_MAX_PAYLOAD_BYTES, parse_ddp_packet, DdpDevice, DdpPacket, DDP_MAX_PAYLOAD, DDP_MAX_PIXELS, DDP_PORT};
pub use device::{multicast_addr, SacnDevice};
pub use heartbeat::{health, HealthStatus, Heartbeat, HEARTBEAT_MS};
pub use pool::BufferPool;
pub use router::{DdpBackend, ProtocolBackend, RouteEntry, RouterDevice, SacnBackend};
pub use sender::{FrameSlice, ParallelSender, UniverseState};
