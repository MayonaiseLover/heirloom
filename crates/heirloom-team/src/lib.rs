//! # heirloom-team
//!
//! Self-hostable team-memory server. Lets an organization share a memory pool
//! across every team member's local Heirloom installation, over MCP.
//!
//! ## What ships in v1.0
//!
//! - Self-hostable single-binary Rust server (`heirloom-team-server`).
//! - SQLite-backed storage for memories and audit logs.
//! - Bearer-token authentication. Tokens are server-generated, scoped per
//!   member, revokable. Members never share tokens.
//! - All memory content is encrypted client-side with the team passphrase
//!   before upload. The server stores only ciphertext, sizes, timestamps,
//!   and metadata the user explicitly chose to expose.
//! - Comprehensive audit log: every read, write, and delete by actor, IP,
//!   and timestamp. Stream to JSON for SIEM ingestion.
//! - Built-in admin CLI subcommands for member management.
//!
//! ## What's deferred to v1.1
//!
//! - OIDC / SAML SSO. Bearer tokens are honest for v1.0; many small orgs
//!   prefer them anyway. Enterprise customers will need OIDC.
//! - Postgres backend. SQLite is fine up to several million memories.
//! - Per-source ACLs (e.g. "only Engineering sees code review memories").
//!
//! ## Deployment
//!
//! ```bash
//! # Run the server
//! heirloom-team-server --addr 0.0.0.0:7900 --data-dir /var/lib/heirloom-team
//!
//! # Create the first admin and team
//! heirloom-team-server admin init --team-name "Acme Engineering"
//! # → prints a bootstrap token; paste into the first member's CLI
//! ```
//!
//! See `docs/design/teams-architecture.md` for the full architecture,
//! including the planned v1.1 OIDC story and the v2.0 hosted-cloud option.

pub mod auth;
pub mod db;
pub mod server;

pub use server::serve;
