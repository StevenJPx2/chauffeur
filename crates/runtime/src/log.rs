use std::collections::VecDeque;

/// The most recent entries, up to a bound; adding past it drops the oldest
/// and counts it.
#[derive(Clone, Debug)]
pub struct Log<T> {
    entries: VecDeque<T>,
    bound: usize,
    dropped: u64,
}

impl<T> Log<T> {
    /// An empty log keeping at most `bound` entries, and at least one.
    #[must_use]
    pub fn new(bound: usize) -> Self {
        let bound = bound.max(1);
        Self {
            entries: VecDeque::with_capacity(bound),
            bound,
            dropped: 0,
        }
    }

    /// Adds `entry`, dropping the oldest when full.
    pub fn push(&mut self, entry: T) {
        if self.entries.len() >= self.bound {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.entries.push_back(entry);
    }

    /// The entries, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter()
    }

    /// How many entries were dropped to stay within the bound.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}
