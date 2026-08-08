//! Cross-domain shared infrastructure (route-agnostic), reused by both the CDN
//! and winget domains.
//!
//! - [`http`]: the single shared reqwest client (one connection pool).
//! - [`cache`]: TTL cache helpers built on the storage layer.
//! - [`singleflight`]: per-key concurrent-request deduplication.
//! - [`concurrency`]: global semaphores capping bundles and upstream downloads.
pub mod cache;
pub mod concurrency;
pub mod http;
pub mod singleflight;
