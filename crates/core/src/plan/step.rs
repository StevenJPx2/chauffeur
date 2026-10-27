/// One question, `request`, for the judge named `judge`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step<Q> {
    id: String,
    judge: String,
    request: Q,
}

impl<Q> Step<Q> {
    /// A step with its ID, the name of the judge that answers it, and its request.
    pub fn new(id: impl Into<String>, judge: impl Into<String>, request: Q) -> Self {
        Self {
            id: id.into(),
            judge: judge.into(),
            request,
        }
    }

    /// The step's ID, unique within its plan.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name of the judge that answers it.
    pub(super) fn judge(&self) -> &str {
        &self.judge
    }

    /// What the judge is asked.
    #[must_use]
    pub const fn request(&self) -> &Q {
        &self.request
    }
}
