//! Transport abstraction over the ring's BLE link.
//!
//! The protocol is request/response with asynchronous notifications. [`Transport`]
//! captures just what the client needs — write a request, and subscribe to the
//! stream of inbound frames — so the higher layers can be exercised with a mock
//! in tests while [`crate::ble`] provides the real `btleplug` implementation.

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::error::{Error, Result};

/// A bidirectional link to a ring.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Write a raw request frame to the ring's write characteristic.
    async fn write(&self, data: &[u8]) -> Result<()>;

    /// Subscribe to inbound notification frames (raw bytes, one per notification).
    fn subscribe(&self) -> broadcast::Receiver<Vec<u8>>;
}

/// Write `request` and collect notification frames until the link is quiet for
/// `quiet` (i.e. no new frame arrives within that window). This matches the
/// ring's behaviour of emitting one or more notifications per request with no
/// explicit terminator on most commands.
pub async fn transact<T>(transport: &T, request: &[u8], quiet: Duration) -> Result<Vec<Vec<u8>>>
where
    T: Transport + ?Sized,
{
    transact_until(transport, request, quiet, |_| false).await
}

/// Like [`transact`], but returns as soon as a frame satisfying `is_terminal`
/// arrives, instead of always waiting out the quiet window after the last frame.
///
/// Most ring commands have a well-known response (the official app proceeds the
/// moment it sees it), so terminating on it saves the full `quiet` window per
/// request — which dominates the handshake/setup phase otherwise. The quiet
/// window remains as the fallback for unexpected responses or a dead link.
///
/// A lagged inbound channel is an error, not a skip: history batches are
/// cursor-checkpointed, so a caller that sees the error re-pulls the batch. A
/// silent `continue` would drop events without any trace.
pub async fn transact_until<T, F>(
    transport: &T,
    request: &[u8],
    quiet: Duration,
    mut is_terminal: F,
) -> Result<Vec<Vec<u8>>>
where
    T: Transport + ?Sized,
    F: FnMut(&[u8]) -> bool,
{
    let mut rx = transport.subscribe();
    // Drop any backlog so we only observe responses to *this* request.
    while rx.try_recv().is_ok() {}

    transport.write(request).await?;

    let mut frames = Vec::new();
    loop {
        match tokio::time::timeout(quiet, rx.recv()).await {
            Ok(Ok(frame)) => {
                let done = is_terminal(&frame);
                frames.push(frame);
                if done {
                    break;
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                return Err(Error::Protocol(format!(
                    "inbound channel lagged by {n} frame(s) — the link outran the consumer; \
                     retry the request (history batches resume from the last checkpoint)"
                )));
            }
            // Channel closed or quiet window elapsed: we're done collecting.
            _ => break,
        }
    }
    Ok(frames)
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    //! A scripted transport for unit tests: maps request bytes to canned
    //! response frames. Three lookups, checked in this order on every write:
    //!
    //! 1. [`MockTransport::on_sequence`] — per-call queues keyed by the exact
    //!    request hex (the same authenticate bytes can answer `0x02` first and
    //!    `0x00` second). The last entry repeats once the queue is exhausted.
    //! 2. [`MockTransport::on`] — one fixed response set keyed by exact hex.
    //! 3. [`MockTransport::on_prefix`] — longest matching hex prefix, for
    //!    requests that embed the wall clock (`12 09 …` time syncs).
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    pub struct MockTransport {
        tx: broadcast::Sender<Vec<u8>>,
        responses: Mutex<HashMap<String, Vec<Vec<u8>>>>,
        sequences: Mutex<HashMap<String, VecDeque<Vec<Vec<u8>>>>>,
        prefixes: Mutex<Vec<(String, Vec<Vec<u8>>)>>,
        writes: Mutex<Vec<Vec<u8>>>,
    }

    impl Default for MockTransport {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self::with_capacity(64)
        }

        /// A mock whose inbound channel holds at most `capacity` frames — lets a
        /// test provoke a lagged receiver.
        pub fn with_capacity(capacity: usize) -> Self {
            let (tx, _) = broadcast::channel(capacity.max(1));
            Self {
                tx,
                responses: Mutex::new(HashMap::new()),
                sequences: Mutex::new(HashMap::new()),
                prefixes: Mutex::new(Vec::new()),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn decode_all(responses: &[&str]) -> Vec<Vec<u8>> {
            responses.iter().map(|h| hex::decode(h).unwrap()).collect()
        }

        /// Register canned responses keyed by the request's full hex.
        pub fn on(&self, request_hex: &str, responses: &[&str]) {
            self.responses
                .lock()
                .unwrap()
                .insert(request_hex.to_string(), Self::decode_all(responses));
        }

        /// Register one response set per call for a request that is sent more
        /// than once. The final set repeats once the queue runs dry.
        pub fn on_sequence(&self, request_hex: &str, per_call: &[&[&str]]) {
            let queue: VecDeque<Vec<Vec<u8>>> =
                per_call.iter().map(|r| Self::decode_all(r)).collect();
            self.sequences
                .lock()
                .unwrap()
                .insert(request_hex.to_string(), queue);
        }

        /// Register canned responses for any request whose hex starts with
        /// `prefix_hex`. The longest registered prefix wins.
        pub fn on_prefix(&self, prefix_hex: &str, responses: &[&str]) {
            let mut prefixes = self.prefixes.lock().unwrap();
            prefixes.retain(|(p, _)| p != prefix_hex);
            prefixes.push((prefix_hex.to_string(), Self::decode_all(responses)));
            prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        }

        pub fn writes(&self) -> Vec<Vec<u8>> {
            self.writes.lock().unwrap().clone()
        }

        fn lookup(&self, key: &str) -> Option<Vec<Vec<u8>>> {
            if let Some(queue) = self.sequences.lock().unwrap().get_mut(key) {
                if queue.len() > 1 {
                    return queue.pop_front();
                }
                if let Some(last) = queue.front() {
                    return Some(last.clone());
                }
            }
            if let Some(frames) = self.responses.lock().unwrap().get(key) {
                return Some(frames.clone());
            }
            self.prefixes
                .lock()
                .unwrap()
                .iter()
                .find(|(p, _)| key.starts_with(p.as_str()))
                .map(|(_, frames)| frames.clone())
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn write(&self, data: &[u8]) -> Result<()> {
            self.writes.lock().unwrap().push(data.to_vec());
            let key = hex::encode(data);
            if let Some(frames) = self.lookup(&key) {
                for f in frames {
                    let _ = self.tx.send(f);
                }
            }
            Ok(())
        }

        fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
            self.tx.subscribe()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockTransport;
    use super::*;

    #[tokio::test]
    async fn lagged_channel_is_an_error_not_a_skip() {
        let mock = MockTransport::with_capacity(1);
        mock.on("0c00", &["0d0100", "0d0101", "0d0102"]);
        let err = transact(&mock, &[0x0c, 0x00], Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("lagged"), "{err}");
    }

    #[tokio::test]
    async fn sequence_answers_per_call_then_repeats_last() {
        let mock = MockTransport::new();
        mock.on_sequence("0c00", &[&["0d0101"], &["0d0102"]]);
        let q = Duration::from_millis(20);
        let a = transact(&mock, &[0x0c, 0x00], q).await.unwrap();
        let b = transact(&mock, &[0x0c, 0x00], q).await.unwrap();
        let c = transact(&mock, &[0x0c, 0x00], q).await.unwrap();
        assert_eq!(a, vec![vec![0x0d, 0x01, 0x01]]);
        assert_eq!(b, vec![vec![0x0d, 0x01, 0x02]]);
        assert_eq!(c, vec![vec![0x0d, 0x01, 0x02]]);
    }

    #[tokio::test]
    async fn prefix_matches_longest_registered() {
        let mock = MockTransport::new();
        mock.on_prefix("12", &["0d0100"]);
        mock.on_prefix("1209", &["0d0109"]);
        let frames = transact(&mock, &[0x12, 0x09, 0xaa], Duration::from_millis(20))
            .await
            .unwrap();
        assert_eq!(frames, vec![vec![0x0d, 0x01, 0x09]]);
    }
}
