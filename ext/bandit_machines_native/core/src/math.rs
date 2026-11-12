//! Deterministic fixed-point UCB1 exploration bonus and empirical mean.
//!
//! Every value is an unsigned Q8.56 number: the integer `v` stands for
//! `v / 2^56`. Only integer operations are used, so results are bit-identical
//! on every target. The logarithms come from `sliderule-machines`; the bonus and
//! the mean are built on them here. Everything rounds towards zero, and the
//! errors are one-sided: the computed value never exceeds the true one.

use sliderule_machines::{ln_floor, log2_floor};

/// Fractional bits of every fixed-point value in this module.
pub const FRACTION_BITS: u32 = 56;

/// `1.0` in Q8.56.
pub const ONE: u64 = 1 << FRACTION_BITS;

/// Binary logarithm of `value` in Q8.56; zero is treated as one.
///
/// Error bound: `0 <= log2(value) - result / 2^56 < 2^-56 + 2^-60`. Exact for
/// powers of two. This is [`sliderule_machines::log2_floor`] at 56 fraction bits.
#[must_use]
pub const fn log2_q56(value: u64) -> u64 {
    log2_floor::<FRACTION_BITS>(value)
}

/// Natural logarithm of `value` in Q8.56; zero is treated as one.
///
/// Error bound: `0 <= ln(value) - result / 2^56 < 2^-55`. This is
/// [`sliderule_machines::ln_floor`] at 56 fraction bits.
#[must_use]
pub const fn ln_q56(value: u64) -> u64 {
    ln_floor::<FRACTION_BITS>(value)
}

/// UCB1 exploration bonus `sqrt(2 * ln(total) / tries)` in Q8.56.
///
/// `tries` of zero is treated as one. Error bound for `tries <= total`:
/// `0 <= sqrt(2 ln(total) / tries) - result / 2^56 < 2^-26`.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // Every narrowed value is below 2^64.
pub const fn bonus_q56(total: u64, tries: u64) -> u64 {
    let tries = if tries == 0 { 1 } else { tries };
    let radicand = (2 * ln_q56(total)) / tries;
    ((radicand as u128) << FRACTION_BITS).isqrt() as u64
}

/// Empirical mean `sum / (count * range)` in Q8.56, clamped to `1.0`.
///
/// `count` or `range` of zero is treated as one. Error bound for
/// `sum <= count * range`: `0 <= mean - result / 2^56 < 2^-56`.
#[must_use]
#[allow(clippy::cast_possible_truncation)] // Every narrowed value is below 2^64.
pub const fn mean_q56(sum: u64, count: u64, range: u32) -> u64 {
    let count = if count == 0 { 1 } else { count };
    let range = if range == 0 { 1 } else { range };
    let scaled = ((sum as u128) << FRACTION_BITS) / ((count as u128) * (range as u128));
    if scaled > ONE as u128 {
        ONE
    } else {
        scaled as u64
    }
}

#[cfg(test)]
mod tests;
