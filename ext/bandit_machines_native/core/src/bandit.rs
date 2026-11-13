//! The bounded bandit: arms, outstanding tickets and the two policies.

use core::cmp::Ordering;

use periscope_machines::{Issuer, scope_of};

use crate::math::{bonus_q56, mean_q56};
use crate::pick::{Candidate, best, draw_below};
use crate::{
    ArmStats, Closing, Config, Draws, Opening, Ratio, Reason, Refusal, Selection, Ticket, TieBreak,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Arm {
    open: bool,
    generation: u64,
    rewarded: u64,
    reward_sum: u64,
    pending: u64,
}

impl Arm {
    const NEW: Self = Self {
        open: false,
        generation: 0,
        rewarded: 0,
        reward_sum: 0,
        pending: 0,
    };

    const fn tries(&self) -> u64 {
        self.rewarded.saturating_add(self.pending)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    arm: usize,
    serial: u64,
}

/// An exact mean `sum / count`, ordered by cross-multiplication.
#[derive(Debug, Clone, Copy)]
struct Mean {
    sum: u64,
    count: u64,
}

impl Ord for Mean {
    fn cmp(&self, other: &Self) -> Ordering {
        let left = u128::from(self.sum) * u128::from(other.count);
        let right = u128::from(other.sum) * u128::from(self.count);
        left.cmp(&right)
    }
}

impl PartialOrd for Mean {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Mean {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Mean {}

/// `ARMS` arms and at most `TICKETS` selections awaiting their reward.
///
/// See the crate documentation for the cold-start, tie, delayed-reward and
/// nonstationarity contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bandit<const ARMS: usize, const TICKETS: usize> {
    pub(crate) config: Config,
    arms: [Arm; ARMS],
    tickets: [Option<Pending>; TICKETS],
    /// Issues ticket serials; never reissues one.
    issuer: Issuer,
    outstanding: usize,
}

impl<const ARMS: usize, const TICKETS: usize> Bandit<ARMS, TICKETS> {
    /// Compiled number of arms.
    pub const ARMS: usize = ARMS;

    /// Compiled number of outstanding tickets.
    pub const TICKETS: usize = TICKETS;

    /// A bandit with every arm closed and nothing observed, in scope zero.
    #[must_use]
    pub const fn new(config: Config) -> Self {
        Self::with_scope(config, 0)
    }

    /// A bandit with every arm closed and nothing observed, whose tickets
    /// carry `scope`.
    ///
    /// The scope is the bandit's identity, not policy, so it is not part of
    /// [`Config`]: replicas share a configuration and must not share a scope.
    /// Tickets are only meaningful to the bandit that issued them, and two
    /// bandits in the same scope issue the same tickets.
    #[must_use]
    pub const fn with_scope(config: Config, scope: u16) -> Self {
        Self::build(config, Issuer::new(scope))
    }

    /// A bandit that continues `scope` after a restart: `issued` is what
    /// [`issued`](Self::issued) answered when the caller saved it.
    ///
    /// Only the serial sequence is restored. Every arm is closed with nothing
    /// observed and generation zero, and no ticket is outstanding: a ticket
    /// issued before the restart is refused ([`Refusal::UnknownTicket`], or
    /// [`Refusal::StaleTicket`] when its arm's generation differs), and
    /// reopening arms and restoring what they learned is the caller's job.
    /// The next ticket carries sequence `issued + 1`, so no serial is
    /// reissued as long as the saved count is not lower than what the bandit
    /// had really issued; save it before handing a ticket out.
    ///
    /// `None` when `issued` is beyond `periscope_machines::SEQUENCE_MAX`,
    /// which no bandit reaches.
    #[must_use]
    pub const fn resumed(config: Config, scope: u16, issued: u64) -> Option<Self> {
        match Issuer::resumed(scope, issued) {
            Some(sequence) => Some(Self::build(config, sequence)),
            None => None,
        }
    }

    const fn build(config: Config, issuer: Issuer) -> Self {
        Self {
            config,
            arms: [Arm::NEW; ARMS],
            tickets: [None; TICKETS],
            issuer,
            outstanding: 0,
        }
    }

    /// The scope carried by every ticket this bandit issues.
    #[must_use]
    pub const fn scope(&self) -> u16 {
        self.issuer.scope()
    }

    /// Serials issued in this scope so far, outstanding or not: the count to
    /// save for [`resumed`](Self::resumed).
    #[must_use]
    pub const fn issued(&self) -> u64 {
        self.issuer.issued()
    }

    /// The configuration this bandit was built with.
    #[must_use]
    pub const fn config(&self) -> Config {
        self.config
    }

    /// Tickets issued and not yet rewarded, abandoned, revoked or voided.
    #[must_use]
    pub const fn outstanding(&self) -> usize {
        self.outstanding
    }

    /// Serial the next ticket will carry; every issued serial is below it.
    /// `None` once every serial of the scope has been issued.
    #[must_use]
    pub const fn next_serial(&self) -> Option<u64> {
        match self.issuer.peek() {
            Ok(serial) => Some(serial),
            Err(periscope_machines::Refusal::Exhausted) => None,
        }
    }

    /// Snapshot of one arm, or `None` past the compiled capacity.
    #[must_use]
    pub fn stats(&self, arm: usize) -> Option<ArmStats> {
        self.arms.get(arm).map(|arm| ArmStats {
            open: arm.open,
            generation: arm.generation,
            rewarded: arm.rewarded,
            reward_sum: arm.reward_sum,
            pending: arm.pending,
        })
    }

    /// Make an arm eligible for selection.
    ///
    /// [`Opening::AlreadyOpen`] is an idempotent repeat: it changes nothing.
    ///
    /// # Errors
    /// [`Refusal::UnknownArm`] past the compiled capacity.
    pub fn open(&mut self, arm: usize) -> Result<Opening, Refusal> {
        let arm = self.arms.get_mut(arm).ok_or(Refusal::UnknownArm)?;
        if arm.open {
            return Ok(Opening::AlreadyOpen);
        }
        arm.open = true;
        Ok(Opening::Opened)
    }

    /// Make an arm ineligible.
    ///
    /// Closing is eligibility only: statistics stay, and outstanding tickets
    /// for the arm are still rewardable. [`Closing::AlreadyClosed`] is an
    /// idempotent repeat: it changes nothing.
    ///
    /// # Errors
    /// [`Refusal::UnknownArm`] past the compiled capacity.
    pub fn close(&mut self, arm: usize) -> Result<Closing, Refusal> {
        let arm = self.arms.get_mut(arm).ok_or(Refusal::UnknownArm)?;
        if !arm.open {
            return Ok(Closing::AlreadyClosed);
        }
        arm.open = false;
        Ok(Closing::Closed)
    }

    /// Select by UCB1 and issue a ticket for the chosen arm.
    ///
    /// `pick` is consumed only under [`TieBreak::Draw`].
    ///
    /// # Errors
    /// [`Refusal::NoOpenArm`], [`Refusal::TicketsFull`] or
    /// [`Refusal::Exhausted`].
    pub fn select_ucb1(&mut self, pick: u64) -> Result<Selection, Refusal> {
        let slot = self.admit()?;
        let (arm, reason) = if let Some(arm) = self.cold(pick) {
            (arm, Reason::ColdStart)
        } else {
            let total = self.total_tries();
            let rule = self.config.tie_break;
            let arm = self.choose(rule, pick, |index| self.ucb1_index(index, total));
            (arm.ok_or(Refusal::NoOpenArm)?, Reason::UpperBound)
        };
        self.issue(slot, arm, reason)
    }

    /// Select by epsilon-greedy and issue a ticket for the chosen arm.
    ///
    /// With probability `epsilon` (decided by `draws.gate`) the arm is uniform
    /// over the open arms (by `draws.pick`); otherwise it is the highest exact
    /// mean, ties settled by the configured rule.
    ///
    /// # Errors
    /// [`Refusal::NoOpenArm`], [`Refusal::TicketsFull`] or
    /// [`Refusal::Exhausted`].
    pub fn select_epsilon_greedy(
        &mut self,
        epsilon: Ratio,
        draws: Draws,
    ) -> Result<Selection, Refusal> {
        let slot = self.admit()?;
        let explore = draw_below(draws.gate, u64::from(epsilon.denominator()))
            < u64::from(epsilon.numerator());
        let (arm, reason) = if let Some(arm) = self.cold(draws.pick) {
            (Some(arm), Reason::ColdStart)
        } else if explore {
            let arm = self.choose(TieBreak::Draw, draws.pick, |_| Some(()));
            (arm, Reason::Explore)
        } else {
            let arm = self.choose(self.config.tie_break, draws.pick, |index| self.mean(index));
            (arm, Reason::Exploit)
        };
        self.issue(slot, arm.ok_or(Refusal::NoOpenArm)?, reason)
    }

    /// UCB1 index of an open arm in Q8.56, in units of the reward range:
    /// `mean / max_reward + sqrt(2 ln(total tries) / tries)`.
    ///
    /// `None` for an unknown, closed or cold arm (a cold arm's index is
    /// unbounded; it is selected before any index is compared).
    #[must_use]
    pub fn ucb1_index_q56(&self, arm: usize) -> Option<u64> {
        self.ucb1_index(arm, self.total_tries())
    }

    /// Redeem a ticket with its reward.
    ///
    /// # Errors
    /// [`Refusal::UnknownTicket`] if this bandit never issued the ticket
    /// (another scope) or it is spent; [`Refusal::StaleTicket`] if the arm
    /// was reset since the selection;
    /// [`Refusal::RewardOutOfRange`] or [`Refusal::Saturated`] leave the
    /// ticket outstanding so it can be redeemed again.
    pub fn reward(&mut self, ticket: Ticket, reward: u32) -> Result<(), Refusal> {
        self.check(ticket)?;
        if reward > self.config.max_reward {
            return Err(Refusal::RewardOutOfRange);
        }
        let arm = &mut self.arms[ticket.arm];
        let rewarded = arm.rewarded.checked_add(1).ok_or(Refusal::Saturated)?;
        let reward_sum = arm
            .reward_sum
            .checked_add(u64::from(reward))
            .ok_or(Refusal::Saturated)?;
        arm.rewarded = rewarded;
        arm.reward_sum = reward_sum;
        arm.pending -= 1;
        self.tickets[ticket.slot] = None;
        self.outstanding -= 1;
        Ok(())
    }

    /// Give a ticket back without a reward: the selection never happened.
    ///
    /// # Errors
    /// [`Refusal::UnknownTicket`] or [`Refusal::StaleTicket`], as for
    /// [`reward`](Self::reward).
    pub fn abandon(&mut self, ticket: Ticket) -> Result<(), Refusal> {
        self.check(ticket)?;
        self.arms[ticket.arm].pending -= 1;
        self.tickets[ticket.slot] = None;
        self.outstanding -= 1;
        Ok(())
    }

    /// Abandon every outstanding ticket whose serial is below `serial`.
    ///
    /// `serial` is a cut in this bandit's selection order: a ticket's
    /// [`serial`](Ticket::serial) or an earlier [`next_serial`](Self::next_serial).
    /// Returns how many were revoked. Their late rewards answer
    /// [`Refusal::UnknownTicket`].
    ///
    /// # Errors
    /// [`Refusal::UnknownTicket`] for a serial of another scope: it orders
    /// nothing here, and nothing is revoked. No serial is a cut above every
    /// ticket (the last serial of a scope can be outstanding, and `u64::MAX`
    /// belongs to another scope): to revoke them all, call
    /// [`revoke_all`](Self::revoke_all).
    pub fn revoke_before(&mut self, serial: u64) -> Result<usize, Refusal> {
        if scope_of(serial) != self.issuer.scope() {
            return Err(Refusal::UnknownTicket);
        }
        Ok(self.void(|pending| pending.serial < serial))
    }

    /// Abandon every outstanding ticket, whatever its serial. Returns how
    /// many were revoked. Their late rewards answer [`Refusal::UnknownTicket`].
    ///
    /// Arm statistics and generations are untouched: unlike
    /// [`reset_all`](Self::reset_all), nothing learned is forgotten. It
    /// cannot refuse, and with nothing outstanding it changes nothing.
    pub fn revoke_all(&mut self) -> usize {
        self.void(|_| true)
    }

    /// Forget one arm: zero its statistics, bump its generation and void its
    /// outstanding tickets. Eligibility is kept. Returns the tickets voided.
    ///
    /// # Errors
    /// [`Refusal::UnknownArm`], or [`Refusal::Exhausted`] when the arm has no
    /// generation left.
    pub fn reset(&mut self, arm: usize) -> Result<usize, Refusal> {
        let state = self.arms.get_mut(arm).ok_or(Refusal::UnknownArm)?;
        let generation = state.generation.checked_add(1).ok_or(Refusal::Exhausted)?;
        *state = Arm {
            open: state.open,
            generation,
            ..Arm::NEW
        };
        Ok(self.void(|pending| pending.arm == arm))
    }

    /// [`reset`](Self::reset) every arm, or none of them.
    ///
    /// # Errors
    /// [`Refusal::Exhausted`] when any arm has no generation left.
    pub fn reset_all(&mut self) -> Result<usize, Refusal> {
        if self.arms.iter().any(|arm| arm.generation == u64::MAX) {
            return Err(Refusal::Exhausted);
        }
        for arm in &mut self.arms {
            *arm = Arm {
                open: arm.open,
                generation: arm.generation + 1,
                ..Arm::NEW
            };
        }
        Ok(self.void(|_| true))
    }

    /// Discount one arm's history by `keep`: the reward count becomes
    /// `ceil(count * keep)` and the reward sum `floor(sum * keep)`.
    ///
    /// The generation and outstanding tickets are untouched, so late rewards
    /// still land, at full weight. Only `keep` of zero makes an arm cold.
    ///
    /// # Errors
    /// [`Refusal::UnknownArm`] past the compiled capacity.
    pub fn decay(&mut self, arm: usize, keep: Ratio) -> Result<(), Refusal> {
        let arm = self.arms.get_mut(arm).ok_or(Refusal::UnknownArm)?;
        discount(arm, keep);
        Ok(())
    }

    /// [`decay`](Self::decay) every arm.
    pub fn decay_all(&mut self, keep: Ratio) {
        for arm in &mut self.arms {
            discount(arm, keep);
        }
    }

    /// Everything a selection needs before it may pick: an open arm, a free
    /// ticket slot and a serial left to issue.
    fn admit(&self) -> Result<usize, Refusal> {
        if !self.arms.iter().any(|arm| arm.open) {
            return Err(Refusal::NoOpenArm);
        }
        let slot = self.tickets.iter().position(Option::is_none);
        let slot = slot.ok_or(Refusal::TicketsFull)?;
        match self.issuer.peek() {
            Ok(_) => Ok(slot),
            Err(periscope_machines::Refusal::Exhausted) => Err(Refusal::Exhausted),
        }
    }

    /// The serial is issued first and is the only step that can refuse, so a
    /// refused selection spends no serial and changes nothing.
    const fn issue(
        &mut self,
        slot: usize,
        arm: usize,
        reason: Reason,
    ) -> Result<Selection, Refusal> {
        let serial = match self.issuer.issue() {
            Ok(serial) => serial,
            Err(periscope_machines::Refusal::Exhausted) => return Err(Refusal::Exhausted),
        };
        self.tickets[slot] = Some(Pending { arm, serial });
        self.outstanding += 1;
        let state = &mut self.arms[arm];
        state.pending += 1;
        Ok(Selection {
            ticket: Ticket {
                slot,
                arm,
                generation: state.generation,
                serial,
            },
            reason,
        })
    }

    /// Origin first: a ticket this bandit's issuer never handed out names
    /// nothing here, whatever slot, arm and generation it carries.
    fn check(&self, ticket: Ticket) -> Result<(), Refusal> {
        if !self.issuer.owns(ticket.serial) {
            return Err(Refusal::UnknownTicket);
        }
        let arm = self.arms.get(ticket.arm).ok_or(Refusal::UnknownTicket)?;
        if arm.generation != ticket.generation {
            return Err(Refusal::StaleTicket);
        }
        let expected = Pending {
            arm: ticket.arm,
            serial: ticket.serial,
        };
        match self.tickets.get(ticket.slot) {
            Some(Some(pending)) if *pending == expected => Ok(()),
            Some(Some(_) | None) | None => Err(Refusal::UnknownTicket),
        }
    }

    fn void(&mut self, doomed: impl Fn(&Pending) -> bool) -> usize {
        let mut voided = 0;
        for slot in &mut self.tickets {
            let Some(pending) = slot.filter(|pending| doomed(pending)) else {
                continue;
            };
            let arm = &mut self.arms[pending.arm];
            arm.pending = arm.pending.saturating_sub(1);
            *slot = None;
            voided += 1;
        }
        self.outstanding -= voided;
        voided
    }

    fn total_tries(&self) -> u64 {
        self.arms
            .iter()
            .filter(|arm| arm.open)
            .fold(0, |total, arm| total.saturating_add(arm.tries()))
    }

    fn choose<S: Ord + Copy>(
        &self,
        rule: TieBreak,
        pick: u64,
        score: impl Fn(usize) -> Option<S>,
    ) -> Option<usize> {
        best(ARMS, rule, pick, |index| {
            let arm = self.arms.get(index).filter(|arm| arm.open)?;
            Some(Candidate {
                score: score(index)?,
                tries: arm.tries(),
            })
        })
    }

    /// An open arm nobody has tried, if there is one.
    fn cold(&self, pick: u64) -> Option<usize> {
        self.choose(self.config.tie_break, pick, |index| {
            (self.arms[index].tries() == 0).then_some(())
        })
    }

    fn mean(&self, arm: usize) -> Option<Mean> {
        let arm = self.arms.get(arm)?;
        Some(if arm.rewarded == 0 {
            Mean {
                sum: u64::from(self.config.prior),
                count: 1,
            }
        } else {
            Mean {
                sum: arm.reward_sum,
                count: arm.rewarded,
            }
        })
    }

    fn ucb1_index(&self, arm: usize, total: u64) -> Option<u64> {
        let state = self.arms.get(arm).filter(|arm| arm.open)?;
        if state.tries() == 0 {
            return None;
        }
        let mean = self.mean(arm)?;
        let mean = mean_q56(mean.sum, mean.count, self.config.max_reward);
        Some(mean + bonus_q56(total, state.tries()))
    }
}

fn discount(arm: &mut Arm, keep: Ratio) {
    let numerator = u128::from(keep.numerator());
    let denominator = u128::from(keep.denominator());
    let rewarded = (u128::from(arm.rewarded) * numerator).div_ceil(denominator);
    let reward_sum = u128::from(arm.reward_sum) * numerator / denominator;
    arm.rewarded = u64::try_from(rewarded).unwrap_or(u64::MAX);
    arm.reward_sum = u64::try_from(reward_sum).unwrap_or(u64::MAX);
}

#[cfg(test)]
impl<const ARMS: usize, const TICKETS: usize> Bandit<ARMS, TICKETS> {
    /// Test hook: overwrite one arm's counters to reach `u64` limits.
    pub(crate) fn force(&mut self, arm: usize, generation: u64, rewarded: u64, reward_sum: u64) {
        let arm = &mut self.arms[arm];
        arm.generation = generation;
        arm.rewarded = rewarded;
        arm.reward_sum = reward_sum;
    }
}
