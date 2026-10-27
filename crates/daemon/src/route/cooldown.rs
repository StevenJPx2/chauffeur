use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How many rule and session pairs keep a cooldown.
const BOUND: usize = 4_096;

/// A rule's ID and the session it returned something in.
pub(super) type Key = (String, String);

/// When each rule last returned something in each session. Past its bound,
/// the oldest pair is forgotten.
pub(super) struct Cooldowns {
    fired: Mutex<BTreeMap<Key, Instant>>,
}

impl Cooldowns {
    pub(super) const fn new() -> Self {
        Self {
            fired: Mutex::new(BTreeMap::new()),
        }
    }

    /// Whether `key` returned something within `cooldown`.
    pub(super) fn cooling(&self, key: &Key, cooldown: Option<Duration>) -> bool {
        let (Some(cooldown), Ok(fired)) = (cooldown, self.fired.lock()) else {
            return false;
        };
        fired.get(key).is_some_and(|at| at.elapsed() < cooldown)
    }

    /// Records that `key` returned something now.
    pub(super) fn fire(&self, key: Key) {
        let Ok(mut fired) = self.fired.lock() else {
            return;
        };
        if fired.len() >= BOUND
            && let Some(oldest) = fired
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(key, _)| key.clone())
        {
            fired.remove(&oldest);
        }
        fired.insert(key, Instant::now());
    }
}
