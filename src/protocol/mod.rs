//! Shared wire protocol and presentation encoding code.

#[cfg(test)]
mod contract_fixtures;
pub mod endpoint;
pub(crate) mod render_ansi;
pub(crate) mod surface_delta;
pub(crate) mod surface_reuse;
pub(crate) mod surface_scroll;
mod wire;

pub use wire::*;
