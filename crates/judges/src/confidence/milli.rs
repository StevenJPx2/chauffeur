/// A probability in thousandths, from 0 to 1000, so comparisons stay exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Milli(pub(super) u16);

impl Milli {
    /// Certainty.
    pub const ONE: Self = Self(1000);

    /// `thousandths` as a probability, if it is at most 1000.
    #[must_use]
    pub const fn new(thousandths: u16) -> Option<Self> {
        if thousandths <= 1000 {
            Some(Self(thousandths))
        } else {
            None
        }
    }

    /// `p`, from 0 to 1, rounded to the nearest thousandth.
    #[must_use]
    pub fn from_f64(p: f64) -> Option<Self> {
        if !(0.0..=1.0).contains(&p) {
            return None;
        }
        let rounded = (p * 1000.0).round();
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "`rounded` is a whole number from 0 to 1000, checked above"
        )]
        let thousandths = rounded as u16;
        Self::new(thousandths)
    }

    /// The probability in thousandths.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The probability, from 0 to 1.
    #[must_use]
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 1000.0
    }
}
