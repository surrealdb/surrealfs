//! surrealfs-core: High-performance asynchronous Rust client for SurrealFS.

pub mod chunking;
pub mod crdt;
pub mod errors;
pub mod fs;
pub mod models;
pub mod paths;
pub mod understanding;

pub use chunking::*;
pub use crdt::*;
pub use errors::{Result, SurrealFsError};
pub use fs::{ConnectOptions, SurrealFs};
pub use models::*;
pub use paths::*;
pub use understanding::*;
