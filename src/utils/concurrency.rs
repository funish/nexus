//! Global resource limits for bursty cold-start work.
//!
//! Single-flight dedups requests for the *same* key, but distinct packages can
//! still arrive concurrently. Bundles are CPU/memory heavy. Outbound downloads
//! materialize bodies in memory, so their cap is a local memory guardrail rather
//! than upstream politeness — npm, JSR, cdnjs, raw, and codeload are CDN-style
//! read paths without published numeric budgets. Only `api.github.com` has a
//! hard primary quota (60 requests/hour unauthenticated, 5,000 with a token),
//! and its callers are separately grouped in [`GITHUB_API_SEMAPHORE`].

use std::sync::LazyLock;
use tokio::sync::Semaphore;

use crate::cdn::constants::CDN_MAX_PACKAGE_SIZE;
use crate::utils::machine::{cpu_count, total_memory_bytes};

/// Max concurrent ESM bundles. Defaults to the CPU core count so bundles
/// saturate the machine, with a memory-derived secondary limit on small hosts.
pub static BUNDLE_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    Semaphore::new(env_permits("NEXUS_BUNDLE_CONCURRENCY").unwrap_or_else(bundle_default))
});

/// Max concurrent materialized outbound responses in the permissive group (npm,
/// JSR, cdnjs, WordPress SVN, raw, codeload, the winget CDN). These endpoints
/// have no published numeric limit; npm alone fronts reads with a CDN that
/// serves billions of weekly downloads. The cap exists because each slot can
/// hold up to one max-sized response while waiting for storage work. The
/// memory-derived default is a guardrail, not an upstream rate limit. Override
/// via `NEXUS_DOWNLOAD_CONCURRENCY`.
pub static DOWNLOAD_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    Semaphore::new(env_permits("NEXUS_DOWNLOAD_CONCURRENCY").unwrap_or_else(download_default))
});

/// Max concurrent outbound fetches to api.github.com — the only GitHub endpoint
/// with a hard published limit (60 req/h per IP unauthenticated, 5,000/h with a
/// token, at most 100 concurrent requests). Only directory/metadata calls use
/// it (repo tags, winget tree discovery); file and tarball downloads go through
/// raw.githubusercontent.com and codeload.github.com, which are CDN-style
/// distribution endpoints with unpublished per-IP limits and belong to
/// [`DOWNLOAD_SEMAPHORE`]. Request *volume* against the API is already
/// minimized by tree caching, tags TTL, and single-flight — this bounds
/// concurrency as a second line of defense.
pub static GITHUB_API_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    Semaphore::new(env_permits("NEXUS_GITHUB_API_CONCURRENCY").unwrap_or_else(github_api_default))
});

/// Max concurrent local storage writes when caching a package's files. Local
/// filesystem and S3 benefit from a wider queue than one file at a time.
pub static STORAGE_WRITE_CONCURRENCY: LazyLock<usize> = LazyLock::new(|| {
    env_permits("NEXUS_STORAGE_WRITE_CONCURRENCY").unwrap_or_else(|| cpu_count().saturating_mul(2))
});

/// Max concurrent WinGet version-manifest builds in `packageManifests`. Parsing
/// is CPU-bound while its raw fetches stay bounded by the download semaphore.
pub static WINGET_MANIFEST_BUILD_CONCURRENCY: LazyLock<usize> =
    LazyLock::new(|| env_permits("NEXUS_MANIFEST_BUILD_CONCURRENCY").unwrap_or_else(cpu_count));

pub fn download_concurrency() -> usize {
    env_permits("NEXUS_DOWNLOAD_CONCURRENCY").unwrap_or_else(download_default)
}

fn bundle_default() -> usize {
    derive_bundle_concurrency(total_memory_bytes(), cpu_count())
}

fn download_default() -> usize {
    derive_download_concurrency(total_memory_bytes(), cpu_count())
}

fn github_api_default() -> usize {
    cpu_count().min(100)
}

// At worst every download slot may hold one max-sized body. Reserve only a
// quarter of effective memory for that extreme, avoiding both a tiny fixed pool
// on large hosts and an oversized pool on small containers.
fn derive_download_concurrency(memory: Option<u64>, cores: usize) -> usize {
    match memory {
        Some(memory) => usize::try_from(memory / 4 / CDN_MAX_PACKAGE_SIZE)
            .unwrap_or(usize::MAX)
            .max(1),
        None => cores.saturating_mul(32),
    }
}

// Bundles also keep derived file data alive. Use an eighth of memory as a
// conservative secondary limit; CPU remains the primary limit on large hosts.
fn derive_bundle_concurrency(memory: Option<u64>, cores: usize) -> usize {
    match memory {
        Some(memory) => {
            let by_memory = usize::try_from(memory / 8 / CDN_MAX_PACKAGE_SIZE)
                .unwrap_or(usize::MAX)
                .max(1);
            cores.min(by_memory)
        }
        None => cores,
    }
}

fn env_permits(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|permits| *permits > 0)
}
