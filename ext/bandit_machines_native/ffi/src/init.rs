//! Private native entry point.
use bandit_machines::{Ratio, math};
use magnus::{Error, Ruby, Value, function, method, prelude::*};

use crate::convert::{self, Number, Small};
use crate::{Bandit, Engine, Handle};

fn validate_config(ruby: &Ruby, this: Value) -> Result<(), Error> {
    convert::config(ruby, this).map(|_| ())
}

fn validate_ratio(ruby: &Ruby, this: Value) -> Result<(), Error> {
    convert::ratio(ruby, this).map(|_| ())
}

fn validate_selection(ruby: &Ruby, this: Value) -> Result<(), Error> {
    convert::selection(ruby, this)
}

fn validate_arm_stats(ruby: &Ruby, this: Value) -> Result<(), Error> {
    convert::arm_stats(ruby, this)
}

fn validate_draws(ruby: &Ruby, this: Value) -> Result<(), Error> {
    convert::draws(ruby, this).map(|_| ())
}

const fn log2_q56(value: Number) -> u64 {
    math::log2_q56(value.0)
}

const fn ln_q56(value: Number) -> u64 {
    math::ln_q56(value.0)
}

const fn bonus_q56(total: Number, tries: Number) -> u64 {
    math::bonus_q56(total.0, tries.0)
}

const fn mean_q56(sum: Number, count: Number, range: Small) -> u64 {
    math::mean_q56(sum.0, count.0, range.0)
}

/// The checks of the `Data` values defined in Ruby, and the core's constants.
fn values(ruby: &Ruby) -> Result<(), Error> {
    let classes = convert::classes(ruby)?;
    classes
        .config
        .define_private_method("__validate", method!(validate_config, 0))?;
    classes
        .draws
        .define_private_method("__validate", method!(validate_draws, 0))?;
    classes
        .selection
        .define_private_method("__validate", method!(validate_selection, 0))?;
    classes
        .arm_stats
        .define_private_method("__validate", method!(validate_arm_stats, 0))?;
    let ratio = classes.ratio;
    ratio.define_private_method("__validate", method!(validate_ratio, 0))?;
    ratio.const_set("ZERO", convert::ratio_value(ruby, Ratio::ZERO)?)?;
    ratio.const_set("ONE", convert::ratio_value(ruby, Ratio::ONE)?)?;
    Ok(())
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("BanditMachines")?;

    let math = module.define_module("Math")?;
    math.const_set("FRACTION_BITS", math::FRACTION_BITS)?;
    math.const_set("ONE", math::ONE)?;
    math.define_singleton_method("log2_q56", function!(log2_q56, 1))?;
    math.define_singleton_method("ln_q56", function!(ln_q56, 1))?;
    math.define_singleton_method("bonus_q56", function!(bonus_q56, 2))?;
    math.define_singleton_method("mean_q56", function!(mean_q56, 3))?;

    // Before the values: `Selection` checks its ticket against this class.
    let handle = module.define_class("Ticket", ruby.class_object())?;
    handle.undef_default_alloc_func();
    handle.define_method("arm", method!(Handle::arm, 0))?;
    handle.define_method("generation", method!(Handle::generation, 0))?;
    handle.define_method("serial", method!(Handle::serial, 0))?;
    handle.define_method("scope", method!(Handle::scope, 0))?;
    convert::resolve(ruby, module)?;
    values(ruby)?;

    let class = module.define_class("Bandit", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.const_set("ARMS", Engine::ARMS)?;
    class.const_set("TICKETS", Engine::TICKETS)?;
    class.define_singleton_method("new", function!(Bandit::new, 1))?;
    class.define_singleton_method("with_scope", function!(Bandit::with_scope, 2))?;
    class.define_singleton_method("resumed", function!(Bandit::resumed, 3))?;
    class.define_method("scope", method!(Bandit::scope, 0))?;
    class.define_method("issued", method!(Bandit::issued, 0))?;
    class.define_method("config", method!(Bandit::config, 0))?;
    class.define_method("outstanding", method!(Bandit::outstanding, 0))?;
    class.define_method("next_serial", method!(Bandit::next_serial, 0))?;
    class.define_method("stats", method!(Bandit::stats, 1))?;
    class.define_method("open", method!(Bandit::open, 1))?;
    class.define_method("close", method!(Bandit::close, 1))?;
    class.define_method("select_ucb1", method!(Bandit::select_ucb1, 1))?;
    class.define_method(
        "select_epsilon_greedy",
        method!(Bandit::select_epsilon_greedy, 2),
    )?;
    class.define_method("ucb1_index_q56", method!(Bandit::ucb1_index_q56, 1))?;
    class.define_method("reward", method!(Bandit::reward, 2))?;
    class.define_method("abandon", method!(Bandit::abandon, 1))?;
    class.define_method("revoke_before", method!(Bandit::revoke_before, 1))?;
    class.define_method("revoke_all", method!(Bandit::revoke_all, 0))?;
    class.define_method("reset", method!(Bandit::reset, 1))?;
    class.define_method("reset_all", method!(Bandit::reset_all, 0))?;
    class.define_method("decay", method!(Bandit::decay, 2))?;
    class.define_method("decay_all", method!(Bandit::decay_all, 1))?;
    Ok(())
}
