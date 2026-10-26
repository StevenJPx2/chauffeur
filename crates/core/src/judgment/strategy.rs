//! Common judging strategies, each a composition of [`Judgment`]'s primitives.
//! A capability composes its own when these do not fit.

use serde::Deserialize;

use crate::system_one::Question;

use super::rule::probability;
use super::{Judgment, Rule};

/// What one System One answer settled, read against a confident yes and a
/// confident no.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screened {
    Yes,
    No,
    /// Answered, but neither rule held.
    Unsure,
    /// The call failed.
    Failed,
}

/// Which screened outcomes go on to System Two.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Escalation {
    #[serde(default)]
    pub unsure: bool,
    #[serde(default)]
    pub failed: bool,
}

impl Escalation {
    #[must_use]
    pub const fn covers(self, screened: Screened) -> bool {
        match screened {
            Screened::Unsure => self.unsure,
            Screened::Failed => self.failed,
            Screened::Yes | Screened::No => false,
        }
    }
}

/// One question per candidate, judged by its own rule.
pub struct Candidate<K> {
    pub key: K,
    pub question: Question,
    pub rule: Rule,
}

/// A candidate judged in ordered steps: each is asked, a round later, only
/// when the previous step held.
pub struct Chained<K> {
    pub key: K,
    pub steps: Vec<(Question, Rule)>,
}

/// One question and one rule: whether it holds.
#[must_use]
pub fn single(question: Question, rule: Rule) -> Judgment<bool> {
    Judgment::ask(question, move |answer| rule.holds(answer))
}

/// One question read as a confident `yes`, a confident `no`, unsure, or a
/// failed call.
#[must_use]
pub fn screen(question: Question, yes: Rule, no: Rule) -> Judgment<Screened> {
    Judgment::ask(question, move |answer| match answer {
        None => Screened::Failed,
        Some(_) if yes.holds(answer) => Screened::Yes,
        Some(_) if no.holds(answer) => Screened::No,
        Some(_) => Screened::Unsure,
    })
}

/// System One first, System Two when it matters: the outcomes `when` covers
/// continue as `review` in the next round; every other outcome settles as
/// `settle` reads it.
#[must_use]
pub fn escalate<T: Send + 'static>(
    screen: Judgment<Screened>,
    when: Escalation,
    settle: impl FnOnce(Screened) -> T + Send + 'static,
    review: impl FnOnce(Screened) -> Judgment<T> + Send + 'static,
) -> Judgment<T> {
    screen.then(move |screened| {
        if when.covers(screened) {
            review(screened)
        } else {
            Judgment::done(settle(screened))
        }
    })
}

/// Every candidate asked in the same round; the keys whose rule holds, most
/// likely first.
#[must_use]
pub fn fan_out<K: Send + 'static>(
    candidates: impl IntoIterator<Item = Candidate<K>>,
) -> Judgment<Vec<K>> {
    let judges = candidates
        .into_iter()
        .map(
            |Candidate {
                 key,
                 question,
                 rule,
             }| {
                Judgment::ask(question, move |answer| {
                    rule.holds(answer)
                        .then(|| (key, answer.and_then(probability).unwrap_or_default()))
                })
            },
        )
        .collect();

    Judgment::all(judges).map(|held: Vec<Option<(K, f32)>>| {
        let mut held: Vec<(K, f32)> = held.into_iter().flatten().collect();

        held.sort_by(|a, b| b.1.total_cmp(&a.1));
        held.into_iter().map(|(key, _)| key).collect()
    })
}

/// [`fan_out`], also reading each answer against `other`: the keys whose own
/// rule held, most likely first, and the keys `other` held for, in candidate
/// order. One question per candidate serves both.
#[must_use]
pub fn fan_out_with<K: Clone + Send + 'static>(
    candidates: impl IntoIterator<Item = Candidate<K>>,
    other: Rule,
) -> Judgment<(Vec<K>, Vec<K>)> {
    let judges = candidates
        .into_iter()
        .map(
            |Candidate {
                 key,
                 question,
                 rule,
             }| {
                Judgment::ask(question, move |answer| {
                    let p = answer.and_then(probability).unwrap_or_default();

                    (key, p, rule.holds(answer), other.holds(answer))
                })
            },
        )
        .collect();

    Judgment::all(judges).map(|read: Vec<(K, f32, bool, bool)>| {
        let others = read
            .iter()
            .filter(|(_, _, _, other)| *other)
            .map(|(key, ..)| key.clone())
            .collect();
        let mut held: Vec<(K, f32)> = read
            .into_iter()
            .filter(|(_, _, own, _)| *own)
            .map(|(key, p, ..)| (key, p))
            .collect();

        held.sort_by(|a, b| b.1.total_cmp(&a.1));

        (held.into_iter().map(|(key, _)| key).collect(), others)
    })
}

/// Every candidate's steps in order, candidates side by side; the keys whose
/// every step held, in candidate order.
#[must_use]
pub fn chain<K: Send + 'static>(
    candidates: impl IntoIterator<Item = Chained<K>>,
) -> Judgment<Vec<K>> {
    let judges = candidates
        .into_iter()
        .map(|Chained { key, steps }| {
            steps_hold(steps.into_iter()).map(move |held| held.then_some(key))
        })
        .collect();

    Judgment::all(judges).map(|held: Vec<Option<K>>| held.into_iter().flatten().collect())
}

/// Whether every step holds, asking each only after the previous held.
fn steps_hold(
    mut steps: impl Iterator<Item = (Question, Rule)> + Send + 'static,
) -> Judgment<bool> {
    let Some((question, rule)) = steps.next() else {
        return Judgment::done(true);
    };

    single(question, rule).then(move |held| {
        if held {
            steps_hold(steps)
        } else {
            Judgment::done(false)
        }
    })
}
