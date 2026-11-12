//! Strict Ruby-to-core conversions and the symbols refusals come back as.

use bandit_machines::{Ratio, Reason, Refusal, TieBreak};
use magnus::{Error, Integer, Ruby, Symbol, TryConvert, Value, prelude::*};

fn integer(value: Value) -> Result<(), Error> {
    if Integer::from_value(value).is_some() {
        return Ok(());
    }
    Err(Error::new(
        Ruby::get_with(value).exception_type_error(),
        "expected an Integer",
    ))
}

/// An unsigned 64-bit Integer: draws and serials.
#[derive(Clone, Copy)]
pub struct Number(pub u64);

impl TryConvert for Number {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value)?;
        u64::try_convert(value).map(Self)
    }
}

/// An unsigned 32-bit Integer: rewards and ratio terms.
#[derive(Clone, Copy)]
pub struct Small(pub u32);

impl TryConvert for Small {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value)?;
        u32::try_convert(value).map(Self)
    }
}

/// An arm index below the compiled capacity.
#[derive(Clone, Copy)]
pub struct Arm<const ARMS: usize>(pub usize);

impl<const ARMS: usize> TryConvert for Arm<ARMS> {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value)?;
        let index = usize::try_convert(value)?;
        if index < ARMS {
            return Ok(Self(index));
        }
        Err(Error::new(
            Ruby::get_with(value).exception_range_error(),
            "arm index exceeds the compiled arm capacity",
        ))
    }
}

pub fn ratio(ruby: &Ruby, numerator: Small, denominator: Small) -> Result<Ratio, Error> {
    Ratio::new(numerator.0, denominator.0).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "ratio needs 0 <= numerator <= denominator and denominator > 0",
        )
    })
}

const TIE_BREAKS: [(&str, TieBreak); 4] = [
    ("lowest_index", TieBreak::LowestIndex),
    ("highest_index", TieBreak::HighestIndex),
    ("least_tried", TieBreak::LeastTried),
    ("draw", TieBreak::Draw),
];

pub fn tie_break(ruby: &Ruby, value: Value) -> Result<TieBreak, Error> {
    let symbol = Symbol::from_value(value)
        .ok_or_else(|| Error::new(ruby.exception_type_error(), "tie_break must be a Symbol"))?;
    let name = symbol.name()?;
    TIE_BREAKS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, rule)| *rule)
        .ok_or_else(|| {
            Error::new(
                ruby.exception_arg_error(),
                "tie_break must be :lowest_index, :highest_index, :least_tried or :draw",
            )
        })
}

pub fn tie_break_name(rule: TieBreak) -> &'static str {
    TIE_BREAKS
        .iter()
        .find(|(_, known)| *known == rule)
        .map_or("unknown", |(name, _)| name)
}

pub const fn reason(reason: Reason) -> &'static str {
    match reason {
        Reason::ColdStart => "cold_start",
        Reason::Explore => "explore",
        Reason::Exploit => "exploit",
        Reason::UpperBound => "upper_bound",
        _ => "unknown",
    }
}

pub fn refusal(ruby: &Ruby, refusal: Refusal) -> Value {
    let name = match refusal {
        Refusal::UnknownArm => "unknown_arm",
        Refusal::NoOpenArm => "no_open_arm",
        Refusal::TicketsFull => "tickets_full",
        Refusal::Exhausted => "exhausted",
        Refusal::StaleTicket => "stale_ticket",
        Refusal::UnknownTicket => "unknown_ticket",
        Refusal::RewardOutOfRange => "reward_out_of_range",
        Refusal::Saturated => "saturated",
        _ => "refused",
    };
    ruby.to_symbol(name).as_value()
}
