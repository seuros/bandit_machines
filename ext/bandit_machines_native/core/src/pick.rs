//! Allocation-free arg-max over arms with an explicit tie rule.

use crate::TieBreak;

/// Map a uniform draw onto `0..count` as `floor(draw * count / 2^64)`.
#[allow(clippy::cast_possible_truncation)] // The high half of a 128-bit product.
pub const fn draw_below(draw: u64, count: u64) -> u64 {
    let wide = (draw as u128) * (count as u128);
    (wide >> 64) as u64
}

/// One candidate arm as the chooser sees it.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<S> {
    /// Score to maximise.
    pub score: S,
    /// Rewards plus outstanding tickets, for [`TieBreak::LeastTried`].
    pub tries: u64,
}

/// Index of the best-scoring candidate, ties settled by `rule`.
///
/// `candidate(index)` answers `None` for arms that do not take part. The
/// closure is called for every index on each of at most two passes.
pub fn best<S: Ord + Copy>(
    arms: usize,
    rule: TieBreak,
    pick: u64,
    candidate: impl Fn(usize) -> Option<Candidate<S>>,
) -> Option<usize> {
    let mut top: Option<S> = None;
    let mut tied = 0_u64;
    let mut first = None;
    let mut last = None;
    let mut fewest: Option<(u64, usize)> = None;
    for index in 0..arms {
        let Some(entry) = candidate(index) else {
            continue;
        };
        if top.is_none_or(|score| entry.score > score) {
            top = Some(entry.score);
            tied = 0;
            first = None;
            fewest = None;
        }
        if top != Some(entry.score) {
            continue;
        }
        tied += 1;
        first = first.or(Some(index));
        last = Some(index);
        if fewest.is_none_or(|(tries, _)| entry.tries < tries) {
            fewest = Some((entry.tries, index));
        }
    }
    let top = top?;
    match rule {
        TieBreak::LowestIndex => first,
        TieBreak::HighestIndex => last,
        TieBreak::LeastTried => fewest.map(|(_, index)| index),
        TieBreak::Draw => {
            let mut skip = draw_below(pick, tied);
            (0..arms).find(|index| {
                let hit = candidate(*index).is_some_and(|entry| entry.score == top);
                if hit && skip == 0 {
                    return true;
                }
                skip -= u64::from(hit);
                false
            })
        }
    }
}
