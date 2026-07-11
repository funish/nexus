//! Single-flight deduplication.
//!
//! When N concurrent callers miss the cache for the same key, only the leader
//! runs `compute`; followers wait on a per-key broadcast for the leader to finish,
//! then re-read the cache themselves (the leader's stored result is shared via
//! the storage layer).
//!
//! Uses a `tokio::sync::broadcast` channel per pending key. The leader creates
//! the sender on entry; every follower clones a receiver *before* the leader can
//! finish, so no one can miss the completion signal (broadcast permits late
//! receivers to read a buffered value even if the sender already fired).

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::future::Future;
use std::sync::LazyLock;
use tokio::sync::broadcast;

/// Channel capacity 1 — only one "done" signal is ever sent. `recv()` resolves
/// even for followers that subscribe after the leader sends, because the message
/// is buffered until the last receiver drops.
type Done = broadcast::Sender<()>;

static PENDING: LazyLock<DashMap<String, Done>> = LazyLock::new(DashMap::new);

/// Run `compute` at most once concurrently for `key`. The leader executes it;
/// followers wait for the leader to finish, then return (callers re-read storage).
pub async fn run_once<F, Fut>(key: &str, compute: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    // First caller inserts the sender and becomes the leader. Followers find the
    // entry occupied and subscribe to the same channel.
    let (is_leader, mut done_rx) = match PENDING.entry(key.to_string()) {
        Entry::Vacant(v) => {
            let (tx, rx) = broadcast::channel(1);
            v.insert(tx);
            (true, rx)
        }
        Entry::Occupied(o) => (false, o.get().subscribe()),
    };

    if is_leader {
        compute().await;
        // Signal every subscriber. `send` fails only when there are no receivers,
        // which is fine (no one is waiting). The buffered message lets late
        // subscribers — those who grabbed the receiver after this point but before
        // the entry is removed — drain it instead of blocking forever.
        if let Some((_, tx)) = PENDING.remove(key) {
            let _ = tx.send(());
            // tx drops here, closing the channel; receivers that never got a
            // message (impossible here, since send succeeded above) would get
            // a RecvError::Closed.
        }
    } else {
        // Wait for the leader's signal. Errors only occur if the leader panicked
        // and dropped the sender without sending — treat that the same as "done"
        // and re-read storage (which will simply miss again).
        let _ = done_rx.recv().await;
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
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                })
                .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        // Exactly one leader runs compute; the rest wait and skip.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
