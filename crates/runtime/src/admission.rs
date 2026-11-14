//! Admission: one permit per navigation, from admission through delivery,
//! within a share for each source and a global capacity. A navigation that
//! finds no room is refused at once; nothing waits in a queue.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

/// Where a navigation comes from. Each source has its own share, so a flood
/// from one never starves another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A harness's adapter, through `chauffeur.emit`.
    Harness,
    /// A caller on the socket, through `chauffeur.navigate`.
    Integration,
}

impl Source {
    const COUNT: usize = 2;

    const fn index(self) -> usize {
        match self {
            Self::Harness => 0,
            Self::Integration => 1,
        }
    }

    /// The source's name, such as `integration`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Integration => "integration",
        }
    }
}

/// How many navigations may run at once: in all, and from each source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    /// Across every source.
    pub global: usize,
    /// From any one source.
    pub per_source: usize,
}

impl Capacity {
    /// The daemon's: 16 in all, 8 from each source.
    pub const DAEMON: Self = Self {
        global: 16,
        per_source: 8,
    };
}

/// Why a navigation was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub struct Busy {
    /// The source that asked.
    pub from: Source,
    /// Whether the source's share, rather than the global capacity, was full.
    pub share: bool,
}

impl fmt::Display for Busy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let which = if self.share {
            format!("the {} share", self.from.name())
        } else {
            "every share".to_owned()
        };
        write!(f, "{which} of capacity is in use")
    }
}

/// The permits in use, by source. Every change keeps each count within its
/// capacity and the total equal to their sum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ledger {
    capacity: Capacity,
    total: usize,
    by_source: [usize; Source::COUNT],
}

impl Ledger {
    /// An empty ledger.
    #[must_use]
    pub const fn new(capacity: Capacity) -> Self {
        Self {
            capacity,
            total: 0,
            by_source: [0; Source::COUNT],
        }
    }

    /// Takes a permit for `source` if its share and the capacity have room.
    ///
    /// # Errors
    ///
    /// [`Busy`], naming whether the share or the capacity is full.
    pub fn take(&mut self, source: Source) -> Result<(), Busy> {
        let Some(count) = self.by_source.get_mut(source.index()) else {
            return Err(Busy {
                from: source,
                share: true,
            });
        };
        if *count >= self.capacity.per_source {
            return Err(Busy {
                from: source,
                share: true,
            });
        }
        if self.total >= self.capacity.global {
            return Err(Busy {
                from: source,
                share: false,
            });
        }
        *count = count.saturating_add(1);
        self.total = self.total.saturating_add(1);
        Ok(())
    }

    /// Gives back a permit `source` took. One never taken changes nothing.
    pub fn give(&mut self, source: Source) {
        if let Some(count) = self.by_source.get_mut(source.index())
            && *count > 0
        {
            *count = count.saturating_sub(1);
            self.total = self.total.saturating_sub(1);
        }
    }

    /// The permits `source` holds.
    #[must_use]
    pub fn held(&self, source: Source) -> usize {
        self.by_source.get(source.index()).copied().unwrap_or(0)
    }

    /// The permits held in all.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.total
    }
}

/// Admits navigations against one ledger, shared by every connection.
#[derive(Clone, Debug)]
pub struct Admission {
    ledger: Arc<Mutex<Ledger>>,
}

impl Admission {
    /// Admission with `capacity`.
    #[must_use]
    pub fn new(capacity: Capacity) -> Self {
        Self {
            ledger: Arc::new(Mutex::new(Ledger::new(capacity))),
        }
    }

    /// A permit for one navigation from `source`, released when it drops.
    ///
    /// # Errors
    ///
    /// [`Busy`] while the source's share or the capacity is in use.
    pub fn admit(&self, source: Source) -> Result<Permit, Busy> {
        self.with(|ledger| ledger.take(source))?;
        Ok(Permit {
            ledger: Arc::clone(&self.ledger),
            source,
        })
    }

    /// The permits in use now.
    #[must_use]
    pub fn in_use(&self) -> Ledger {
        self.with(|ledger| *ledger)
    }

    // A ledger's changes cannot be left halfway, so one a panicking thread
    // held is still whole.
    fn with<T>(&self, change: impl FnOnce(&mut Ledger) -> T) -> T {
        change(&mut self.ledger.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// One admitted navigation's place. Dropping it, on any path, gives the
/// place back once.
#[derive(Debug)]
pub struct Permit {
    ledger: Arc<Mutex<Ledger>>,
    source: Source,
}

impl Permit {
    /// The source it was admitted for.
    #[must_use]
    pub const fn source(&self) -> Source {
        self.source
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.ledger
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .give(self.source);
    }
}
