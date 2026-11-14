//! Strict Ruby-to-core conversions and the symbols refusals come back as.

use bandit_machines::{
    ArmStats, Closing, Config, Draws, Opening, Ratio, Reason, Refusal, TieBreak,
};
use magnus::{
    Error, IntoValue, RClass, RModule, RStruct, Ruby, TryConvert, Value, prelude::*,
    typed_data::Obj,
};

use crate::Handle;
use matryona_machines::ruby::{Class, Registry, Symbols, exact_fields, exact_typed, integer};

/// Every class the binding builds or checks, as resolved at init.
pub struct Classes {
    pub config: Class,
    pub ratio: Class,
    pub draws: Class,
    pub selection: Class,
    pub arm_stats: Class,
    pub ticket: Class,
}

static CLASSES: Registry<Classes> = Registry::new("bandit_machines");

/// Resolves and pins every class, once: `ticket`, which `init` defined, as
/// given, with no lookup back by name; the rest, which
/// `lib/bandit_machines.rb` defined, by name.
pub fn resolve(ruby: &Ruby, module: RModule, ticket: RClass) -> Result<(), Error> {
    let class = |name| Class::resolve(ruby, module, name);
    let classes = Classes {
        config: class("Config")?,
        ratio: class("Ratio")?,
        draws: class("Draws")?,
        selection: class("Selection")?,
        arm_stats: class("ArmStats")?,
        ticket: Class::pin(ruby, ticket),
    };
    CLASSES.set(ruby, classes).map(drop)
}

pub fn classes(ruby: &Ruby) -> Result<&'static Classes, Error> {
    CLASSES.get(ruby)
}

/// Build one of the gem's `Data` values natively from what the core answered:
/// allocate, fill the members by position, freeze. That is all
/// `Data#initialize` does past the check, and the core's own answers need no
/// check, so no Ruby code runs.
pub fn build<const N: usize>(class: RClass, members: [Value; N]) -> Result<Value, Error> {
    let instance = RStruct::try_convert(class.obj_alloc()?)?;
    for (index, member) in members.into_iter().enumerate() {
        instance.aset(index, member)?;
    }
    instance.freeze();
    Ok(instance.as_value())
}

/// The members of `value`, which must be an instance of exactly `class` (a
/// subclass's constant leaks past the seal). Read straight from the struct,
/// so no reader runs; `Marshal.load` skips `initialize`, so every caller
/// converts each member again through the core's types.
fn members<const N: usize>(
    ruby: &Ruby,
    value: Value,
    class: Class,
    message: &'static str,
) -> Result<[Value; N], Error> {
    exact_fields(ruby, value, class.get(ruby), message)
}

pub fn ratio(ruby: &Ruby, value: Value) -> Result<Ratio, Error> {
    let class = classes(ruby)?.ratio;
    let [numerator, denominator] = members(ruby, value, class, "expected a BanditMachines::Ratio")?;
    let numerator = integer(numerator, "numerator")?;
    let denominator = integer(denominator, "denominator")?;
    Ratio::new(numerator, denominator).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "ratio needs 0 <= numerator <= denominator and denominator > 0",
        )
    })
}

pub fn ratio_value(ruby: &Ruby, ratio: Ratio) -> Result<Value, Error> {
    let members = [ratio.numerator(), ratio.denominator()].map(|term| term.into_value_with(ruby));
    build(classes(ruby)?.ratio.get(ruby), members)
}

pub fn draws(ruby: &Ruby, value: Value) -> Result<Draws, Error> {
    let class = classes(ruby)?.draws;
    let [gate, pick] = members(ruby, value, class, "expected a BanditMachines::Draws")?;
    let (gate, pick) = (integer(gate, "gate")?, integer(pick, "pick")?);
    Ok(Draws { gate, pick })
}

matryona_machines::symbols! {
    /// How a tie between equal arms is broken, as `Config#tie_break` holds it.
    pub TieBreaks for TieBreak as "tie_break" {
        LowestIndex => "lowest_index",
        HighestIndex => "highest_index",
        LeastTried => "least_tried",
        Draw => "draw",
    }
}

matryona_machines::symbols! {
    /// Why an arm was selected, as `Selection#reason` holds it.
    pub Reasons for Reason as "reason" {
        ColdStart => "cold_start",
        Explore => "explore",
        Exploit => "exploit",
        UpperBound => "upper_bound",
    }
}

matryona_machines::symbols! {
    /// A refusal as its Symbol: the snake case of the core variant.
    pub Refusals for Refusal as "refusal" {
        UnknownArm => "unknown_arm",
        NoOpenArm => "no_open_arm",
        TicketsFull => "tickets_full",
        Exhausted => "exhausted",
        StaleTicket => "stale_ticket",
        UnknownTicket => "unknown_ticket",
        RewardOutOfRange => "reward_out_of_range",
        Saturated => "saturated",
    }
}

pub fn config(ruby: &Ruby, value: Value) -> Result<Config, Error> {
    let class = classes(ruby)?.config;
    let [max_reward, prior, rule] =
        members(ruby, value, class, "expected a BanditMachines::Config")?;
    let (max_reward, prior) = (integer(max_reward, "max_reward")?, integer(prior, "prior")?);
    let rule = TieBreaks::parse(ruby, rule)?;
    Config::new(max_reward, prior, rule).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "max_reward must be positive and prior at most max_reward",
        )
    })
}

pub fn config_value(ruby: &Ruby, config: Config) -> Result<Value, Error> {
    build(
        classes(ruby)?.config.get(ruby),
        [
            config.max_reward().into_value_with(ruby),
            config.prior().into_value_with(ruby),
            TieBreaks::symbol(ruby, config.tie_break()).as_value(),
        ],
    )
}

pub fn stats_value(ruby: &Ruby, stats: ArmStats) -> Result<Value, Error> {
    build(
        classes(ruby)?.arm_stats.get(ruby),
        [
            stats.open.into_value_with(ruby),
            stats.generation.into_value_with(ruby),
            stats.rewarded.into_value_with(ruby),
            stats.reward_sum.into_value_with(ruby),
            stats.pending.into_value_with(ruby),
        ],
    )
}

/// Build a `Selection` natively, with the class resolved at init: nothing
/// after the core's answer runs Ruby code.
pub fn selection_value(
    ruby: &Ruby,
    class: RClass,
    ticket: Value,
    why: Reason,
) -> Result<Value, Error> {
    build(class, [ticket, Reasons::symbol(ruby, why).as_value()])
}

/// Check a hand-built `Selection`: the ticket a `Ticket`, the reason one of
/// the core's variants. Every such pair is a selection the core could answer.
pub fn selection(ruby: &Ruby, value: Value) -> Result<(), Error> {
    let known = classes(ruby)?;
    let [ticket, why] = members(
        ruby,
        value,
        known.selection,
        "expected a BanditMachines::Selection",
    )?;
    self::ticket(ruby, ticket)?;
    Reasons::parse(ruby, why).map(drop)
}

/// Check a hand-built `ArmStats`: `open` a boolean, every count an unsigned
/// 64-bit Integer. The core's snapshot has no further invariant.
pub fn arm_stats(ruby: &Ruby, value: Value) -> Result<(), Error> {
    let class = classes(ruby)?.arm_stats;
    let [open, counts @ ..] =
        members::<5>(ruby, value, class, "expected a BanditMachines::ArmStats")?;
    if !(open.is_kind_of(ruby.class_true_class()) || open.is_kind_of(ruby.class_false_class())) {
        return Err(Error::new(
            ruby.exception_type_error(),
            "open must be true or false",
        ));
    }
    let names = ["generation", "rewarded", "reward_sum", "pending"];
    for (count, name) in counts.into_iter().zip(names) {
        integer::<u64>(count, name)?;
    }
    Ok(())
}

/// The ticket `value` wraps: exactly a `BanditMachines::Ticket`, a subclass
/// instance refused with `TypeError`.
pub fn ticket(ruby: &Ruby, value: Value) -> Result<Obj<Handle>, Error> {
    let class = classes(ruby)?.ticket.get(ruby);
    exact_typed(
        ruby,
        value,
        class,
        "ticket must be a BanditMachines::Ticket",
    )
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
    Refusals::symbol(ruby, refusal).as_value()
}
