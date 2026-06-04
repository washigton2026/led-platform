//! # led-protocols — wire-protocol device drivers
//!
//! Drivers that plug in **under the HAL** ([`led_core::DeviceDriver`]). They live at the
//! final output stage, in physical space, and turn already-mapped `UniverseData` into
//! packets on the wire. This slice implements E1.31 (sACN); Art-Net / DDP slot in the same
//! way later.

pub mod artnet;
pub mod device;
pub mod packet;

pub use artnet::{find_conflicts, ArtPollReply, ConflictReport};
pub use device::{multicast_addr, SacnDevice};
