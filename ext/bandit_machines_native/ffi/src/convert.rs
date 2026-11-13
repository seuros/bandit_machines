//! Strict Ruby-to-core conversions and the symbols refusals come back as.

use bandit_machines::{
    ArmStats, Closing, Config, Draws, Opening, Ratio, Reason, Refusal, TieBreak,
};
use magnus::{Error, Integer, RClass, RModule, Ruby, Symbol, TryConvert, Value, prelude::*};

fn integer(value: Value) -> Result<(), Error> {
    if Integer::from_value(value).is_some() {
        return Ok(());
    }
    Err(Error::new(
        Ruby::get_with(value).exception_type_error(),
        "expected an Integer",
    ))
}

/// An unsigned 64-bit Integer: draws, serials and counts.
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

/// An issuer scope: an Integer in `0..=65535`.
#[derive(Clone, Copy)]
pub struct Scope(pub u16);

impl TryConvert for Scope {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value)?;
        u16::try_convert(value).map(Self)
    }
}

/// An arm index: any Integer that fits `usize`. Whether it names an arm is
/// the core's to answer (`Refusal::UnknownArm`, or `None` from a query).
#[derive(Clone, Copy)]
pub struct Arm(pub usize);

impl TryConvert for Arm {
    fn try_convert(value: Value) -> Result<Self, Error> {
        integer(value)?;
        usize::try_convert(value).map(Self)
    }
}

/// A value object class defined on the Ruby side with `Data.define`.
pub fn data_class(ruby: &Ruby, name: &str) -> Result<RClass, Error> {
    ruby.class_object()
        .const_get::<_, RModule>("BanditMachines")?
        .const_get(name)
}

fn data<A: magnus::ArgList>(ruby: &Ruby, name: &str, fields: A) -> Result<Value, Error> {
    data_class(ruby, name)?.funcall("new", fields)
}

/// Insist that `value` is the `Data` class `name`, so its readers are the
/// validated fields.
fn expect(ruby: &Ruby, value: Value, name: &str, message: &'static str) -> Result<(), Error> {
    if value.is_kind_of(data_class(ruby, name)?) {
        return Ok(());
    }
    Err(Error::new(ruby.exception_type_error(), message))
}

pub fn ratio(ruby: &Ruby, value: Value) -> Result<Ratio, Error> {
    expect(ruby, value, "Ratio", "expected a BanditMachines::Ratio")?;
    let (numerator, denominator): (Small, Small) = (
        value.funcall("numerator", ())?,
        value.funcall("denominator", ())?,
    );
    Ratio::new(numerator.0, denominator.0).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "ratio needs 0 <= numerator <= denominator and denominator > 0",
        )
    })
}

pub fn ratio_value(ruby: &Ruby, ratio: Ratio) -> Result<Value, Error> {
    data(ruby, "Ratio", (ratio.numerator(), ratio.denominator()))
}

pub fn draws(ruby: &Ruby, value: Value) -> Result<Draws, Error> {
    expect(ruby, value, "Draws", "expected a BanditMachines::Draws")?;
    let (gate, pick): (Number, Number) = (value.funcall("gate", ())?, value.funcall("pick", ())?);
    Ok(Draws {
        gate: gate.0,
        pick: pick.0,
    })
}

fn tie_break(ruby: &Ruby, value: Value) -> Result<TieBreak, Error> {
    let symbol = Symbol::from_value(value)
        .ok_or_else(|| Error::new(ruby.exception_type_error(), "tie_break must be a Symbol"))?;
    match symbol.name()?.as_ref() {
        "lowest_index" => Ok(TieBreak::LowestIndex),
        "highest_index" => Ok(TieBreak::HighestIndex),
        "least_tried" => Ok(TieBreak::LeastTried),
        "draw" => Ok(TieBreak::Draw),
        _ => Err(Error::new(
            ruby.exception_arg_error(),
            "tie_break must be :lowest_index, :highest_index, :least_tried or :draw",
        )),
    }
}

const fn tie_break_name(rule: TieBreak) -> &'static str {
    match rule {
        TieBreak::LowestIndex => "lowest_index",
        TieBreak::HighestIndex => "highest_index",
        TieBreak::LeastTried => "least_tried",
        TieBreak::Draw => "draw",
    }
}

pub fn config(ruby: &Ruby, value: Value) -> Result<Config, Error> {
    expect(ruby, value, "Config", "expected a BanditMachines::Config")?;
    let (max_reward, prior): (Small, Small) = (
        value.funcall("max_reward", ())?,
        value.funcall("prior", ())?,
    );
    let rule = tie_break(ruby, value.funcall("tie_break", ())?)?;
    Config::new(max_reward.0, prior.0, rule).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "max_reward must be positive and prior at most max_reward",
        )
    })
}

pub fn config_value(ruby: &Ruby, config: Config) -> Result<Value, Error> {
    data(
        ruby,
        "Config",
        (
            config.max_reward(),
            config.prior(),
            ruby.to_symbol(tie_break_name(config.tie_break())),
        ),
    )
}

pub fn stats_value(ruby: &Ruby, stats: ArmStats) -> Result<Value, Error> {
    data(
        ruby,
        "ArmStats",
        (
            stats.open,
            stats.generation,
            stats.rewarded,
            stats.reward_sum,
            stats.pending,
        ),
    )
}

pub fn selection_value(ruby: &Ruby, ticket: Value, why: Reason) -> Result<Value, Error> {
    data(ruby, "Selection", (ticket, ruby.to_symbol(reason(why))))
}

const fn reason(reason: Reason) -> &'static str {
    match reason {
        Reason::ColdStart => "cold_start",
        Reason::Explore => "explore",
        Reason::Exploit => "exploit",
        Reason::UpperBound => "upper_bound",
    }
}

pub const fn opening(opening: Opening) -> &'static str {
    match opening {
        Opening::Opened => "opened",
        Opening::AlreadyOpen => "already_open",
    }
}

pub const fn closing(closing: Closing) -> &'static str {
    match closing {
        Closing::Closed => "closed",
        Closing::AlreadyClosed => "already_closed",
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
    };
    ruby.to_symbol(name).as_value()
}
