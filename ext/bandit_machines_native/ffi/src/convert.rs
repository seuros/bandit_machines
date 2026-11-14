//! Strict Ruby-to-core conversions and the symbols refusals come back as.

use bandit_machines::{
    ArmStats, Closing, Config, Draws, Opening, Ratio, Reason, Refusal, TieBreak,
};
use std::sync::OnceLock;

use magnus::rb_sys::AsRawValue;
use magnus::value::Opaque;
use magnus::{
    Error, Integer, IntoValue, RClass, RModule, RStruct, Ruby, Symbol, TryConvert, Value,
    prelude::*,
};

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

/// The classes, in [`Classes`] field order. `Ticket` is defined by `init`
/// before it resolves them; the rest by `lib/bandit_machines.rb`.
const NAMES: [&str; 6] = [
    "Config",
    "Ratio",
    "Draws",
    "Selection",
    "ArmStats",
    "Ticket",
];

static CLASSES: OnceLock<[Opaque<RClass>; 6]> = OnceLock::new();

/// Every class the binding builds or checks, as resolved at init.
#[derive(Clone, Copy)]
pub struct Classes {
    pub config: RClass,
    pub ratio: RClass,
    pub draws: RClass,
    pub selection: RClass,
    pub arm_stats: RClass,
    pub ticket: RClass,
}

/// Resolves and pins every class, once.
pub fn resolve(ruby: &Ruby, module: RModule) -> Result<(), Error> {
    let class = |name: &str| {
        let class: RClass = module.const_get(name)?;
        // Held in a static, so pinned: `GC.compact` must not move it.
        ruby.gc_register_mark_object(class);
        Ok::<_, Error>(Opaque::from(class))
    };
    let resolved = [
        class(NAMES[0])?,
        class(NAMES[1])?,
        class(NAMES[2])?,
        class(NAMES[3])?,
        class(NAMES[4])?,
        class(NAMES[5])?,
    ];
    // A second load keeps the first classes: they are the same constants.
    let _ = CLASSES.get_or_init(|| resolved);
    Ok(())
}

pub fn classes(ruby: &Ruby) -> Result<Classes, Error> {
    let held = CLASSES.get().ok_or_else(|| {
        Error::new(
            ruby.exception_load_error(),
            "the bandit_machines extension is not initialised",
        )
    })?;
    let [config, ratio, draws, selection, arm_stats, ticket] =
        held.map(|class| ruby.get_inner(class));
    Ok(Classes {
        config,
        ratio,
        draws,
        selection,
        arm_stats,
        ticket,
    })
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

/// The members of `value`, which must be an instance of exactly `class`: the
/// classes compared by identity, which no Ruby method can fake (a subclass's
/// constant leaks past the seal). Read straight from the struct, so no reader
/// runs; `Marshal.load` skips `initialize`, so every caller converts each
/// member again through the core's types.
fn members<const N: usize>(
    ruby: &Ruby,
    value: Value,
    class: RClass,
    message: &'static str,
) -> Result<[Value; N], Error> {
    if value.class().as_raw() != class.as_raw() {
        return Err(Error::new(ruby.exception_type_error(), message));
    }
    let fields = RStruct::try_convert(value)?;
    let mut members = [value; N];
    for (index, member) in members.iter_mut().enumerate() {
        *member = fields.aref(index)?;
    }
    Ok(members)
}

pub fn ratio(ruby: &Ruby, value: Value) -> Result<Ratio, Error> {
    let class = classes(ruby)?.ratio;
    let [numerator, denominator] = members(ruby, value, class, "expected a BanditMachines::Ratio")?;
    let (numerator, denominator) = (
        Small::try_convert(numerator)?,
        Small::try_convert(denominator)?,
    );
    Ratio::new(numerator.0, denominator.0).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "ratio needs 0 <= numerator <= denominator and denominator > 0",
        )
    })
}

pub fn ratio_value(ruby: &Ruby, ratio: Ratio) -> Result<Value, Error> {
    let members = [ratio.numerator(), ratio.denominator()].map(|term| term.into_value_with(ruby));
    build(classes(ruby)?.ratio, members)
}

pub fn draws(ruby: &Ruby, value: Value) -> Result<Draws, Error> {
    let class = classes(ruby)?.draws;
    let [gate, pick] = members(ruby, value, class, "expected a BanditMachines::Draws")?;
    let (gate, pick) = (Number::try_convert(gate)?, Number::try_convert(pick)?);
    Ok(Draws {
        gate: gate.0,
        pick: pick.0,
    })
}

fn tie_break(ruby: &Ruby, value: Value) -> Result<TieBreak, Error> {
    variant(
        ruby,
        value,
        RULES,
        tie_break_name,
        Choices {
            not_symbol: "tie_break must be a Symbol",
            unknown: "tie_break must be :lowest_index, :highest_index, :least_tried or :draw",
        },
    )
}

/// Every `TieBreak`, in declaration order, for parsing a `tie_break` Symbol.
const RULES: [TieBreak; 4] = [
    TieBreak::LowestIndex,
    TieBreak::HighestIndex,
    TieBreak::LeastTried,
    TieBreak::Draw,
];

/// Every `Reason`, in declaration order, for parsing a `reason` Symbol.
const REASONS: [Reason; 4] = [
    Reason::ColdStart,
    Reason::Explore,
    Reason::Exploit,
    Reason::UpperBound,
];

/// What to say when a Symbol field is not a Symbol, or names no variant.
#[derive(Clone, Copy)]
struct Choices {
    not_symbol: &'static str,
    unknown: &'static str,
}

/// The variant among `all` whose snake-case `name` is the Symbol `value`.
fn variant<T: Copy, const N: usize>(
    ruby: &Ruby,
    value: Value,
    all: [T; N],
    name: fn(T) -> &'static str,
    choices: Choices,
) -> Result<T, Error> {
    let symbol = Symbol::from_value(value)
        .ok_or_else(|| Error::new(ruby.exception_type_error(), choices.not_symbol))?;
    let given = symbol.name()?;
    all.into_iter()
        .find(|candidate| name(*candidate) == given.as_ref())
        .ok_or_else(|| Error::new(ruby.exception_range_error(), choices.unknown))
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
    let class = classes(ruby)?.config;
    let [max_reward, prior, rule] =
        members(ruby, value, class, "expected a BanditMachines::Config")?;
    let (max_reward, prior) = (Small::try_convert(max_reward)?, Small::try_convert(prior)?);
    let rule = tie_break(ruby, rule)?;
    Config::new(max_reward.0, prior.0, rule).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "max_reward must be positive and prior at most max_reward",
        )
    })
}

pub fn config_value(ruby: &Ruby, config: Config) -> Result<Value, Error> {
    build(
        classes(ruby)?.config,
        [
            config.max_reward().into_value_with(ruby),
            config.prior().into_value_with(ruby),
            ruby.to_symbol(tie_break_name(config.tie_break()))
                .as_value(),
        ],
    )
}

pub fn stats_value(ruby: &Ruby, stats: ArmStats) -> Result<Value, Error> {
    build(
        classes(ruby)?.arm_stats,
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
    build(class, [ticket, ruby.to_symbol(reason(why)).as_value()])
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
    if ticket.class().as_raw() != known.ticket.as_raw() {
        return Err(Error::new(
            ruby.exception_type_error(),
            "ticket must be a BanditMachines::Ticket",
        ));
    }
    let choices = Choices {
        not_symbol: "reason must be a Symbol",
        unknown: "reason must be :cold_start, :explore, :exploit or :upper_bound",
    };
    variant(ruby, why, REASONS, reason, choices).map(|_| ())
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
    for count in counts {
        Number::try_convert(count)?;
    }
    Ok(())
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
