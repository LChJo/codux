//! Exactly-once input within the advertised retry window. Retain failed writes:
//! write_all/flush errors may have already delivered a prefix to the PTY.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const RETENTION: Duration = Duration::from_secs(60);
const MAX_INPUTS: usize = 8192;

type InputKey = (String, String, String);
#[derive(Default)]
pub(super) struct TerminalInputLedger {
    state: Mutex<InputLedgerState>,
}

#[derive(Default)]
struct InputLedgerState {
    entries: HashMap<InputKey, Arc<Mutex<InputResult>>>,
    reject_new_until: Option<Instant>,
}

#[derive(Default)]
struct InputResult {
    completed: Option<(Instant, Value)>,
}

impl TerminalInputLedger {
    pub(super) fn execute(
        &self,
        device: &str,
        session: &str,
        id: &str,
        write: impl FnOnce() -> Value,
    ) -> Value {
        let key = (device.to_owned(), session.to_owned(), id.to_owned());
        let entry = {
            let Ok(mut state) = self.state.lock() else {
                return rejected("Input result cache unavailable.");
            };
            if let Some(entry) = state.entries.get(&key) {
                Arc::clone(entry)
            } else {
                // Capacity rejections also must remain final through the retry
                // window. A bounded admission cooldown replaces unbounded
                // rejection tombstones; each rejected unknown id extends it.
                if state
                    .reject_new_until
                    .is_some_and(|until| Instant::now() < until)
                {
                    state.reject_new_until = Some(Instant::now() + RETENTION);
                    return rejected("Input retry cache is full; input was not written.");
                }
                // Never evict a live result just to admit new input. Expiring
                // only completed entries also protects slow in-flight writes.
                if state.entries.len() >= MAX_INPUTS {
                    state.entries.retain(|_, entry| {
                        entry
                            .try_lock()
                            .map(|result| {
                                result
                                    .completed
                                    .as_ref()
                                    .is_none_or(|(at, _)| at.elapsed() < RETENTION)
                            })
                            .unwrap_or(true)
                    });
                }
                if state.entries.len() >= MAX_INPUTS {
                    state.reject_new_until = Some(Instant::now() + RETENTION);
                    return rejected("Input retry cache is full; input was not written.");
                }
                let entry = Arc::new(Mutex::new(InputResult::default()));
                state.entries.insert(key, Arc::clone(&entry));
                entry
            }
        };
        let Ok(mut result) = entry.lock() else {
            // A panic during write leaves its delivery unknown. Never retry it.
            return uncertain("Input processing interrupted; delivery is unknown.");
        };
        if let Some((_, value)) = &result.completed {
            return value.clone();
        }
        let value = write();
        result.completed = Some((Instant::now(), value.clone()));
        value
    }
}

pub(super) fn rejected(error: &str) -> Value {
    json!({"ok": false, "accepted": false, "status": "rejected", "error": error})
}

pub(super) fn uncertain(error: &str) -> Value {
    json!({"ok": false, "accepted": false, "status": "uncertain", "error": error})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn concurrent_terminal_input_duplicates_execute_once() {
        let ledger = Arc::new(TerminalInputLedger::default());
        let barrier = Arc::new(Barrier::new(8));
        let writes = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let ledger = Arc::clone(&ledger);
                let barrier = Arc::clone(&barrier);
                let writes = Arc::clone(&writes);
                std::thread::spawn(move || {
                    barrier.wait();
                    ledger.execute("device", "session", "id", || {
                        writes.fetch_add(1, Ordering::SeqCst);
                        json!({"ok": true})
                    })
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), json!({"ok": true}));
        }
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn terminal_input_partial_write_failure_is_cached_and_scoped() {
        let ledger = TerminalInputLedger::default();
        let result = ledger.execute("device", "session", "id", || uncertain("partial write"));
        assert_eq!(result["status"], "uncertain");
        assert_eq!(
            ledger.execute("device", "session", "id", || panic!("must not replay")),
            result
        );
        assert_eq!(
            ledger.execute("other", "session", "id", || json!({"ok": true}))["ok"],
            true
        );
        assert_eq!(
            ledger.execute("device", "other", "id", || json!({"ok": true}))["ok"],
            true
        );
    }

    #[test]
    fn terminal_input_full_cache_does_not_evict_live_results() {
        let ledger = TerminalInputLedger::default();
        for index in 0..MAX_INPUTS {
            ledger.execute(
                "device",
                "session",
                &index.to_string(),
                || json!({"ok": true}),
            );
        }
        let result = ledger.execute("device", "session", "new", || panic!("cache full"));
        assert_eq!(result["status"], "rejected");
        assert_eq!(
            ledger.execute("device", "session", "0", || panic!("must retain"))["ok"],
            true
        );
        let mut state = ledger.state.lock().unwrap();
        for entry in state.entries.values() {
            entry.lock().unwrap().completed.as_mut().unwrap().0 = Instant::now() - RETENTION;
        }
        drop(state);
        // A retry of the rejected id must stay rejected even though space is
        // now reclaimable. Otherwise delayed retries could type after failure.
        assert_eq!(
            ledger.execute("device", "session", "new", || panic!("rejected retry")),
            result
        );
        state = ledger.state.lock().unwrap();
        state.reject_new_until = Some(Instant::now() - RETENTION);
        drop(state);
        assert_eq!(
            ledger.execute("device", "session", "new", || json!({"ok": true}))["ok"],
            true
        );
    }
}
