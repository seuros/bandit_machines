//! Naive heap-backed reference bandit: exact rationals for greedy, `f64` for
//! UCB1, tickets as a plain list. Shares no selection code with the core.

extern crate std;

use std::vec::Vec;

use crate::{Ratio, Refusal, TieBreak};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelArm {
    pub open: bool,
    pub generation: u64,
    pub rewarded: u128,
    pub reward_sum: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelTicket {
    pub serial: u64,
    pub arm: usize,
    pub generation: u64,
}

pub struct Model {
    pub max_reward: u32,
    pub prior: u32,
    pub tie: TieBreak,
    pub arms: Vec<ModelArm>,
    pub live: Vec<ModelTicket>,
    pub ticket_capacity: usize,
    /// Plain sequence number of the next ticket, counted from one.
    pub next_serial: u64,
}

impl Model {
    pub fn new(arms: usize, tickets: usize, max_reward: u32, prior: u32, tie: TieBreak) -> Self {
        let arm = ModelArm {
            open: false,
            generation: 0,
            rewarded: 0,
            reward_sum: 0,
        };
        Self {
            max_reward,
            prior,
            tie,
            arms: std::vec![arm; arms],
            live: Vec::new(),
            ticket_capacity: tickets,
            next_serial: 1,
        }
    }

    pub fn pending(&self, arm: usize) -> u64 {
        self.live.iter().filter(|ticket| ticket.arm == arm).count() as u64
    }

    pub fn tries(&self, arm: usize) -> u128 {
        self.arms[arm].rewarded + u128::from(self.pending(arm))
    }

    pub fn open_arms(&self) -> Vec<usize> {
        (0..self.arms.len())
            .filter(|arm| self.arms[*arm].open)
            .collect()
    }

    pub fn select_refusal(&self) -> Option<Refusal> {
        if self.open_arms().is_empty() {
            Some(Refusal::NoOpenArm)
        } else if self.live.len() == self.ticket_capacity {
            Some(Refusal::TicketsFull)
        } else {
            None
        }
    }

    pub fn break_tie(&self, tied: &[usize], pick: u64) -> usize {
        match self.tie {
            TieBreak::LowestIndex => tied[0],
            TieBreak::HighestIndex => tied[tied.len() - 1],
            TieBreak::LeastTried => {
                let fewest = tied.iter().map(|arm| self.tries(*arm)).min().unwrap();
                *tied.iter().find(|arm| self.tries(**arm) == fewest).unwrap()
            }
            TieBreak::Draw => tied[uniform(pick, tied.len())],
        }
    }

    pub fn cold(&self, pick: u64) -> Option<usize> {
        let mut cold = self.open_arms();
        cold.retain(|arm| self.tries(*arm) == 0);
        (!cold.is_empty()).then(|| self.break_tie(&cold, pick))
    }

    /// `(numerator, denominator)` of the mean the arm competes with.
    pub fn mean(&self, arm: usize) -> (u128, u128) {
        let state = &self.arms[arm];
        if state.rewarded == 0 {
            (u128::from(self.prior), 1)
        } else {
            (state.reward_sum, state.rewarded)
        }
    }

    pub fn greedy(&self, pick: u64) -> usize {
        let open = self.open_arms();
        let beats = |a: usize, b: usize| {
            let ((an, ad), (bn, bd)) = (self.mean(a), self.mean(b));
            an * bd > bn * ad
        };
        let tied: Vec<usize> = open
            .iter()
            .copied()
            .filter(|arm| !open.iter().any(|other| beats(*other, *arm)))
            .collect();
        self.break_tie(&tied, pick)
    }

    pub fn explore(&self, pick: u64) -> usize {
        let open = self.open_arms();
        open[uniform(pick, open.len())]
    }

    /// Floating-point UCB1 index of every open arm.
    pub fn ucb1(&self) -> Vec<(usize, f64)> {
        let open = self.open_arms();
        let total: u128 = open.iter().map(|arm| self.tries(*arm)).sum();
        open.iter()
            .map(|arm| {
                let (sum, count) = self.mean(*arm);
                let mean = sum as f64 / (count as f64 * f64::from(self.max_reward));
                let bonus = (2.0 * (total as f64).ln() / self.tries(*arm) as f64).sqrt();
                (*arm, mean + bonus)
            })
            .collect()
    }

    pub fn issue(&mut self, arm: usize) -> ModelTicket {
        let ticket = ModelTicket {
            serial: self.next_serial,
            arm,
            generation: self.arms[arm].generation,
        };
        self.next_serial += 1;
        self.live.push(ticket);
        ticket
    }

    /// `reward` of `None` abandons.
    pub fn redeem(&mut self, ticket: ModelTicket, reward: Option<u32>) -> Result<(), Refusal> {
        if self.arms[ticket.arm].generation != ticket.generation {
            return Err(Refusal::StaleTicket);
        }
        let Some(position) = self.live.iter().position(|live| *live == ticket) else {
            return Err(Refusal::UnknownTicket);
        };
        if let Some(reward) = reward {
            if reward > self.max_reward {
                return Err(Refusal::RewardOutOfRange);
            }
            self.arms[ticket.arm].rewarded += 1;
            self.arms[ticket.arm].reward_sum += u128::from(reward);
        }
        self.live.remove(position);
        Ok(())
    }

    pub fn revoke_before(&mut self, serial: u64) -> usize {
        let before = self.live.len();
        self.live.retain(|ticket| ticket.serial >= serial);
        before - self.live.len()
    }

    pub fn revoke_all(&mut self) -> usize {
        let revoked = self.live.len();
        self.live.clear();
        revoked
    }

    pub fn reset(&mut self, arm: usize) -> usize {
        let before = self.live.len();
        self.live.retain(|ticket| ticket.arm != arm);
        let state = &mut self.arms[arm];
        state.generation += 1;
        state.rewarded = 0;
        state.reward_sum = 0;
        before - self.live.len()
    }

    pub fn decay(&mut self, arm: usize, keep: Ratio) {
        let (n, d) = (u128::from(keep.numerator()), u128::from(keep.denominator()));
        let state = &mut self.arms[arm];
        let scaled = state.rewarded * n;
        state.rewarded = scaled / d + u128::from(!scaled.is_multiple_of(d));
        state.reward_sum = state.reward_sum * n / d;
    }
}

/// Does a gate draw explore? Decided by long division instead of a shift.
pub fn explores(epsilon: Ratio, gate: u64) -> bool {
    let bucket = u128::from(gate) * u128::from(epsilon.denominator()) / (1_u128 << 64);
    bucket < u128::from(epsilon.numerator())
}

fn uniform(pick: u64, count: usize) -> usize {
    (u128::from(pick) * count as u128 / (1_u128 << 64)) as usize
}
