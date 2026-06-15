//! Global concurrency limits that protect the origin under burst load.
//!
//! Single-flight dedups requests for the *same* key, but a cold-start burst of
//! *distinct* packages can still overwhelm the process. Two semaphores cap the
//! expensive resources:
//!
//! - [`BUNDLE_SEMAPHORE`]: caps concurrent rolldown bundles. Each bundle is
//!   CPU-heavy and unpacks the package to a temp dir, so unbounded concurrency
//!   risks CPU starvation and OOM.
//! - [`DOWNLOAD_SEMAPHORE`]: caps concurrent outbound fetches (tarball +
//!   metadata) to upstream registries. npm rate-limits anonymous callers by IP
//!   (HTTP 429, then IP blocks); staying under the threshold avoids being
//!   blocked. See npm's rate-limit policy.

use std::sync::LazyLock;
use tokio::sync::Semaphore;

/// Max concurrent ESM bundles. Defaults to the CPU core count so bundles
/// saturate the machine without over-subscribing.
pub static BUNDLE_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    let permits = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    Semaphore::new(permits)
});

/// Max concurrent outbound fetches (tarball downloads + registry metadata).
/// npm returns 429 and eventually blocks IPs that burst; 50 is a conservative
/// ceiling. Override via `NEXUS_DOWNLOAD_CONCURRENCY`.
pub static DOWNLOAD_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    let permits = std::env::var("NEXUS_DOWNLOAD_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    Semaphore::new(permits)
});
