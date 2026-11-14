//! A navigation's events, kept in a log with a bound: what comes past it is
//! counted, not kept.

/// The most events one navigation keeps: every rule a source may hold, each
/// with its match, its round and its steps, fits many times over.
pub const MAX_EVENTS: usize = 4096;

/// Events in order, at most `max` of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventLog<T> {
    events: Vec<T>,
    max: usize,
    dropped: usize,
}

impl<T> EventLog<T> {
    /// An empty log keeping at most `max` events.
    #[must_use]
    pub const fn new(max: usize) -> Self {
        Self {
            events: Vec::new(),
            max,
            dropped: 0,
        }
    }

    /// Keeps `event`, unless the log is full. Whether it was kept.
    pub fn push(&mut self, event: T) -> bool {
        if self.events.len() >= self.max {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.events.push(event);
        true
    }

    /// The events kept, in order.
    #[must_use]
    pub fn events(&self) -> &[T] {
        &self.events
    }

    /// How many events were not kept.
    #[must_use]
    pub const fn dropped(&self) -> usize {
        self.dropped
    }
}
