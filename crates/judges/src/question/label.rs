/// One possible outcome: its value, and an optional description the model sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    value: String,
    description: Option<String>,
}

impl Label {
    /// An outcome with only its value.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            description: None,
        }
    }

    /// An outcome with a description the model sees.
    pub fn described(value: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            description: Some(description.into()),
        }
    }

    /// The outcome's value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The description the model sees, if any.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

impl From<&str> for Label {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}
