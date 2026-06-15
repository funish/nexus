//! Single-flight deduplication.
//!
//! When N concurrent callers miss the cache for the same key, only the leader
//! runs `compute`; followers poll until the leader finishes, then re-read the
//! cache themselves (the leader's stored result is shared via the storage layer).
//!
//! Uses simple key-presence polling rather than `Notify`, avoiding the race where
//! a follower registers interest after the leader has already notified.

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

static PENDING: LazyLock<DashMap<String, ()>> = LazyLock::new(DashMap::new);

/// Run `compute` at most once concurrently for `key`. The leader executes it;
/// followers wait for the key to disappear, then return (callers re-read storage).
pub async fn run_once<F, Fut>(key: &str, compute: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let is_leader = match PENDING.entry(key.to_string()) {
        Entry::Vacant(v) => {
            v.insert(());
            true
        }
        Entry::Occupied(_) => false,
    };

    if is_leader {
        compute().await;
        PENDING.remove(key);
    } else {
        while PENDING.contains_key(key) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn run_once_dedupes_concurrent_compute() {
        let count = Arc::new(AtomicUsize::new(0));
        let key = "singleflight-test-dedup";
        let barrier = Arc::new(tokio::sync::Barrier::new(10));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let count = count.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                run_once(key, || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // Exactly one leader runs compute; the rest poll and skip.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
