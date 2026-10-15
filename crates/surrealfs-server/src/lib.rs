pub mod api;
pub mod auth;
pub mod chat;
pub mod embed;
pub mod gateway;
pub mod http;
pub mod mcp;
pub mod s3;
pub mod webdav;

pub mod nfs;

#[cfg(not(target_arch = "wasm32"))]
pub mod sftp;

#[cfg(target_arch = "wasm32")]
pub mod worker;

pub use auth::{AuthenticatedUser, Authenticator};
pub use chat::ChatConfig;
pub use embed::{Embedder, index_file_on_write, process_pending_jobs};
pub use gateway::SurrealFsGateway;
pub use http::{HttpHandler, HttpRequest, HttpResponse};
pub use nfs::{NfsFattr3, NfsFileHandle, NfsServer};
pub use s3::S3Server;
pub use webdav::WebDavServer;

#[cfg(not(target_arch = "wasm32"))]
pub use http::{run_http_server, serve_http};
#[cfg(not(target_arch = "wasm32"))]
pub use nfs::serve_nfs;
#[cfg(not(target_arch = "wasm32"))]
pub use sftp::SftpSession;
