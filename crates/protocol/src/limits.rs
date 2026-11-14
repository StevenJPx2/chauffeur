use std::fmt;

/// A bound a request or a strategy is checked against before it is admitted,
/// so no navigation can ask for unbounded work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Limit {
    /// Steps in one strategy.
    Steps,
    /// Outcomes in one question.
    Outcomes,
    /// Evidence blocks in one request.
    EvidenceBlocks,
    /// Candidates in one request.
    Candidates,
    /// Bytes of evidence and candidates, names included, in one request.
    EvidenceBytes,
    /// Milliseconds a request may give its rules.
    Deadline,
}

impl Limit {
    /// Every limit, in the order the docs list them.
    pub const ALL: [Self; 6] = [
        Self::Steps,
        Self::Outcomes,
        Self::EvidenceBlocks,
        Self::Candidates,
        Self::EvidenceBytes,
        Self::Deadline,
    ];

    /// The limit's wire name, such as `evidence_bytes`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Steps => "steps",
            Self::Outcomes => "outcomes",
            Self::EvidenceBlocks => "evidence_blocks",
            Self::Candidates => "candidates",
            Self::EvidenceBytes => "evidence_bytes",
            Self::Deadline => "deadline_ms",
        }
    }

    /// The most it allows.
    #[must_use]
    pub const fn max(self) -> u64 {
        match self {
            Self::Steps => 32,
            Self::Outcomes | Self::EvidenceBlocks | Self::Candidates => 64,
            Self::EvidenceBytes => 256 * 1024,
            Self::Deadline => 30_000,
        }
    }

    /// Checks `count` against the limit.
    ///
    /// # Errors
    ///
    /// [`Exceeded`] when `count` is over [`Limit::max`].
    pub const fn check(self, count: u64) -> Result<(), Exceeded> {
        if count > self.max() {
            return Err(Exceeded { limit: self, count });
        }
        Ok(())
    }
}

/// A limit that was exceeded, and by how much.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub struct Exceeded {
    /// The limit.
    pub limit: Limit,
    /// What was asked for.
    pub count: u64,
}

impl fmt::Display for Exceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (count, max) = (self.count, self.limit.max());
        match self.limit {
            Limit::Steps => write!(f, "the strategy has {count} steps, over {max}"),
            Limit::Outcomes => write!(f, "a question has {count} outcomes, over {max}"),
            Limit::EvidenceBlocks => write!(f, "{count} evidence blocks, over {max}"),
            Limit::Candidates => write!(f, "{count} candidates, over {max}"),
            Limit::EvidenceBytes => write!(f, "evidence is over {} KiB", max / 1024),
            Limit::Deadline => write!(f, "deadline_ms {count} is over {max}"),
        }
    }
}

/// A length as a count to check; one too long for `u64` is over any limit.
#[must_use]
pub fn count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
