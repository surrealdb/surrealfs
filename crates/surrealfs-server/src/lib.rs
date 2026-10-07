//! surrealfs-server: Multi-protocol server daemon for SurrealFS (§22).
//!
//! Provides native endpoints for WebDAV (§22.1), S3-compatible API (§22.3),
//! NFSv3 loopback (§22.2), and SFTP (§22.8).

pub mod auth;
pub mod http;
pub mod nfs;
pub mod s3;
pub mod sftp;
pub mod webdav;

pub use auth::{AuthenticatedUser, Authenticator};
pub use http::{run_http_server, HttpHandler, HttpRequest, HttpResponse};
pub use nfs::{NfsFileHandle, NfsServer};
pub use s3::S3Server;
pub use sftp::SftpSession;
pub use webdav::WebDavServer;
