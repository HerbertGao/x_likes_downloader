// Existing modules — used by the legacy human-facing CLI (`xld download` etc.)
pub mod config;
pub mod downloader;
pub mod envelope;
pub mod error;
pub mod organize_files;
pub mod sandbox;
pub mod setup;
pub mod updater;
pub mod x_api;

/// Agent-facing API.
///
/// Pure-function library surface used by:
/// - the new CLI subcommands (`xld likes list`, `xld media download`, `xld auth status`)
/// - the future MCP server (v2)
///
/// Functions in this module never write to stdout/stderr by themselves. CLI
/// entry points wrap their results in `OutputEnvelope` and inject a
/// `ProgressSink` (`McpProgressSink` for v2 Agent, `IndicatifSink` for human CLI) when streaming.
pub mod agent {
    pub mod auth_status;
    pub mod download_media;
    pub mod etag_cache;
    pub mod import_curl;
    pub mod list_likes;
    pub mod mcp_server;
    pub mod types;

    pub use auth_status::auth_status;
    pub use download_media::download_media;
    pub use import_curl::import_curl;
    pub use list_likes::list_likes;
    pub use mcp_server::run_serve_mcp;
    pub use types::*;
}
