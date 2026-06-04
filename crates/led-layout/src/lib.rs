//! # led-layout — where pixels live in space
//!
//! Owns the logical pixel model ([`PixelLogical`]/[`Layout`]), the prop generators
//! ([`LayoutBuilder`]), and the [`LayoutMapper`] that compiles a layout into a
//! [`led_core::CompiledLayout`] for the HAL. Pure logical space; the one mapping is applied
//! once, downstream, at the HAL.

pub mod mapper;
pub mod model;
pub mod props;

pub use mapper::LayoutMapper;
pub use model::{GroupId, Layout, PixelLogical};
pub use props::LayoutBuilder;
