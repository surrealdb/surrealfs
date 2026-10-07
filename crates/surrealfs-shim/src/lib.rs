//! `surrealfs-shim`: Zero-privilege user-space POSIX libc interceptor & WASI filesystem
//!
//! Intercepts standard libc file calls (`open`, `read`, `write`, `stat`, `unlink`, etc.)
//! directing `/surrealfs/...` paths to SurrealFS over WebSocket/in-process memory.
//! Passes through all other filesystem calls to real libc via `dlsym(RTLD_NEXT)`.

pub mod interceptor;
pub mod virtual_fs;
pub mod wasi;

pub use virtual_fs::{VirtualFs, MIN_VIRTUAL_FD};
pub use wasi::{WasiDescriptorType, WasiMetadata, WasiVirtualFs};
