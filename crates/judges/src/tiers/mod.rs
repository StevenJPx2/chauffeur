mod fallback;
mod judge;
pub mod system_one;

pub use fallback::{Fallback, Model};
pub use judge::NamedJudge;
use serde::Deserialize;

/// The four tiers a step's judge can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// JavaScript facts, with no model.
    Facts,
    /// A fast classifier.
    SystemOne,
    /// A reasoning model.
    SystemTwo,
    /// The user.
    Person,
}

impl Tier {
    /// The tier named `name`, such as `system-one`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Facts, Self::SystemOne, Self::SystemTwo, Self::Person]
            .into_iter()
            .find(|tier| tier.name() == name)
    }

    /// The tier's name in rule files and `models.json`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Facts => "facts",
            Self::SystemOne => "system-one",
            Self::SystemTwo => "system-two",
            Self::Person => "person",
        }
    }
}
