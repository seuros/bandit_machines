//! Thin binding to the bounded bandit.
mod convert;
mod init;

use std::{cell::RefCell, sync::Arc};

use bandit_machines::{Bandit as Core, Refusal, Selection, Ticket};
use magnus::{Error, Ruby, Symbol, Value, prelude::*, typed_data::Obj};

use convert::{Arm, Number, Small, refusal};

const ARMS: usize = 16;
const TICKETS: usize = 64;

#[magnus::wrap(class = "BanditMachines::Ticket", free_immediately, size)]
struct Handle {
    owner: Arc<()>,
    ticket: Ticket,
}

impl Handle {
    const fn arm(&self) -> usize {
        self.ticket.arm()
    }

    const fn generation(&self) -> u64 {
        self.ticket.generation()
    }

    const fn serial(&self) -> u64 {
        self.ticket.serial()
    }
}

#[magnus::wrap(class = "BanditMachines::Bandit", free_immediately, size)]
struct Bandit {
    owner: Arc<()>,
    core: RefCell<Core<ARMS, TICKETS>>,
}

impl Bandit {
    /// Every arm starts closed, as in the core.
    fn new(ruby: &Ruby, config: Value) -> Result<Self, Error> {
        let config = convert::config(ruby, config)?;
        Ok(Self {
            owner: Arc::new(()),
            core: RefCell::new(Core::new(config)),
        })
    }

    fn config(ruby: &Ruby, this: &Self) -> Result<Value, Error> {
        convert::config_value(ruby, this.core.borrow().config())
    }

    fn selected(
        ruby: &Ruby,
        this: Obj<Self>,
        outcome: Result<Selection, Refusal>,
    ) -> Result<Value, Error> {
        match outcome {
            Ok(selection) => {
                let handle = ruby.obj_wrap(Handle {
                    owner: Arc::clone(&this.owner),
                    ticket: selection.ticket,
                });
                convert::selection_value(ruby, handle.as_value(), selection.reason)
            }
            Err(refused) => Ok(refusal(ruby, refused)),
        }
    }

    fn select_ucb1(ruby: &Ruby, this: Obj<Self>, pick: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().select_ucb1(pick.0);
        Self::selected(ruby, this, outcome)
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
        let outcome = this.core.borrow_mut().select_epsilon_greedy(epsilon, draws);
        Self::selected(ruby, this, outcome)
    }

    /// The core answers `Ok(())`; Ruby names the honoured effect with `done`.
    fn settled(ruby: &Ruby, outcome: Result<(), Refusal>, done: &'static str) -> Value {
        match outcome {
            Ok(()) => ruby.to_symbol(done).as_value(),
            Err(refused) => refusal(ruby, refused),
        }
    }

    fn reward(
        ruby: &Ruby,
        this: Obj<Self>,
        handle: Obj<Handle>,
        reward: Small,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        if !Arc::ptr_eq(&this.owner, &handle.owner) {
            return Ok(refusal(ruby, Refusal::UnknownTicket));
        }
        let outcome = this.core.borrow_mut().reward(handle.ticket, reward.0);
        Ok(Self::settled(ruby, outcome, "rewarded"))
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, handle: Obj<Handle>) -> Result<Value, Error> {
        this.check_frozen()?;
        if !Arc::ptr_eq(&this.owner, &handle.owner) {
            return Ok(refusal(ruby, Refusal::UnknownTicket));
        }
        let outcome = this.core.borrow_mut().abandon(handle.ticket);
        Ok(Self::settled(ruby, outcome, "abandoned"))
    }

    fn revoke_before(this: Obj<Self>, serial: Number) -> Result<usize, Error> {
        this.check_frozen()?;
        Ok(this.core.borrow_mut().revoke_before(serial.0))
    }

    fn voided(ruby: &Ruby, outcome: Result<usize, Refusal>) -> Value {
        match outcome {
            Ok(voided) => ruby.into_value(voided),
            Err(refused) => refusal(ruby, refused),
        }
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
        Ok(match outcome {
            Ok(opening) => ruby.to_symbol(convert::opening(opening)).as_value(),
            Err(refused) => refusal(ruby, refused),
        })
    }

    fn close(ruby: &Ruby, this: Obj<Self>, arm: Arm) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().close(arm.0);
        Ok(match outcome {
            Ok(closing) => ruby.to_symbol(convert::closing(closing)).as_value(),
            Err(refused) => refusal(ruby, refused),
        })
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

    fn next_serial(&self) -> u64 {
        self.core.borrow().next_serial()
    }

    const fn arm_capacity() -> usize {
        ARMS
    }

    const fn ticket_capacity() -> usize {
        TICKETS
    }
}
