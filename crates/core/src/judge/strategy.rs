//! Common judging strategies, each a composition of [`Judge`]'s primitives.
//! A capability composes its own when these do not fit.

use crate::system_one::Question;

use super::rule::probability;
use super::{Judge, Rule};

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
pub fn single(question: Question, rule: Rule) -> Judge<bool> {
    Judge::ask(question, move |answer| rule.holds(answer))
}

/// Every candidate asked in the same round; the keys whose rule holds, most
/// likely first.
#[must_use]
pub fn fan_out<K: Send + 'static>(
    candidates: impl IntoIterator<Item = Candidate<K>>,
) -> Judge<Vec<K>> {
    let judges = candidates
        .into_iter()
        .map(
            |Candidate {
                 key,
                 question,
                 rule,
             }| {
                Judge::ask(question, move |answer| {
                    rule.holds(answer)
                        .then(|| (key, answer.and_then(probability).unwrap_or_default()))
                })
            },
        )
        .collect();

    Judge::all(judges).map(|held: Vec<Option<(K, f32)>>| {
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
) -> Judge<(Vec<K>, Vec<K>)> {
    let judges = candidates
        .into_iter()
        .map(
            |Candidate {
                 key,
                 question,
                 rule,
             }| {
                Judge::ask(question, move |answer| {
                    let p = answer.and_then(probability).unwrap_or_default();

                    (key, p, rule.holds(answer), other.holds(answer))
                })
            },
        )
        .collect();

    Judge::all(judges).map(|read: Vec<(K, f32, bool, bool)>| {
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
pub fn chain<K: Send + 'static>(candidates: impl IntoIterator<Item = Chained<K>>) -> Judge<Vec<K>> {
    let judges = candidates
        .into_iter()
        .map(|Chained { key, steps }| {
            steps_hold(steps.into_iter()).map(move |held| held.then_some(key))
        })
        .collect();

    Judge::all(judges).map(|held: Vec<Option<K>>| held.into_iter().flatten().collect())
}

/// Whether every step holds, asking each only after the previous held.
fn steps_hold(mut steps: impl Iterator<Item = (Question, Rule)> + Send + 'static) -> Judge<bool> {
    let Some((question, rule)) = steps.next() else {
        return Judge::done(true);
    };

    single(question, rule).then(move |held| {
        if held {
            steps_hold(steps)
        } else {
            Judge::done(false)
        }
    })
}
