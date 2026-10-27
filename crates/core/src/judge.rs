use crate::plan::Outcome;

/// Answers a step's request. Any `Fn(&Q) -> Outcome<O>` is a judge.
pub trait Judge<Q, O> {
    /// Answers one request.
    fn judge(&self, request: &Q) -> Outcome<O>;

    /// Answers several requests, one outcome each, in order. A judge that can
    /// batch, such as one model call per round, overrides this.
    fn judge_all(&self, requests: &[&Q]) -> Vec<Outcome<O>> {
        requests.iter().map(|request| self.judge(request)).collect()
    }
}

impl<Q, O, F> Judge<Q, O> for F
where
    F: Fn(&Q) -> Outcome<O>,
{
    fn judge(&self, request: &Q) -> Outcome<O> {
        self(request)
    }
}

/// The judges a plan may name, by name. A panel holds a handful of judges,
/// so it looks them up in order.
pub struct Panel<'a, Q, O> {
    judges: Vec<(String, &'a dyn Judge<Q, O>)>,
}

impl<'a, Q, O> Panel<'a, Q, O> {
    /// A panel with no judges.
    #[must_use]
    pub const fn new() -> Self {
        Self { judges: Vec::new() }
    }

    /// The panel with `judge` under `name`, replacing any judge of that name.
    #[must_use]
    pub fn with<J: Judge<Q, O>>(mut self, name: impl Into<String>, judge: &'a J) -> Self {
        let name = name.into();
        match self
            .judges
            .iter_mut()
            .find(|(existing, _)| *existing == name)
        {
            Some((_, slot)) => *slot = judge,
            None => self.judges.push((name, judge)),
        }
        self
    }

    /// The judge named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&'a dyn Judge<Q, O>> {
        self.judges
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, judge)| *judge)
    }
}

impl<Q, O> Default for Panel<'_, Q, O> {
    fn default() -> Self {
        Self::new()
    }
}
