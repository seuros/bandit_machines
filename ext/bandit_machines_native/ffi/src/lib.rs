//! Thin binding to the bounded bandit.
mod convert;
mod init;

use std::cell::RefCell;

use bandit_machines::{Bandit as Core, Refusal, Selection, Ticket};
use magnus::{Error, RClass, Ruby, Symbol, Value, prelude::*, typed_data::Obj};

use convert::{Arm, Number, Scope, Small, refusal};

/// The core as compiled for Ruby: 16 arms, 64 outstanding tickets. Everything
/// else reads the core's `Engine::ARMS` and `Engine::TICKETS`.
type Engine = Core<16, 64>;

/// `BanditMachines::Ticket`: the core's `Ticket`. Its serial carries the
/// scope of the bandit that issued it; the core refuses it anywhere else.
#[magnus::wrap(class = "BanditMachines::Ticket", free_immediately, size)]
struct Handle(Ticket);

impl Handle {
    const fn arm(&self) -> usize {
        self.0.arm()
    }

    const fn generation(&self) -> u64 {
        self.0.generation()
    }

    const fn serial(&self) -> u64 {
        self.0.serial()
    }

    const fn scope(&self) -> u16 {
        self.0.scope()
    }
}

#[magnus::wrap(class = "BanditMachines::Bandit", free_immediately, size)]
struct Bandit {
    core: RefCell<Engine>,
}

impl Bandit {
    /// Every arm starts closed, as in the core. Scope zero.
    fn new(ruby: &Ruby, config: Value) -> Result<Self, Error> {
        let config = convert::config(ruby, config)?;
        Ok(Self {
            core: RefCell::new(Core::new(config)),
        })
    }

    fn with_scope(ruby: &Ruby, config: Value, scope: Scope) -> Result<Self, Error> {
        let config = convert::config(ruby, config)?;
        Ok(Self {
            core: RefCell::new(Core::with_scope(config, scope.0)),
        })
    }

    fn resumed(ruby: &Ruby, config: Value, scope: Scope, issued: Number) -> Result<Self, Error> {
        let config = convert::config(ruby, config)?;
        let core = Core::resumed(config, scope.0, issued.0).ok_or_else(|| {
            Error::new(
                ruby.exception_range_error(),
                "issued is past the last sequence of a scope (2**48 - 1)",
            )
        })?;
        Ok(Self {
            core: RefCell::new(core),
        })
    }

    fn scope(&self) -> u16 {
        self.core.borrow().scope()
    }

    fn issued(&self) -> u64 {
        self.core.borrow().issued()
    }

    fn config(ruby: &Ruby, this: &Self) -> Result<Value, Error> {
        convert::config_value(ruby, this.core.borrow().config())
    }

    /// `class` is `Selection`, resolved at init.
    fn selected(
        ruby: &Ruby,
        class: RClass,
        outcome: Result<Selection, Refusal>,
    ) -> Result<Value, Error> {
        match outcome {
            Ok(selection) => {
                let handle = ruby.obj_wrap(Handle(selection.ticket));
                convert::selection_value(ruby, class, handle.as_value(), selection.reason)
            }
            Err(refused) => Ok(refusal(ruby, refused)),
        }
    }

    /// An honoured answer as `honoured` builds it, or the refusal's symbol.
    fn answer<T>(
        ruby: &Ruby,
        outcome: Result<T, Refusal>,
        honoured: impl FnOnce(T) -> Value,
    ) -> Value {
        outcome.map_or_else(|refused| refusal(ruby, refused), honoured)
    }

    fn select_ucb1(ruby: &Ruby, this: Obj<Self>, pick: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let class = convert::classes(ruby)?.selection;
        Self::on_copy(ruby, this, |core| {
            Self::selected(ruby, class, core.select_ucb1(pick.0))
        })
    }

    fn select_epsilon_greedy(
        ruby: &Ruby,
        this: Obj<Self>,
        epsilon: Value,
        draws: Value,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let epsilon = convert::ratio(ruby, epsilon)?;
        let draws = convert::draws(ruby, draws)?;
        let class = convert::classes(ruby)?.selection;
        Self::on_copy(ruby, this, |core| {
            Self::selected(ruby, class, core.select_epsilon_greedy(epsilon, draws))
        })
    }

    /// Runs `act`, which builds the `Selection`, on a copy of the bandit and
    /// keeps the copy only once the answer exists. The answer is built
    /// without running Ruby code; the receiver is still checked frozen and
    /// unchanged right before the store (`FrozenError`, `RuntimeError`), and
    /// in both cases nothing is applied. No borrow is held while `act` runs.
    fn on_copy(
        ruby: &Ruby,
        this: Obj<Self>,
        act: impl FnOnce(&mut Engine) -> Result<Value, Error>,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let before = this.core.borrow().clone();
        let mut after = before.clone();
        let answer = act(&mut after)?;
        this.check_frozen()?;
        let mut held = this.core.borrow_mut();
        if *held != before {
            return Err(Error::new(
                ruby.exception_runtime_error(),
                "the bandit changed while the answer was built; the call was not applied",
            ));
        }
        *held = after;
        Ok(answer)
    }

    /// The core answers `Ok(())`; Ruby names the honoured effect with `done`.
    fn settled(ruby: &Ruby, outcome: Result<(), Refusal>, done: &'static str) -> Value {
        Self::answer(ruby, outcome, |()| ruby.to_symbol(done).as_value())
    }

    fn reward(
        ruby: &Ruby,
        this: Obj<Self>,
        handle: Obj<Handle>,
        reward: Small,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().reward(handle.0, reward.0);
        Ok(Self::settled(ruby, outcome, "rewarded"))
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, handle: Obj<Handle>) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().abandon(handle.0);
        Ok(Self::settled(ruby, outcome, "abandoned"))
    }

    fn revoke_before(ruby: &Ruby, this: Obj<Self>, serial: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().revoke_before(serial.0);
        Ok(Self::voided(ruby, outcome))
    }

    fn revoke_all(this: Obj<Self>) -> Result<usize, Error> {
        this.check_frozen()?;
        Ok(this.core.borrow_mut().revoke_all())
    }

    fn voided(ruby: &Ruby, outcome: Result<usize, Refusal>) -> Value {
        Self::answer(ruby, outcome, |voided| ruby.into_value(voided))
    }

    fn reset(ruby: &Ruby, this: Obj<Self>, arm: Arm) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().reset(arm.0);
        Ok(Self::voided(ruby, outcome))
    }

    fn reset_all(ruby: &Ruby, this: Obj<Self>) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().reset_all();
        Ok(Self::voided(ruby, outcome))
    }

    fn decay(ruby: &Ruby, this: Obj<Self>, arm: Arm, keep: Value) -> Result<Value, Error> {
        this.check_frozen()?;
        let keep = convert::ratio(ruby, keep)?;
        let outcome = this.core.borrow_mut().decay(arm.0, keep);
        Ok(Self::settled(ruby, outcome, "decayed"))
    }

    fn decay_all(ruby: &Ruby, this: Obj<Self>, keep: Value) -> Result<Symbol, Error> {
        this.check_frozen()?;
        let keep = convert::ratio(ruby, keep)?;
        this.core.borrow_mut().decay_all(keep);
        Ok(ruby.to_symbol("decayed"))
    }

    fn open(ruby: &Ruby, this: Obj<Self>, arm: Arm) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().open(arm.0);
        Ok(Self::answer(ruby, outcome, |opening| {
            ruby.to_symbol(convert::opening(opening)).as_value()
        }))
    }

    fn close(ruby: &Ruby, this: Obj<Self>, arm: Arm) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().close(arm.0);
        Ok(Self::answer(ruby, outcome, |closing| {
            ruby.to_symbol(convert::closing(closing)).as_value()
        }))
    }

    fn stats(ruby: &Ruby, this: &Self, arm: Arm) -> Result<Option<Value>, Error> {
        let stats = this.core.borrow().stats(arm.0);
        stats
            .map(|stats| convert::stats_value(ruby, stats))
            .transpose()
    }

    fn ucb1_index_q56(&self, arm: Arm) -> Option<u64> {
        self.core.borrow().ucb1_index_q56(arm.0)
    }

    fn outstanding(&self) -> usize {
        self.core.borrow().outstanding()
    }

    fn next_serial(&self) -> Option<u64> {
        self.core.borrow().next_serial()
    }
}
