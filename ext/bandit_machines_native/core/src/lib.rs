#![no_std]
#![doc = include_str!("../README.md")]

mod bandit;
pub mod math;
mod pick;

pub use bandit::Bandit;

/// How arms that score exactly the same are told apart.
///
/// Applies to cold-start sweeps, UCB1 indices and greedy means alike. Scores
/// are compared as integers, so "the same" is exact and platform independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieBreak {
    /// The tied arm with the lowest index.
    LowestIndex,
    /// The tied arm with the highest index.
    HighestIndex,
    /// The tied arm with the fewest tries (rewards plus outstanding tickets),
    /// then the lowest index.
    LeastTried,
    /// Uniform among the tied arms, using the caller's `pick` draw.
    Draw,
}

/// Reward range, warming prior and tie rule shared by every arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    max_reward: u32,
    prior: u32,
    tie_break: TieBreak,
}

impl Config {
    /// Rewards are integers in `0..=max_reward`; `prior` is the mean assumed
    /// for an arm that has outstanding tickets but no reward yet.
    ///
    /// `None` when `max_reward` is zero or `prior` exceeds it.
    #[must_use]
    pub const fn new(max_reward: u32, prior: u32, tie_break: TieBreak) -> Option<Self> {
        if max_reward == 0 || prior > max_reward {
            return None;
        }
        Some(Self {
            max_reward,
            prior,
            tie_break,
        })
    }

    /// Largest reward one ticket may report.
    #[must_use]
    pub const fn max_reward(&self) -> u32 {
        self.max_reward
    }

    /// Mean assumed for an arm with tickets out and no reward yet.
    #[must_use]
    pub const fn prior(&self) -> u32 {
        self.prior
    }

    /// Rule separating exactly tied arms.
    #[must_use]
    pub const fn tie_break(&self) -> TieBreak {
        self.tie_break
    }
}

/// A fraction in `0..=1`, written `numerator / denominator`.
///
/// Used as the exploration probability of epsilon-greedy and as the retention
/// factor of a decay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    numerator: u32,
    denominator: u32,
}

impl Ratio {
    /// Zero: never explore, or forget everything.
    pub const ZERO: Self = Self::raw(0, 1);

    /// One: always explore, or keep everything.
    pub const ONE: Self = Self::raw(1, 1);

    /// `None` when `denominator` is zero or `numerator` exceeds it.
    #[must_use]
    pub const fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if denominator == 0 || numerator > denominator {
            return None;
        }
        Some(Self::raw(numerator, denominator))
    }

    /// Top of the fraction.
    #[must_use]
    pub const fn numerator(&self) -> u32 {
        self.numerator
    }

    /// Bottom of the fraction; never zero.
    #[must_use]
    pub const fn denominator(&self) -> u32 {
        self.denominator
    }

    const fn raw(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }
}

/// The two uniform `u64` draws one epsilon-greedy selection may consume.
///
/// The bandit holds no generator. `gate` decides explore versus exploit;
/// `pick` chooses the explored arm, or breaks a tie under [`TieBreak::Draw`].
/// A draw `d` is mapped onto `0..n` as `floor(d * n / 2^64)`, which is uniform
/// to within `n / 2^64` when `d` is uniform over all of `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draws {
    /// Explore when `floor(gate * denominator / 2^64) < numerator`.
    pub gate: u64,
    /// Index draw for the explored arm or for a drawn tie.
    pub pick: u64,
}

/// Why a selection landed on its arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The arm had no rewards and no outstanding tickets.
    ColdStart,
    /// Epsilon-greedy explored: uniform over the open arms.
    Explore,
    /// Epsilon-greedy exploited: the highest mean.
    Exploit,
    /// UCB1: the highest upper confidence index.
    UpperBound,
}

/// Why a call was refused. A refused call changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The arm index is not below the compiled arm capacity.
    UnknownArm,
    /// No arm is open for selection.
    NoOpenArm,
    /// Every ticket slot holds an outstanding selection.
    TicketsFull,
    /// Ticket serials or an arm's generations are used up.
    Exhausted,
    /// The ticket's arm was reset after the selection was made.
    StaleTicket,
    /// The ticket was already rewarded, abandoned or revoked, or is not ours.
    UnknownTicket,
    /// The reward is above the configured maximum.
    RewardOutOfRange,
    /// The arm's reward count or reward sum would overflow; decay first.
    Saturated,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::UnknownArm => "the arm index is not below the arm capacity",
            Self::NoOpenArm => "no arm is open for selection",
            Self::TicketsFull => "every ticket slot holds an outstanding selection",
            Self::Exhausted => "ticket serials or the arm's generations are used up",
            Self::StaleTicket => "the ticket's arm was reset after the selection",
            Self::UnknownTicket => "the ticket is spent or belongs to another bandit",
            Self::RewardOutOfRange => "the reward is above the configured maximum",
            Self::Saturated => "the arm's reward count or sum would overflow",
        })
    }
}

impl core::error::Error for Refusal {}

/// What an honoured [`Bandit::open`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// The arm was closed and is now eligible for selection.
    Opened,
    /// The arm was already open: an idempotent repeat that changed nothing.
    AlreadyOpen,
}

/// What an honoured [`Bandit::close`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// The arm was open and is no longer eligible for selection.
    Closed,
    /// The arm was already closed: an idempotent repeat that changed nothing.
    AlreadyClosed,
}

/// Proof of one selection, to be redeemed by exactly one reward or abandon.
///
/// Scoped to one bandit value. It names the arm's generation at selection
/// time, so a reward that arrives after the arm was reset is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    pub(crate) slot: usize,
    pub(crate) arm: usize,
    pub(crate) generation: u64,
    pub(crate) serial: u64,
}

impl Ticket {
    /// Arm the selection landed on.
    #[must_use]
    pub const fn arm(&self) -> usize {
        self.arm
    }

    /// Generation of that arm when it was selected.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Position in the bandit's selection order; strictly increasing.
    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }
}

/// A selected arm, its ticket and the rule that picked it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Redeem with [`Bandit::reward`] or [`Bandit::abandon`].
    pub ticket: Ticket,
    /// Why this arm.
    pub reason: Reason,
}

impl Selection {
    /// Arm the selection landed on.
    #[must_use]
    pub const fn arm(&self) -> usize {
        self.ticket.arm
    }
}

/// Snapshot of one arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmStats {
    /// Whether the arm is eligible for selection.
    pub open: bool,
    /// Bumped by every reset; tickets from older generations are stale.
    pub generation: u64,
    /// Rewards counted, after any decay.
    pub rewarded: u64,
    /// Sum of those rewards, after any decay.
    pub reward_sum: u64,
    /// Outstanding tickets for this arm.
    pub pending: u64,
}

impl ArmStats {
    /// Rewards counted plus outstanding tickets: what exploration counts.
    #[must_use]
    pub const fn tries(&self) -> u64 {
        self.rewarded.saturating_add(self.pending)
    }
}

#[cfg(test)]
mod tests;
