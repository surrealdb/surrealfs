//! surrealfs-core: High-performance asynchronous Rust client for SurrealFS.

pub mod errors;
pub mod fs;
pub mod models;
pub mod paths;

pub use errors::{Result, SurrealFsError};
pub use fs::{ConnectOptions, SurrealFs};
pub use models::*;
pub use paths::*;
