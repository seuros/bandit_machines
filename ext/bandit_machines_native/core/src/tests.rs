#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // Failed test assumptions should panic; the reference model is floating point.

extern crate std;

mod model;
mod reference;

use super::*;

const LOW: TieBreak = TieBreak::LowestIndex;

fn open_bandit<const A: usize, const T: usize>(tie: TieBreak) -> Bandit<A, T> {
    let mut bandit = Bandit::new(Config::new(10, 5, tie).unwrap());
    for arm in 0..A {
        assert_eq!(bandit.open(arm), Ok(Opening::Opened));
    }
    bandit
}

const fn draws(gate: u64, pick: u64) -> Draws {
    Draws { gate, pick }
}

/// Pull `arm` once through the cold sweep or policy and reward it.
fn feed<const A: usize, const T: usize>(bandit: &mut Bandit<A, T>, rewards: &[u32]) {
    for reward in rewards {
        let selection = bandit.select_ucb1(0).unwrap();
        bandit.reward(selection.ticket, *reward).unwrap();
    }
}

#[test]
fn config_and_ratio_reject_out_of_range_values() {
    assert!(Config::new(0, 0, LOW).is_none());
    assert!(Config::new(10, 11, LOW).is_none());
    let config = Config::new(u32::MAX, u32::MAX, TieBreak::Draw).unwrap();
    assert_eq!(config.max_reward(), u32::MAX);
    assert_eq!(config.prior(), u32::MAX);
    assert_eq!(config.tie_break(), TieBreak::Draw);
    assert!(Ratio::new(1, 0).is_none());
    assert!(Ratio::new(3, 2).is_none());
    let ratio = Ratio::new(1, 4).unwrap();
    assert_eq!((ratio.numerator(), ratio.denominator()), (1, 4));
    assert_eq!(
        Ratio::new(0, 9).unwrap().numerator(),
        Ratio::ZERO.numerator()
    );
    assert_eq!(
        Ratio::new(u32::MAX, u32::MAX).unwrap().numerator(),
        u32::MAX
    );
}

#[test]
fn repeated_open_and_close_are_honoured_and_change_nothing() {
    let mut bandit = open_bandit::<3, 4>(LOW);
    feed(&mut bandit, &[4, 6, 2]);
    let pending = bandit.select_ucb1(0).unwrap();
    bandit.close(2).unwrap();

    let before = bandit.clone();
    assert_eq!(bandit.open(0), Ok(Opening::AlreadyOpen));
    assert_eq!(bandit, before);
    assert_eq!(bandit.close(2), Ok(Closing::AlreadyClosed));
    assert_eq!(bandit, before);
    // A refused call changes nothing either.
    assert_eq!(bandit.open(3), Err(Refusal::UnknownArm));
    assert_eq!(bandit.close(3), Err(Refusal::UnknownArm));
    assert_eq!(bandit, before);

    // The first call is the one that changes eligibility, and only that.
    assert_eq!(bandit.close(0), Ok(Closing::Closed));
    assert_ne!(bandit, before);
    assert_eq!(bandit.open(0), Ok(Opening::Opened));
    assert_eq!(bandit, before);
    assert_eq!(bandit.reward(pending.ticket, 1), Ok(()));
}

#[test]
fn selection_needs_an_open_arm_and_a_free_ticket() {
    let mut closed = Bandit::<2, 2>::new(Config::new(1, 0, LOW).unwrap());
    assert_eq!(closed.select_ucb1(0), Err(Refusal::NoOpenArm));
    assert_eq!(
        closed.select_epsilon_greedy(Ratio::ONE, draws(0, 0)),
        Err(Refusal::NoOpenArm)
    );
    assert_eq!(closed.open(2), Err(Refusal::UnknownArm));
    assert_eq!(closed.close(2), Err(Refusal::UnknownArm));
    assert_eq!(closed.open(1), Ok(Opening::Opened));
    assert_eq!(closed.open(1), Ok(Opening::AlreadyOpen));
    assert_eq!(closed.select_ucb1(0).unwrap().arm(), 1);
    assert_eq!(closed.close(1), Ok(Closing::Closed));
    assert_eq!(closed.close(1), Ok(Closing::AlreadyClosed));

    let mut full = open_bandit::<2, 2>(LOW);
    let first = full.select_ucb1(0).unwrap();
    full.select_ucb1(0).unwrap();
    let before = full.clone();
    assert_eq!(full.select_ucb1(0), Err(Refusal::TicketsFull));
    assert_eq!(
        full.select_epsilon_greedy(Ratio::ZERO, draws(0, 0)),
        Err(Refusal::TicketsFull)
    );
    assert_eq!(full, before);
    assert_eq!(full.outstanding(), 2);
    full.abandon(first.ticket).unwrap();
    assert!(full.select_ucb1(0).is_ok());
    assert_eq!(
        Bandit::<0, 0>::new(Config::new(1, 0, LOW).unwrap()).select_ucb1(0),
        Err(Refusal::NoOpenArm)
    );
    assert_eq!(
        open_bandit::<1, 0>(LOW).select_ucb1(0),
        Err(Refusal::TicketsFull)
    );
}

#[test]
fn cold_arms_are_swept_first_under_each_tie_rule() {
    let order = |tie, pick| {
        let mut bandit = open_bandit::<3, 3>(tie);
        [(); 3].map(|()| {
            let selection = bandit.select_ucb1(pick).unwrap();
            assert_eq!(selection.reason, Reason::ColdStart);
            selection.arm()
        })
    };
    assert_eq!(order(TieBreak::LowestIndex, u64::MAX), [0, 1, 2]);
    assert_eq!(order(TieBreak::HighestIndex, 0), [2, 1, 0]);
    assert_eq!(order(TieBreak::LeastTried, u64::MAX), [0, 1, 2]);
    assert_eq!(order(TieBreak::Draw, u64::MAX), [2, 1, 0]);
    assert_eq!(order(TieBreak::Draw, 0), [0, 1, 2]);
    assert_eq!(order(TieBreak::Draw, 1 << 63), [1, 2, 0]);

    // Epsilon-greedy sweeps too, even when told to exploit, and a closed cold
    // arm is not part of the sweep.
    let mut greedy = open_bandit::<3, 3>(LOW);
    greedy.close(0).unwrap();
    let first = greedy
        .select_epsilon_greedy(Ratio::ZERO, draws(0, 0))
        .unwrap();
    assert_eq!((first.arm(), first.reason), (1, Reason::ColdStart));
    let second = greedy
        .select_epsilon_greedy(Ratio::ONE, draws(0, 0))
        .unwrap();
    assert_eq!((second.arm(), second.reason), (2, Reason::ColdStart));
}

#[test]
fn ucb1_prefers_the_higher_index_and_explores_the_neglected_arm() {
    let mut bandit = open_bandit::<2, 4>(LOW);
    feed(&mut bandit, &[0, 10]);
    let low = bandit.ucb1_index_q56(0).unwrap();
    let high = bandit.ucb1_index_q56(1).unwrap();
    assert_eq!(high - low, math::ONE);
    assert_eq!(low, math::bonus_q56(2, 1));
    let selection = bandit.select_ucb1(0).unwrap();
    assert_eq!((selection.arm(), selection.reason), (1, Reason::UpperBound));
    bandit.reward(selection.ticket, 10).unwrap();
    // Arm 1 keeps winning until arm 0's bonus outgrows a full reward range.
    let mut pulls = 0;
    loop {
        let selection = bandit.select_ucb1(0).unwrap();
        if selection.arm() == 0 {
            break;
        }
        bandit.reward(selection.ticket, 10).unwrap();
        pulls += 1;
        assert!(pulls < 100, "arm 0 was never explored");
    }
    assert!(pulls > 0);
    assert!(bandit.ucb1_index_q56(2).is_none());
    bandit.close(0).unwrap();
    assert!(bandit.ucb1_index_q56(0).is_none());
}

#[test]
fn outstanding_tickets_count_as_tries_and_compete_at_the_prior() {
    let mut bandit = open_bandit::<2, 4>(LOW);
    let first = bandit.select_ucb1(0).unwrap();
    let second = bandit.select_ucb1(0).unwrap();
    assert_eq!((first.arm(), second.arm()), (0, 1));
    // Nothing is rewarded yet: both arms are warming at prior 5/10.
    let index = bandit.ucb1_index_q56(0).unwrap();
    assert_eq!(index, math::ONE / 2 + math::bonus_q56(2, 1));
    assert_eq!(bandit.ucb1_index_q56(1), Some(index));
    assert_eq!(bandit.stats(0).unwrap().tries(), 1);
    // A reward below the prior hands greedy to the still-warming arm.
    bandit.reward(first.ticket, 4).unwrap();
    let exploit = bandit
        .select_epsilon_greedy(Ratio::ZERO, draws(0, 0))
        .unwrap();
    assert_eq!((exploit.arm(), exploit.reason), (1, Reason::Exploit));
    // An abandoned selection never happened: arm 1 is cold again.
    bandit.abandon(exploit.ticket).unwrap();
    bandit.abandon(second.ticket).unwrap();
    assert_eq!(bandit.select_ucb1(0).unwrap().reason, Reason::ColdStart);
}

#[test]
fn epsilon_gate_and_pick_map_draws_as_documented() {
    let mut bandit = open_bandit::<4, 8>(LOW);
    feed(&mut bandit, &[1, 9, 9, 3]);
    let quarter = Ratio::new(1, 4).unwrap();
    let mut select = |gate, pick| {
        let selection = bandit
            .select_epsilon_greedy(quarter, draws(gate, pick))
            .unwrap();
        bandit.abandon(selection.ticket).unwrap();
        (selection.arm(), selection.reason)
    };
    // gate < 2^62 explores; pick spreads uniformly over the four open arms.
    assert_eq!(select(0, 0), (0, Reason::Explore));
    assert_eq!(select((1 << 62) - 1, u64::MAX), (3, Reason::Explore));
    assert_eq!(select(5, 1 << 63), (2, Reason::Explore));
    // gate >= 2^62 exploits; arms 1 and 2 tie exactly, lowest index wins.
    assert_eq!(select(1 << 62, u64::MAX), (1, Reason::Exploit));
    assert_eq!(select(u64::MAX, 0), (1, Reason::Exploit));
}

#[test]
fn exact_ties_follow_the_configured_rule() {
    let winner = |tie, pick| {
        let mut bandit = open_bandit::<3, 4>(tie);
        // Arms 1 and 2 tie on 6/1 == 12/2 by exact cross-multiplication.
        for _ in 0..3 {
            let selection = bandit.select_epsilon_greedy(Ratio::ZERO, draws(0, 0));
            let ticket = selection.unwrap().ticket;
            bandit.reward(ticket, [2, 6, 6][ticket.arm()]).unwrap();
        }
        bandit.close(0).unwrap();
        bandit.close(1).unwrap();
        let again = bandit.select_epsilon_greedy(Ratio::ZERO, draws(0, 0));
        bandit.reward(again.unwrap().ticket, 6).unwrap();
        bandit.open(0).unwrap();
        bandit.open(1).unwrap();
        bandit
            .select_epsilon_greedy(Ratio::ZERO, draws(u64::MAX, pick))
            .unwrap()
            .arm()
    };
    assert_eq!(winner(TieBreak::LowestIndex, u64::MAX), 1);
    assert_eq!(winner(TieBreak::HighestIndex, 0), 2);
    assert_eq!(winner(TieBreak::LeastTried, u64::MAX), 1);
    assert_eq!(winner(TieBreak::Draw, 0), 1);
    assert_eq!(winner(TieBreak::Draw, u64::MAX), 2);
}

#[test]
fn late_rewards_for_a_reset_arm_are_refused() {
    let mut bandit = open_bandit::<2, 4>(LOW);
    let stale = bandit.select_ucb1(0).unwrap().ticket;
    let other = bandit.select_ucb1(0).unwrap().ticket;
    assert_eq!(bandit.reset(0), Ok(1));
    assert_eq!(bandit.reset(2), Err(Refusal::UnknownArm));
    let before = bandit.clone();
    assert_eq!(bandit.reward(stale, 10), Err(Refusal::StaleTicket));
    assert_eq!(bandit.abandon(stale), Err(Refusal::StaleTicket));
    assert_eq!(bandit, before);
    assert_eq!(bandit.stats(0).unwrap().generation, 1);
    assert_eq!(bandit.outstanding(), 1);
    // The reset arm is cold again and its replacement ticket is distinct.
    let fresh = bandit.select_ucb1(0).unwrap();
    assert_eq!((fresh.arm(), fresh.reason), (0, Reason::ColdStart));
    assert_eq!(fresh.ticket.generation(), 1);
    assert_eq!(bandit.reward(stale, 10), Err(Refusal::StaleTicket));
    assert_eq!(bandit.reward(fresh.ticket, 10), Ok(()));
    // A ticket is spent exactly once, and bad rewards leave it redeemable.
    assert_eq!(bandit.reward(other, 11), Err(Refusal::RewardOutOfRange));
    assert_eq!(bandit.reward(other, 3), Ok(()));
    assert_eq!(bandit.reward(other, 3), Err(Refusal::UnknownTicket));
    assert_eq!(bandit.abandon(other), Err(Refusal::UnknownTicket));
    // A ticket from a differently shaped bandit names no arm here.
    let foreign = open_bandit::<9, 9>(TieBreak::HighestIndex).select_ucb1(0);
    assert_eq!(
        bandit.reward(foreign.unwrap().ticket, 1),
        Err(Refusal::UnknownTicket)
    );
}

#[test]
fn closing_keeps_statistics_and_accepts_the_late_reward() {
    let mut bandit = open_bandit::<2, 2>(LOW);
    let ticket = bandit.select_ucb1(0).unwrap().ticket;
    bandit.close(0).unwrap();
    assert_eq!(bandit.reward(ticket, 7), Ok(()));
    let stats = bandit.stats(0).unwrap();
    assert_eq!(
        (stats.open, stats.rewarded, stats.reward_sum),
        (false, 1, 7)
    );
    assert_eq!(bandit.select_ucb1(0).unwrap().arm(), 1);
}

#[test]
fn revoke_and_reset_all_void_tickets_and_free_their_slots() {
    let mut bandit = open_bandit::<2, 3>(LOW);
    let tickets = [(); 3].map(|()| bandit.select_ucb1(0).unwrap().ticket);
    assert_eq!(tickets.map(|ticket| ticket.serial()), [0, 1, 2]);
    assert_eq!(bandit.revoke_before(0), 0);
    assert_eq!(bandit.revoke_before(2), 2);
    assert_eq!(bandit.reward(tickets[0], 1), Err(Refusal::UnknownTicket));
    assert_eq!(bandit.outstanding(), 1);
    assert_eq!(bandit.next_serial(), 3);
    assert_eq!(bandit.reward(tickets[2], 1), Ok(()));
    let pending = bandit.select_ucb1(0).unwrap().ticket;
    assert_eq!(bandit.reset_all(), Ok(1));
    assert_eq!(bandit.reward(pending, 1), Err(Refusal::StaleTicket));
    assert_eq!(bandit.outstanding(), 0);
    for arm in 0..2 {
        let stats = bandit.stats(arm).unwrap();
        assert_eq!((stats.generation, stats.tries(), stats.open), (1, 0, true));
    }
    assert_eq!(bandit.revoke_before(u64::MAX), 0);
}

#[test]
fn decay_discounts_history_without_touching_tickets() {
    let mut bandit = open_bandit::<2, 4>(LOW);
    feed(&mut bandit, &[9, 2, 9, 9]);
    bandit.close(1).unwrap();
    let late = bandit.select_ucb1(0).unwrap().ticket;
    bandit.open(1).unwrap();
    assert_eq!(bandit.stats(0).unwrap().reward_sum, 27);
    assert_eq!(bandit.decay(0, Ratio::new(1, 2).unwrap()), Ok(()));
    let stats = bandit.stats(0).unwrap();
    // ceil(3 / 2) rewards and floor(27 / 2) reward: the mean never exceeds max.
    assert_eq!(
        (stats.rewarded, stats.reward_sum, stats.generation),
        (2, 13, 0)
    );
    assert_eq!(bandit.reward(late, 10), Ok(()));
    assert_eq!(bandit.stats(0).unwrap().reward_sum, 23);
    assert_eq!(bandit.decay(2, Ratio::ONE), Err(Refusal::UnknownArm));
    let before = bandit.clone();
    bandit.decay_all(Ratio::ONE);
    assert_eq!(bandit, before);
    bandit.decay_all(Ratio::ZERO);
    assert_eq!(bandit.stats(0).unwrap().tries(), 0);
    assert_eq!(bandit.stats(1).unwrap().tries(), 0);
    assert_eq!(bandit.select_ucb1(0).unwrap().reason, Reason::ColdStart);
}

#[test]
fn u64_limits_refuse_without_mutating() {
    let mut bandit = open_bandit::<2, 2>(LOW);
    let ticket = bandit.select_ucb1(0).unwrap().ticket;
    bandit.force(0, 0, u64::MAX, 0);
    let before = bandit.clone();
    assert_eq!(bandit.reward(ticket, 1), Err(Refusal::Saturated));
    assert_eq!(bandit, before);
    bandit.force(0, 0, 5, u64::MAX);
    assert_eq!(bandit.reward(ticket, 1), Err(Refusal::Saturated));
    assert_eq!(bandit.reward(ticket, 0), Ok(()));
    // Saturated counters still select, with indices that stay in range.
    bandit.force(0, 0, u64::MAX, u64::MAX);
    bandit.force(1, 0, u64::MAX, 0);
    assert_eq!(bandit.ucb1_index_q56(0), Some(math::ONE / 10));
    let selection = bandit.select_ucb1(0).unwrap();
    assert_eq!(selection.arm(), 0);
    assert_eq!(bandit.stats(0).unwrap().tries(), u64::MAX);
    bandit.decay_all(Ratio::new(u32::MAX - 1, u32::MAX).unwrap());
    assert_eq!(bandit.reward(selection.ticket, 10), Ok(()));

    // Generations run out: the reset is refused and nothing is voided.
    bandit.force(1, u64::MAX, 3, 3);
    let held = bandit.select_epsilon_greedy(Ratio::ZERO, draws(0, 0));
    let before = bandit.clone();
    assert_eq!(bandit.reset(1), Err(Refusal::Exhausted));
    assert_eq!(bandit.reset_all(), Err(Refusal::Exhausted));
    assert_eq!(bandit, before);
    assert_eq!(bandit.reset(0), Ok(usize::from(held.unwrap().arm() == 0)));

    // Serials run out: selection is refused for good.
    bandit.next_serial = u64::MAX - 1;
    assert_eq!(bandit.select_ucb1(0).unwrap().ticket.serial(), u64::MAX - 1);
    let before = bandit.clone();
    assert_eq!(bandit.select_ucb1(0), Err(Refusal::Exhausted));
    assert_eq!(
        bandit.select_epsilon_greedy(Ratio::ONE, draws(0, 0)),
        Err(Refusal::Exhausted)
    );
    assert_eq!(bandit, before);
}
