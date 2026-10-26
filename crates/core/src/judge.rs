//! Judges answer steps. A judge may be a fixed rule, a model such as a System
//! One classifier or System Two reasoner, or a person; core treats them alike.

use crate::plan::Outcome;

/// Anything that answers a step's request with an outcome.
///
/// Plain functions and closures `Fn(&Q) -> Outcome<O>` are judges.
pub trait Judge<Q, O> {
    /// Answer one request. A judge that cannot answer returns
    /// [`Outcome::Failed`].
    fn judge(&self, request: &Q) -> Outcome<O>;

    /// Answer this judge's share of one round, one outcome per request in
    /// order. A judge that can batch, such as a remote classifier, overrides
    /// this to answer the round in one call.
    fn judge_all(&self, requests: &[&Q]) -> Vec<Outcome<O>> {
        requests.iter().map(|request| self.judge(request)).collect()
    }
}

impl<Q, O, F: Fn(&Q) -> Outcome<O>> Judge<Q, O> for F {
    fn judge(&self, request: &Q) -> Outcome<O> {
        self(request)
    }
}

/// The judges a plan may name, by name.
pub struct Panel<'a, Q, O> {
    judges: Vec<(String, &'a dyn Judge<Q, O>)>,
}

impl<Q, O> Default for Panel<'_, Q, O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, Q, O> Panel<'a, Q, O> {
    /// A panel with no judges.
    #[must_use]
    pub const fn new() -> Self {
        Self { judges: Vec::new() }
    }

    /// Seat `judge` under `name`, replacing any judge already of that name.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, judge: &'a dyn Judge<Q, O>) -> Self {
        let name = name.into();
        self.judges.retain(|(seated, _)| *seated != name);
        self.judges.push((name, judge));
        self
    }

    /// The judge seated under `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&'a dyn Judge<Q, O>> {
        self.judges
            .iter()
            .find(|(seated, _)| seated == name)
            .map(|&(_, judge)| judge)
    }
}
