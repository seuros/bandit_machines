//! Thin binding to the bounded bandit.
mod convert;
mod init;

use std::{cell::RefCell, sync::Arc};

use bandit_machines::{Bandit as Core, Config, Draws, Refusal, Selection, Ticket};
use magnus::{Error, Ruby, Symbol, Value, prelude::*, typed_data::Obj};

use convert::{Arm, Number, Small, ratio, reason, refusal, tie_break, tie_break_name};

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

    const fn serial(&self) -> u64 {
        self.ticket.serial()
    }

    const fn generation(&self) -> u64 {
        self.ticket.generation()
    }
}

#[magnus::wrap(class = "BanditMachines::Bandit", free_immediately, size)]
struct Bandit {
    owner: Arc<()>,
    core: RefCell<Core<ARMS, TICKETS>>,
}

impl Bandit {
    fn new(
        ruby: &Ruby,
        arms: Number,
        max_reward: Small,
        prior: Small,
        tie: Value,
    ) -> Result<Self, Error> {
        let range = |message| Error::new(ruby.exception_range_error(), message);
        let config = Config::new(max_reward.0, prior.0, tie_break(ruby, tie)?)
            .ok_or_else(|| range("max_reward must be positive and prior at most max_reward"))?;
        let arms = usize::try_from(arms.0)
            .ok()
            .filter(|arms| *arms <= ARMS)
            .ok_or_else(|| range("arms exceeds the compiled arm capacity"))?;
        let mut core = Core::new(config);
        for arm in 0..arms {
            core.open(arm).map_err(|_| range("arm out of range"))?;
        }
        Ok(Self {
            owner: Arc::new(()),
            core: RefCell::new(core),
        })
    }

    fn selected(ruby: &Ruby, this: Obj<Self>, outcome: Result<Selection, Refusal>) -> Value {
        match outcome {
            Ok(selection) => {
                let handle = ruby.obj_wrap(Handle {
                    owner: Arc::clone(&this.owner),
                    ticket: selection.ticket,
                });
                let reason = ruby.to_symbol(reason(selection.reason));
                ruby.into_value((handle, selection.arm(), reason))
            }
            Err(refused) => refusal(ruby, refused),
        }
    }

    fn select_ucb1(ruby: &Ruby, this: Obj<Self>, pick: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().select_ucb1(pick.0);
        Ok(Self::selected(ruby, this, outcome))
    }

    fn select_epsilon_greedy(
        ruby: &Ruby,
        this: Obj<Self>,
        numerator: Small,
        denominator: Small,
        gate: Number,
        pick: Number,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let epsilon = ratio(ruby, numerator, denominator)?;
        let draws = Draws {
            gate: gate.0,
            pick: pick.0,
        };
        let outcome = this.core.borrow_mut().select_epsilon_greedy(epsilon, draws);
        Ok(Self::selected(ruby, this, outcome))
    }

    fn settled(ruby: &Ruby, outcome: Result<(), Refusal>) -> Value {
        match outcome {
            Ok(()) => ruby.qtrue().as_value(),
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
        Ok(Self::settled(ruby, outcome))
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, handle: Obj<Handle>) -> Result<Value, Error> {
        this.check_frozen()?;
        if !Arc::ptr_eq(&this.owner, &handle.owner) {
            return Ok(refusal(ruby, Refusal::UnknownTicket));
        }
        let outcome = this.core.borrow_mut().abandon(handle.ticket);
        Ok(Self::settled(ruby, outcome))
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

    fn reset(ruby: &Ruby, this: Obj<Self>, arm: Arm<ARMS>) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().reset(arm.0);
        Ok(Self::voided(ruby, outcome))
    }

    fn reset_all(ruby: &Ruby, this: Obj<Self>) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().reset_all();
        Ok(Self::voided(ruby, outcome))
    }

    fn decay(
        ruby: &Ruby,
        this: Obj<Self>,
        arm: Arm<ARMS>,
        numerator: Small,
        denominator: Small,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let keep = ratio(ruby, numerator, denominator)?;
        let outcome = this.core.borrow_mut().decay(arm.0, keep);
        Ok(Self::settled(ruby, outcome))
    }

    fn decay_all(
        ruby: &Ruby,
        this: Obj<Self>,
        numerator: Small,
        denominator: Small,
    ) -> Result<bool, Error> {
        this.check_frozen()?;
        let keep = ratio(ruby, numerator, denominator)?;
        this.core.borrow_mut().decay_all(keep);
        Ok(true)
    }

    fn open(ruby: &Ruby, this: Obj<Self>, arm: Arm<ARMS>) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().open(arm.0);
        Ok(outcome.map_or_else(
            |refused| refusal(ruby, refused),
            |changed| ruby.into_value(changed),
        ))
    }

    fn close(ruby: &Ruby, this: Obj<Self>, arm: Arm<ARMS>) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().close(arm.0);
        Ok(outcome.map_or_else(
            |refused| refusal(ruby, refused),
            |changed| ruby.into_value(changed),
        ))
    }

    fn stats(&self, arm: Arm<ARMS>) -> Option<(bool, u64, u64, u64, u64)> {
        let stats = self.core.borrow().stats(arm.0)?;
        Some((
            stats.open,
            stats.generation,
            stats.rewarded,
            stats.reward_sum,
            stats.pending,
        ))
    }

    fn ucb1_index_q56(&self, arm: Arm<ARMS>) -> Option<u64> {
        self.core.borrow().ucb1_index_q56(arm.0)
    }

    fn outstanding(&self) -> usize {
        self.core.borrow().outstanding()
    }

    fn next_serial(&self) -> u64 {
        self.core.borrow().next_serial()
    }

    fn max_reward(&self) -> u32 {
        self.core.borrow().config().max_reward()
    }

    fn prior(&self) -> u32 {
        self.core.borrow().config().prior()
    }

    fn tie_break(ruby: &Ruby, this: Obj<Self>) -> Symbol {
        ruby.to_symbol(tie_break_name(this.core.borrow().config().tie_break()))
    }

    const fn arm_capacity() -> usize {
        ARMS
    }

    const fn ticket_capacity() -> usize {
        TICKETS
    }
}
