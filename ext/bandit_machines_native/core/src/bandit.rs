//! The bounded bandit: arms, outstanding tickets and the two policies.

use core::cmp::Ordering;

use crate::math::{bonus_q56, mean_q56};
use crate::pick::{Candidate, best, draw_below};
use crate::{ArmStats, Config, Draws, Ratio, Reason, Refusal, Selection, Ticket, TieBreak};

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
    pub(crate) next_serial: u64,
    outstanding: usize,
}

impl<const ARMS: usize, const TICKETS: usize> Bandit<ARMS, TICKETS> {
    /// Compiled number of arms.
    pub const ARMS: usize = ARMS;

    /// Compiled number of outstanding tickets.
    pub const TICKETS: usize = TICKETS;

    /// A bandit with every arm closed and nothing observed.
    #[must_use]
    pub const fn new(config: Config) -> Self {
        Self {
            config,
            arms: [Arm::NEW; ARMS],
            tickets: [None; TICKETS],
            next_serial: 0,
            outstanding: 0,
        }
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
    #[must_use]
    pub const fn next_serial(&self) -> u64 {
        self.next_serial
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

    /// Make an arm eligible for selection. `Ok(false)` if it already was.
    ///
    /// # Errors
    /// [`Refusal::UnknownArm`] past the compiled capacity.
    pub fn open(&mut self, arm: usize) -> Result<bool, Refusal> {
        self.set_open(arm, true)
    }

    /// Make an arm ineligible. `Ok(false)` if it already was.
    ///
    /// Closing is eligibility only: statistics stay, and outstanding tickets
    /// for the arm are still rewardable.
    ///
    /// # Errors
    /// [`Refusal::UnknownArm`] past the compiled capacity.
    pub fn close(&mut self, arm: usize) -> Result<bool, Refusal> {
        self.set_open(arm, false)
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
        Ok(self.issue(slot, arm, reason))
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
        Ok(self.issue(slot, arm.ok_or(Refusal::NoOpenArm)?, reason))
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
    /// [`Refusal::StaleTicket`] if the arm was reset since the selection;
    /// [`Refusal::UnknownTicket`] if the ticket is spent or foreign;
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
    /// [`Refusal::StaleTicket`] or [`Refusal::UnknownTicket`].
    pub fn abandon(&mut self, ticket: Ticket) -> Result<(), Refusal> {
        self.check(ticket)?;
        self.arms[ticket.arm].pending -= 1;
        self.tickets[ticket.slot] = None;
        self.outstanding -= 1;
        Ok(())
    }

    /// Abandon every outstanding ticket whose serial is below `serial`.
    ///
    /// Returns how many were revoked. Their late rewards answer
    /// [`Refusal::UnknownTicket`].
    pub fn revoke_before(&mut self, serial: u64) -> usize {
        self.void(|pending| pending.serial < serial)
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

    fn set_open(&mut self, arm: usize, open: bool) -> Result<bool, Refusal> {
        let arm = self.arms.get_mut(arm).ok_or(Refusal::UnknownArm)?;
        let changed = arm.open != open;
        arm.open = open;
        Ok(changed)
    }

    /// Everything a selection needs before it may pick: a free ticket slot.
    fn admit(&self) -> Result<usize, Refusal> {
        if !self.arms.iter().any(|arm| arm.open) {
            return Err(Refusal::NoOpenArm);
        }
        let slot = self.tickets.iter().position(Option::is_none);
        let slot = slot.ok_or(Refusal::TicketsFull)?;
        if self.next_serial == u64::MAX {
            return Err(Refusal::Exhausted);
        }
        Ok(slot)
    }

    const fn issue(&mut self, slot: usize, arm: usize, reason: Reason) -> Selection {
        let serial = self.next_serial;
        self.next_serial += 1;
        self.tickets[slot] = Some(Pending { arm, serial });
        self.outstanding += 1;
        let state = &mut self.arms[arm];
        state.pending += 1;
        Selection {
            ticket: Ticket {
                slot,
                arm,
                generation: state.generation,
                serial,
            },
            reason,
        }
    }

    fn check(&self, ticket: Ticket) -> Result<(), Refusal> {
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
            _ => Err(Refusal::UnknownTicket),
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
