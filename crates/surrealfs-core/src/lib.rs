//! surrealfs-core: High-performance asynchronous Rust client for SurrealFS.

pub mod archive;
pub mod backup;
pub mod chunking;
pub mod connector;
pub mod crdt;
pub mod crypto;
pub mod errors;
pub mod fs;
pub mod git;
pub mod models;
pub mod paths;
pub mod stats;
pub mod understanding;
pub mod webhooks;

pub use archive::*;
pub use backup::*;
pub use chunking::*;
pub use connector::*;
pub use crdt::*;
pub use crypto::*;
pub use errors::{Result, SurrealFsError};
pub use fs::{ConnectOptions, SurrealFs};
pub use git::*;
pub use models::*;
pub use paths::*;
pub use stats::*;
pub use understanding::*;
pub use webhooks::*;
