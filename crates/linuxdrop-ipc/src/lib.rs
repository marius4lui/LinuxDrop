//! Shared Manager1 wire contract. Regenerate bindings with tools/sync-ipc-contract.py.
mod generated;
pub use generated::*;
pub const MANAGER_XML: &str = include_str!("../manager1.xml");
#[cfg(feature = "client")]
mod proxy;
#[cfg(feature = "client")]
pub use proxy::*;

mod settings;
pub use settings::*;

mod snapshot;
pub use snapshot::*;

#[cfg(feature = "schema")]
mod schema;
#[cfg(feature = "schema")]
pub use schema::snapshot_schema;

mod hardware;
pub use hardware::*;

mod reports;
pub use reports::*;
