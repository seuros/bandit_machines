//! Private native entry point.
use crate::{Bandit, Handle};
use magnus::{Error, Ruby, function, method, prelude::*};

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("BanditMachines")?;
    module.const_set("ARM_CAPACITY", Bandit::arm_capacity())?;
    module.const_set("TICKET_CAPACITY", Bandit::ticket_capacity())?;
    let handle = module.define_class("Ticket", ruby.class_object())?;
    handle.undef_default_alloc_func();
    handle.define_method("arm", method!(Handle::arm, 0))?;
    handle.define_method("serial", method!(Handle::serial, 0))?;
    handle.define_method("generation", method!(Handle::generation, 0))?;
    let class = module.define_class("Bandit", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.define_singleton_method("_new", function!(Bandit::new, 4))?;
    class.define_method("_select_ucb1", method!(Bandit::select_ucb1, 1))?;
    class.define_method(
        "_select_epsilon_greedy",
        method!(Bandit::select_epsilon_greedy, 4),
    )?;
    class.define_method("reward", method!(Bandit::reward, 2))?;
    class.define_method("abandon", method!(Bandit::abandon, 1))?;
    class.define_method("revoke_before", method!(Bandit::revoke_before, 1))?;
    class.define_method("reset", method!(Bandit::reset, 1))?;
    class.define_method("reset_all", method!(Bandit::reset_all, 0))?;
    class.define_method("_decay", method!(Bandit::decay, 3))?;
    class.define_method("_decay_all", method!(Bandit::decay_all, 2))?;
    class.define_method("open", method!(Bandit::open, 1))?;
    class.define_method("close", method!(Bandit::close, 1))?;
    class.define_method("_stats", method!(Bandit::stats, 1))?;
    class.define_method("ucb1_index_q56", method!(Bandit::ucb1_index_q56, 1))?;
    class.define_method("outstanding", method!(Bandit::outstanding, 0))?;
    class.define_method("next_serial", method!(Bandit::next_serial, 0))?;
    class.define_method("max_reward", method!(Bandit::max_reward, 0))?;
    class.define_method("prior", method!(Bandit::prior, 0))?;
    class.define_method("tie_break", method!(Bandit::tie_break, 0))?;
    class.define_method("arm_capacity", function!(Bandit::arm_capacity, 0))?;
    class.define_method("ticket_capacity", function!(Bandit::ticket_capacity, 0))?;
    Ok(())
}
