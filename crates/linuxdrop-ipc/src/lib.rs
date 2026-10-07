//! Shared Manager1 wire contract. Regenerate bindings with tools/sync-ipc-contract.py.
mod generated;
pub use generated::*;
pub const MANAGER_XML: &str = include_str!("../manager1.xml");
#[cfg(feature = "client")]
mod proxy;
#[cfg(feature = "client")]
pub use proxy::*;
