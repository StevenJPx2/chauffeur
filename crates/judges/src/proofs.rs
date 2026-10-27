//! Kani harnesses for settling an answer.

use crate::confidence::{Milli, pick};

fn any_milli() -> Milli {
    let thousandths: u16 = kani::any();
    kani::assume(thousandths <= 1000);
    Milli::new(thousandths).unwrap()
}

fn any_bar() -> Option<Milli> {
    if kani::any() { Some(any_milli()) } else { None }
}

/// For every probability in thousandths and every bar, an answer settles to
/// exactly one outcome or to `unsure`: an outcome exactly when it is the only
/// likeliest one and meets its bar, edges included.
#[kani::proof]
#[kani::unwind(5)]
fn an_answer_settles_to_exactly_one_outcome_or_unsure() {
    let p = [any_milli(), any_milli(), any_milli()];
    let bars = [any_bar(), any_bar(), any_bar()];
    let len: usize = kani::any();
    kani::assume((2..=3).contains(&len));
    let p = &p[..len];

    let picked = pick(p, |index| bars[index]);

    for index in 0..len {
        let only_likeliest = (0..len).all(|other| other == index || p[other] < p[index]);
        let meets = bars[index].is_some_and(|bar| p[index] >= bar);
        assert_eq!(picked == Some(index), only_likeliest && meets);
    }
}
