#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // Failed test assumptions should panic; the reference model is floating point.

extern crate std;

use super::*;

const SCALE: f64 = (1_u64 << FRACTION_BITS) as f64;

/// f64 carries 53 bits, so allow its own rounding on top of the bound.
const FLOAT_SLACK: f64 = 1e-13;

fn real(value: u64) -> f64 {
    value as f64 / SCALE
}

fn samples() -> impl Iterator<Item = u64> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let random = core::iter::from_fn(move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        Some(state >> (state % 64))
    })
    .take(20_000);
    let edges = (0..64).flat_map(|shift| {
        let power = 1_u64 << shift;
        [power.saturating_sub(1), power, power + 1]
    });
    (1..=4_096).chain(edges).chain(random).chain([u64::MAX])
}

#[test]
fn logarithms_are_the_shared_kernel_at_56_fraction_bits() {
    for value in samples() {
        assert_eq!(log2_q56(value), log2_floor::<56>(value));
        assert_eq!(ln_q56(value), ln_floor::<56>(value));
    }
    assert_eq!(log2_q56(0), 0);
    assert_eq!(ln_q56(1), 0);
    assert_eq!(log2_q56(1 << 63), 63 << FRACTION_BITS);
    // Pinned: the values the in-crate logarithm produced before it moved.
    assert_eq!(log2_q56(10), 0x0352_69E1_2F34_6E2B);
    assert_eq!(ln_q56(10), 0x024D_7637_76AA_A2AF);
    assert_eq!(ln_q56(u64::MAX), 0x2C5C_85FD_F473_DE6A);
    let gap = (u64::MAX as f64).ln() - real(ln_q56(u64::MAX));
    assert!(gap > -FLOAT_SLACK && gap < 2f64.powi(-55) + FLOAT_SLACK);
}

#[test]
fn bonus_stays_inside_its_documented_bound() {
    for total in samples().filter(|value| *value > 0) {
        for tries in [1, 2, 3, total / 3, total / 2, total] {
            if tries == 0 {
                continue;
            }
            let exact = (2.0 * (total as f64).ln() / tries as f64).sqrt();
            let gap = exact - real(bonus_q56(total, tries));
            assert!(gap > -1e-9, "bonus({total}, {tries}) overshoots");
            assert!(gap < 2f64.powi(-26), "bonus({total}, {tries})");
        }
    }
    assert_eq!(bonus_q56(1, 1), 0);
    assert_eq!(bonus_q56(0, 0), 0);
    assert_eq!(bonus_q56(u64::MAX, 0), bonus_q56(u64::MAX, 1));
    assert!(real(bonus_q56(u64::MAX, 1)) < 9.5);
}

#[test]
fn bonus_shrinks_with_tries_and_grows_with_total() {
    assert!(bonus_q56(100, 1) > bonus_q56(100, 2));
    assert!(bonus_q56(100, 50) > bonus_q56(100, 100));
    assert!(bonus_q56(1_000, 10) > bonus_q56(100, 10));
}

#[test]
fn mean_floors_and_clamps() {
    assert_eq!(mean_q56(0, 5, 10), 0);
    assert_eq!(mean_q56(50, 5, 10), ONE);
    assert_eq!(mean_q56(25, 5, 10), ONE / 2);
    assert_eq!(mean_q56(1, 3, 1), ONE / 3);
    assert_eq!(mean_q56(99, 1, 10), ONE);
    assert_eq!(mean_q56(7, 0, 0), ONE);
    assert_eq!(mean_q56(u64::MAX, u64::MAX, 1), ONE);
    assert_eq!(
        mean_q56(u64::MAX, u64::MAX, u32::MAX),
        ONE / u64::from(u32::MAX)
    );
}
