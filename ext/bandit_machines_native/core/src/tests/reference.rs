//! Randomised run of the core against the naive model.

extern crate std;

use std::vec::Vec;

use periscope_machines::{scoped, sequence_of};

use super::model::{Model, ModelTicket, explores};
use crate::{
    Bandit, Closing, Config, Draws, Opening, Ratio, Reason, Refusal, Selection, Ticket, TieBreak,
};

const ARMS: usize = 5;
const TICKETS: usize = 6;
const MAX_REWARD: u32 = 10;
const PRIOR: u32 = 4;

/// The core's documented UCB1 index error, plus `f64` rounding.
const INDEX_SLACK: f64 = 1.0 / (1_u64 << 26) as f64 + 1e-12;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

struct Run {
    core: Bandit<ARMS, TICKETS>,
    model: Model,
    rng: Rng,
    scope: u16,
    /// Every ticket ever issued, spent or not, so stale ones get replayed.
    issued: Vec<(Ticket, ModelTicket)>,
}

impl Run {
    fn new(tie: TieBreak, seed: u64, scope: u16) -> Self {
        let config = Config::new(MAX_REWARD, PRIOR, tie).unwrap();
        Self {
            core: Bandit::with_scope(config, scope),
            model: Model::new(ARMS, TICKETS, MAX_REWARD, PRIOR, tie),
            rng: Rng(seed),
            scope,
            issued: Vec::new(),
        }
    }

    /// A scope this bandit does not issue under.
    fn foreign_scope(&mut self) -> u16 {
        self.scope
            .wrapping_add(1 + self.rng.below(u64::from(u16::MAX)) as u16)
    }

    fn assert_same_state(&self) {
        for arm in 0..ARMS {
            let stats = self.core.stats(arm).unwrap();
            let expected = &self.model.arms[arm];
            assert_eq!(stats.open, expected.open);
            assert_eq!(stats.generation, expected.generation);
            assert_eq!(u128::from(stats.rewarded), expected.rewarded);
            assert_eq!(u128::from(stats.reward_sum), expected.reward_sum);
            assert_eq!(stats.pending, self.model.pending(arm));
            assert!(stats.reward_sum <= stats.rewarded * u64::from(MAX_REWARD));
        }
        assert_eq!(self.core.outstanding(), self.model.live.len());
        let next = scoped(self.scope, self.model.next_serial);
        assert_eq!(self.core.next_serial(), Some(next));
        assert_eq!(self.core.scope(), self.scope);
        assert!(self.core.stats(ARMS).is_none());
    }

    fn record(&mut self, outcome: Result<Selection, Refusal>, arm: usize, reason: Reason) {
        let selection = outcome.unwrap();
        assert_eq!((selection.arm(), selection.reason), (arm, reason));
        let ticket = self.model.issue(arm);
        assert_eq!(selection.ticket.serial(), scoped(self.scope, ticket.serial));
        assert_eq!(selection.ticket.scope(), self.scope);
        assert_eq!(selection.ticket.generation(), ticket.generation);
        self.issued.push((selection.ticket, ticket));
    }

    /// Whether the model refuses this selection; if so the core refused the
    /// same way and changed nothing.
    fn refused(&self, outcome: Result<Selection, Refusal>, before: &Bandit<ARMS, TICKETS>) -> bool {
        let Some(refusal) = self.model.select_refusal() else {
            return false;
        };
        assert_eq!(outcome, Err(refusal));
        assert_eq!(&self.core, before);
        true
    }

    fn select_epsilon_greedy(&mut self) {
        let denominator = 1 + self.rng.below(8) as u32;
        let numerator = self.rng.below(u64::from(denominator) + 1) as u32;
        let epsilon = Ratio::new(numerator, denominator).unwrap();
        let draws = Draws {
            gate: self.rng.next(),
            pick: self.rng.next(),
        };
        let before = self.core.clone();
        let outcome = self.core.select_epsilon_greedy(epsilon, draws);
        if self.refused(outcome, &before) {
            return;
        }
        let (arm, reason) = if let Some(arm) = self.model.cold(draws.pick) {
            (arm, Reason::ColdStart)
        } else if explores(epsilon, draws.gate) {
            (self.model.explore(draws.pick), Reason::Explore)
        } else {
            (self.model.greedy(draws.pick), Reason::Exploit)
        };
        self.record(outcome, arm, reason);
    }

    fn select_ucb1(&mut self) {
        let pick = self.rng.next();
        let before = self.core.clone();
        let outcome = self.core.select_ucb1(pick);
        if self.refused(outcome, &before) {
            return;
        }
        if let Some(arm) = self.model.cold(pick) {
            self.record(outcome, arm, Reason::ColdStart);
            return;
        }
        let chosen = outcome.unwrap().arm();
        let scores = self.model.ucb1();
        let top = scores.iter().map(|(_, score)| *score).fold(0.0, f64::max);
        let score = scores.iter().find(|(arm, _)| *arm == chosen).unwrap().1;
        assert!(score >= top - INDEX_SLACK, "{chosen} is not near the top");
        // The tie rule is exact over the core's own published indices.
        let index = |arm: usize| before.ucb1_index_q56(arm).unwrap();
        let open = self.model.open_arms();
        let best = open.iter().map(|arm| index(*arm)).max().unwrap();
        let tied: Vec<usize> = open.into_iter().filter(|arm| index(*arm) == best).collect();
        let expected = self.model.break_tie(&tied, pick);
        self.record(outcome, expected, Reason::UpperBound);
    }

    fn redeem(&mut self) {
        if self.issued.is_empty() {
            return;
        }
        // Mostly recent tickets, which are likely live; sometimes any old one.
        let span = if self.rng.below(5) == 0 {
            self.issued.len()
        } else {
            self.issued.len().min(TICKETS + 2)
        };
        let at = self.issued.len() - 1 - self.rng.below(span as u64) as usize;
        let (mut ticket, shadow) = self.issued[at];
        let before = self.core.clone();
        if self.rng.below(6) == 0 {
            // Same slot, arm, generation and sequence number, another scope.
            ticket.serial = scoped(self.foreign_scope(), sequence_of(ticket.serial));
            assert_eq!(self.core.reward(ticket, 1), Err(Refusal::UnknownTicket));
            assert_eq!(self.core.abandon(ticket), Err(Refusal::UnknownTicket));
            assert_eq!(self.core, before);
            return;
        }
        let (outcome, expected) = if self.rng.below(5) == 0 {
            (self.core.abandon(ticket), self.model.redeem(shadow, None))
        } else {
            let reward = self.rng.below(u64::from(MAX_REWARD) + 3) as u32;
            (
                self.core.reward(ticket, reward),
                self.model.redeem(shadow, Some(reward)),
            )
        };
        assert_eq!(outcome, expected);
        if outcome.is_err() {
            assert_eq!(self.core, before);
        }
    }

    fn step(&mut self) {
        let arm = self.rng.below(ARMS as u64) as usize;
        let keep = Ratio::new(self.rng.below(5) as u32, 4).unwrap();
        match self.rng.below(100) {
            0..30 => self.select_ucb1(),
            30..55 => self.select_epsilon_greedy(),
            55..85 => self.redeem(),
            85..90 => {
                let before = self.core.clone();
                let expected = if self.model.arms[arm].open {
                    Opening::AlreadyOpen
                } else {
                    Opening::Opened
                };
                assert_eq!(self.core.open(arm), Ok(expected));
                assert_eq!(self.core == before, expected == Opening::AlreadyOpen);
                self.model.arms[arm].open = true;
            }
            90..92 => {
                let before = self.core.clone();
                let expected = if self.model.arms[arm].open {
                    Closing::Closed
                } else {
                    Closing::AlreadyClosed
                };
                assert_eq!(self.core.close(arm), Ok(expected));
                assert_eq!(self.core == before, expected == Closing::AlreadyClosed);
                self.model.arms[arm].open = false;
            }
            92 => assert_eq!(self.core.revoke_all(), self.model.revoke_all()),
            93 => {
                let floor = self.rng.below(self.model.next_serial + 2);
                let before = self.core.clone();
                let foreign = scoped(self.foreign_scope(), floor);
                assert_eq!(
                    self.core.revoke_before(foreign),
                    Err(Refusal::UnknownTicket)
                );
                assert_eq!(self.core, before);
                let revoked = self.model.revoke_before(floor);
                assert_eq!(
                    self.core.revoke_before(scoped(self.scope, floor)),
                    Ok(revoked)
                );
            }
            94..96 => assert_eq!(self.core.reset(arm), Ok(self.model.reset(arm))),
            96 => {
                let voided: usize = (0..ARMS).map(|arm| self.model.reset(arm)).sum();
                assert_eq!(self.core.reset_all(), Ok(voided));
            }
            97..99 => {
                assert_eq!(self.core.decay(arm, keep), Ok(()));
                self.model.decay(arm, keep);
            }
            _ => {
                self.core.decay_all(keep);
                (0..ARMS).for_each(|arm| self.model.decay(arm, keep));
            }
        }
        self.assert_same_state();
    }
}

#[test]
fn core_matches_the_naive_model_under_every_tie_rule() {
    let rules = [
        TieBreak::LowestIndex,
        TieBreak::HighestIndex,
        TieBreak::LeastTried,
        TieBreak::Draw,
    ];
    for (offset, tie) in rules.into_iter().enumerate() {
        for seed in 1..=8_u64 {
            // Scope zero, and non-zero scopes up to the last one.
            let scope = [0, 1, 0x4236, u16::MAX][(seed as usize + offset) % 4];
            let mut run = Run::new(tie, seed * 0x9E37_79B9 + offset as u64, scope);
            for _ in 0..6_000 {
                run.step();
            }
            assert!(run.model.next_serial > 500, "the run barely selected");
        }
    }
}
