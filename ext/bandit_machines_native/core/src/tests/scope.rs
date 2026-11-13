//! Tickets are scoped ids: only the bandit that issued one honours it.

use super::*;

fn scoped_bandit<const A: usize, const T: usize>(scope: u16) -> Bandit<A, T> {
    let mut bandit = Bandit::with_scope(Config::new(10, 5, LOW).unwrap(), scope);
    for arm in 0..A {
        assert_eq!(bandit.open(arm), Ok(Opening::Opened));
    }
    bandit
}

/// The independent review's sequence: two bandits with identical config, the
/// same calls on both, and `b` rewarded `a`'s ticket and spent its own.
#[test]
fn a_ticket_from_another_scope_is_refused_and_changes_nothing() {
    let mut a = scoped_bandit::<1, 1>(1);
    let mut b = scoped_bandit::<1, 1>(2);
    assert_eq!(a.config(), b.config());
    assert_eq!((a.scope(), b.scope()), (1, 2));
    let from_a = a.select_ucb1(0).unwrap().ticket;
    let from_b = b.select_ucb1(0).unwrap().ticket;
    assert_eq!((from_a.scope(), from_b.scope()), (1, 2));
    assert_eq!(
        (from_a.slot, from_a.arm, from_a.generation),
        (from_b.slot, from_b.arm, from_b.generation)
    );
    let (a_before, b_before) = (a.clone(), b.clone());

    assert_eq!(b.reward(from_a, 1), Err(Refusal::UnknownTicket));
    assert_eq!(b.abandon(from_a), Err(Refusal::UnknownTicket));
    assert_eq!(
        b.revoke_before(from_a.serial()),
        Err(Refusal::UnknownTicket)
    );
    assert_eq!(
        b.revoke_before(a.next_serial().unwrap()),
        Err(Refusal::UnknownTicket)
    );
    assert_eq!(b, b_before);
    assert_eq!(a.reward(from_b, 1), Err(Refusal::UnknownTicket));
    assert_eq!(a.abandon(from_b), Err(Refusal::UnknownTicket));
    assert_eq!(
        a.revoke_before(from_b.serial()),
        Err(Refusal::UnknownTicket)
    );
    assert_eq!(a, a_before);

    // A foreign ticket is unknown, not stale, even after the arm was reset.
    assert_eq!(b.reset(0), Ok(1));
    assert_eq!(b.reward(from_b, 1), Err(Refusal::StaleTicket));
    assert_eq!(b.reward(from_a, 1), Err(Refusal::UnknownTicket));
    assert_eq!(a.reward(from_a, 1), Ok(()));
}

/// Scopes are the caller's: the core reads no clock and no random source, so
/// two bandits left in one scope issue the same tickets and still alias.
#[test]
fn two_bandits_left_in_the_same_scope_still_alias() {
    let mut a = open_bandit::<1, 1>(LOW);
    let mut b = open_bandit::<1, 1>(LOW);
    let from_a = a.select_ucb1(0).unwrap().ticket;
    let from_b = b.select_ucb1(0).unwrap().ticket;
    assert_eq!(from_a, from_b);
    assert_eq!(b.reward(from_a, 1), Ok(()));
    assert_eq!(b.outstanding(), 0);
}

#[test]
fn new_is_scope_zero_and_serials_count_from_one() {
    let config = Config::new(10, 5, LOW).unwrap();
    assert_eq!(Bandit::<2, 2>::new(config), Bandit::with_scope(config, 0));
    assert_eq!(
        Bandit::<2, 2>::resumed(config, 7, 0),
        Some(Bandit::with_scope(config, 7))
    );
    assert_ne!(
        Bandit::<2, 2>::with_scope(config, 1),
        Bandit::with_scope(config, 2)
    );

    let mut plain = open_bandit::<2, 2>(LOW);
    assert_eq!((plain.scope(), plain.next_serial()), (0, Some(1)));
    let first = plain.select_ucb1(0).unwrap().ticket;
    let second = plain.select_ucb1(0).unwrap().ticket;
    assert_eq!((first.serial(), second.serial()), (1, 2));
    assert_eq!((first.scope(), plain.next_serial()), (0, Some(3)));

    let mut scoped_one = scoped_bandit::<2, 2>(7);
    assert_eq!(scoped_one.next_serial(), Some(scoped(7, 1)));
    let ticket = scoped_one.select_ucb1(0).unwrap().ticket;
    assert_eq!((ticket.serial(), ticket.scope()), (scoped(7, 1), 7));
}

#[test]
fn a_ticket_of_this_scope_that_was_never_issued_is_unknown() {
    let mut bandit = scoped_bandit::<1, 2>(3);
    let live = bandit.select_ucb1(0).unwrap().ticket;
    let before = bandit.clone();
    for serial in [scoped(3, 2), scoped(3, 0), 0] {
        let unissued = Ticket { serial, ..live };
        assert_eq!(bandit.reward(unissued, 1), Err(Refusal::UnknownTicket));
        assert_eq!(bandit.abandon(unissued), Err(Refusal::UnknownTicket));
    }
    assert_eq!(bandit, before);
    // A cut in this scope may lie past what was issued: it is not a ticket.
    assert_eq!(bandit.revoke_before(scoped(3, 9)), Ok(1));
}

#[test]
fn the_last_serial_of_a_scope_is_issued_and_then_selection_is_exhausted() {
    let mut bandit = resumed_bandit::<1, 1>(3, SEQUENCE_MAX - 1);
    let last = bandit.select_ucb1(0).unwrap().ticket;
    assert_eq!((last.serial(), last.scope()), (scoped(3, SEQUENCE_MAX), 3));
    // A call refused for another reason spends no serial.
    let before = bandit.clone();
    assert_eq!(bandit.select_ucb1(0), Err(Refusal::TicketsFull));
    assert_eq!(bandit, before);
    assert_eq!(bandit.reward(last, 1), Ok(()));
    let before = bandit.clone();
    assert_eq!(bandit.next_serial(), None);
    assert_eq!(bandit.select_ucb1(0), Err(Refusal::Exhausted));
    let draws = Draws { gate: 0, pick: 0 };
    assert_eq!(
        bandit.select_epsilon_greedy(Ratio::ONE, draws),
        Err(Refusal::Exhausted)
    );
    assert_eq!(bandit, before);
    // The spent ticket stays unknown: its serial is never reissued.
    assert_eq!(bandit.reward(last, 1), Err(Refusal::UnknownTicket));
}

/// The old idiom `revoke_before(u64::MAX)` names another scope's serial, and
/// no cut of this scope lies above its last serial: `revoke_all` is the
/// operation that needs no serial.
#[test]
fn revoke_all_revokes_every_ticket_and_keeps_what_was_learned() {
    let mut bandit = resumed_bandit::<2, 3>(3, SEQUENCE_MAX - 3);
    let spent = bandit.select_ucb1(0).unwrap().ticket;
    assert_eq!(bandit.reward(spent, 7), Ok(()));
    let before = bandit.clone();
    assert_eq!(bandit.revoke_all(), 0);
    assert_eq!(bandit, before);

    let held = [(); 2].map(|()| bandit.select_ucb1(0).unwrap().ticket);
    assert_eq!(held[1].serial(), scoped(3, SEQUENCE_MAX));
    assert_eq!(bandit.next_serial(), None);
    let before = bandit.clone();
    assert_eq!(bandit.revoke_before(u64::MAX), Err(Refusal::UnknownTicket));
    assert_eq!(bandit, before);
    // The highest cut of the scope still leaves its last ticket outstanding.
    assert_eq!(bandit.revoke_before(scoped(3, SEQUENCE_MAX)), Ok(1));
    assert_eq!(bandit.outstanding(), 1);
    assert_eq!(bandit.revoke_all(), 1);
    assert_eq!(bandit.outstanding(), 0);
    for ticket in held {
        assert_eq!(bandit.reward(ticket, 1), Err(Refusal::UnknownTicket));
    }
    for arm in 0..2 {
        let (now, was) = (bandit.stats(arm).unwrap(), before.stats(arm).unwrap());
        assert_eq!(now.pending, 0);
        assert_eq!(
            (now.open, now.generation, now.rewarded, now.reward_sum),
            (was.open, was.generation, was.rewarded, was.reward_sum)
        );
    }
    assert_eq!(bandit.stats(0).unwrap().reward_sum, 7);
}

/// A fresh bandit would count from one again, and a ticket that outlived the
/// restart would redeem the new selection filed in its slot.
#[test]
fn a_resumed_bandit_continues_its_scope_and_refuses_every_earlier_ticket() {
    let mut first = scoped_bandit::<2, 2>(5);
    let spent = first.select_ucb1(0).unwrap().ticket;
    assert_eq!(first.reward(spent, 3), Ok(()));
    let abandoned = first.select_ucb1(0).unwrap().ticket;
    assert_eq!(first.abandon(abandoned), Ok(()));
    let pending = [(); 2].map(|()| first.select_ucb1(0).unwrap().ticket);
    assert_eq!(
        (first.issued(), first.next_serial()),
        (4, Some(scoped(5, 5)))
    );

    let mut bandit = resumed_bandit::<2, 2>(first.scope(), first.issued());
    assert_eq!((bandit.scope(), bandit.issued()), (5, 4));
    // Only the sequence came back: nothing outstanding, nothing learned.
    assert_eq!((bandit.outstanding(), bandit.config()), (0, first.config()));
    assert_eq!(bandit.stats(0).unwrap().tries(), 0);
    let fresh = [(); 2].map(|()| bandit.select_ucb1(0).unwrap().ticket);
    assert_eq!(fresh.map(|t| t.serial()), [scoped(5, 5), scoped(5, 6)]);
    assert_eq!(fresh.map(|t| t.slot), pending.map(|t| t.slot));

    let before = bandit.clone();
    for ticket in [spent, abandoned, pending[0], pending[1]] {
        assert!(fresh.iter().all(|t| t.serial() != ticket.serial()));
        assert_eq!(bandit.reward(ticket, 1), Err(Refusal::UnknownTicket));
        assert_eq!(bandit.abandon(ticket), Err(Refusal::UnknownTicket));
        // An earlier serial is a cut below everything issued since.
        assert_eq!(bandit.revoke_before(ticket.serial()), Ok(0));
    }
    assert_eq!(bandit, before);
    // Generations restart too: a reset arm answers `StaleTicket` instead.
    assert_eq!(bandit.reset(pending[0].arm()), Ok(1));
    let before = bandit.clone();
    assert_eq!(bandit.reward(pending[0], 1), Err(Refusal::StaleTicket));
    assert_eq!(bandit, before);
    assert_eq!(bandit.outstanding(), 1);
}

#[test]
fn a_bandit_resumed_at_the_last_sequence_is_spent_and_one_past_it_is_none() {
    let mut spent = resumed_bandit::<1, 1>(3, SEQUENCE_MAX);
    assert_eq!((spent.scope(), spent.issued()), (3, SEQUENCE_MAX));
    assert_eq!(spent.next_serial(), None);
    let before = spent.clone();
    assert_eq!(spent.select_ucb1(0), Err(Refusal::Exhausted));
    assert_eq!(spent, before);
    let config = spent.config();
    assert_eq!(Bandit::<1, 1>::resumed(config, 3, SEQUENCE_MAX + 1), None);
    assert_eq!(Bandit::<1, 1>::resumed(config, 3, u64::MAX), None);
}
