# bandit-machines

> **AMS Robbins**, NCC-4227, *Robbins*-class exploration scout.

Engine-room flight manual: choose which of a bounded set of arms to try next,
and learn from rewards that come back late.

Allocation-free `no_std` multi-armed bandit over `ARMS` arms and at most
`TICKETS` unanswered selections. Two policies share one set of statistics:
UCB1 and epsilon-greedy. Rewards are integers in `0..=max_reward`. There is no
generator inside: every random decision consumes a `u64` the caller supplies,
so a run is replayable from its draws.

Initial, unstable API. This is a policy core, not an experiment platform: it
does not assign users, persist anything, or know what an arm means.

```rust
use bandit_machines::{
    Bandit, Closing, Config, Draws, Opening, Ratio, Reason, Refusal, TieBreak,
};

let config = Config::new(100, 50, TieBreak::LowestIndex).expect("valid range");
let mut bandit = Bandit::<4, 16>::new(config);
assert_eq!(bandit.open(0)?, Opening::Opened);
assert_eq!(bandit.open(1)?, Opening::Opened);
// Asking again is honoured and changes nothing.
assert_eq!(bandit.open(1)?, Opening::AlreadyOpen);
assert_eq!(bandit.close(3)?, Closing::AlreadyClosed);

let first = bandit.select_ucb1(0)?;
assert_eq!((first.arm(), first.reason), (0, Reason::ColdStart));
bandit.reward(first.ticket, 80)?;

let epsilon = Ratio::new(1, 10).expect("0.1");
let second = bandit.select_epsilon_greedy(epsilon, Draws { gate: u64::MAX, pick: 0 })?;
assert_eq!((second.arm(), second.reason), (1, Reason::ColdStart));

// The arm is reset while its reward is still in flight: the reward is refused.
bandit.reset(1)?;
assert_eq!(bandit.reward(second.ticket, 100), Err(Refusal::StaleTicket));

// Another bandit, another scope: it refuses a ticket it did not issue.
let mut other = Bandit::<4, 16>::with_scope(config, 7);
other.open(0)?;
let theirs = other.select_ucb1(0)?.ticket;
assert_eq!((theirs.scope(), bandit.scope()), (7, 0));
assert_eq!(bandit.reward(theirs, 1), Err(Refusal::UnknownTicket));
assert_eq!(other.reward(theirs, 1), Ok(()));
# Ok::<(), Refusal>(())
```

## Contracts

**Eligibility.** Arms start closed: the bandit explores only what the caller
opens, and selecting before that answers `NoOpenArm`. The Ruby gem starts the
same way, and its `Config` requires the tie rule just as `Config::new` does.
`open` and `close` only change whether an arm can be selected, and answer what
they did: `Opening::Opened` / `Closing::Closed` when eligibility changed,
`Opening::AlreadyOpen` / `Closing::AlreadyClosed` for an idempotent repeat that
changed nothing (Ruby: `:opened`, `:closed`, `:already_open`,
`:already_closed`). An arm past the capacity is refused with `UnknownArm`
(Ruby: `:unknown_arm`). A
closed arm keeps its statistics and still accepts the
rewards of tickets issued before it closed. Whatever decides eligibility (a
feature gate, a health check) lives with the caller.

**Tries.** An arm's tries are its counted rewards plus its outstanding tickets.
Exploration counts tries, so selecting again before a reward arrives does not
pile onto the same arm.

**Cold start.** An open arm with zero tries is *cold*. Both policies select
cold arms before doing anything else, one per call, in tie-rule order
(`Reason::ColdStart`). An arm with tickets out and no reward yet is *warming*
and competes with the configured `prior` as its mean.

**UCB1.** With `t` the tries of all open arms and `n` an arm's tries, the index
is `mean / max_reward + sqrt(2 ln t / n)`, computed in Q8.56 fixed point by the
integer routines in `math`. The logarithm is not ours: it is
`ln_floor` of the `sliderule-machines` crate at 56 fraction bits, the one integer
logarithm the fleet shares. The computed index never exceeds the true one and
falls short by less than `2^-26` (`ln` alone: less than `2^-55`). Arms whose
true indices differ by more than that are always ordered correctly; closer ones
are ordered by their computed integers, identically on every target.
`ucb1_index_q56` publishes the integer being compared.

**Epsilon-greedy.** Explore when `floor(gate * denominator / 2^64) < numerator`,
picking open arm number `floor(pick * open / 2^64)`. Otherwise exploit the
highest mean, compared exactly by cross-multiplication.

**Ties.** Exactly equal scores are settled by `TieBreak`: lowest index, highest
index, fewest tries then lowest index, or a uniform draw from `pick`.

**Delayed rewards.** Every selection issues a `Ticket` naming the arm, the
arm's generation and a serial. A ticket is redeemed once, by `reward` or
`abandon`. `reset` bumps the generation and voids the arm's tickets, so a late
reward answers `StaleTicket`; a spent, revoked or foreign ticket answers
`UnknownTicket`. `revoke_before(serial)` abandons everything issued before a
serial, which is how a caller with its own clock expires selections that will
never report. `revoke_all()` abandons every outstanding ticket and cannot
refuse; no serial does that (`u64::MAX` is another scope's, and the last
serial of a scope can itself be outstanding). Unlike `reset_all` it keeps
every arm's statistics and generation. `next_serial` is the serial the next ticket will carry, `None`
once the scope is spent. Tickets are not a wire format.

**Scopes.** A ticket is only meaningful to the bandit that issued it. Serials
come from a `periscope_machines::Issuer`: the bandit's scope in the top 16
bits, a sequence counted from one below, never reissued.
`Bandit::with_scope(config, scope)` chooses the scope and `new(config)` is
scope zero; `Bandit::scope` and `Ticket::scope` read it. The scope is identity,
not policy, so it is not in `Config`: replicas share a configuration and must
not share a scope. `reward` and `abandon` answer `UnknownTicket` for a ticket
the bandit did not issue, before looking at any arm or slot, and
`revoke_before` answers it for a serial of another scope. Give each bandit its
own scope: the core reads no clock and no random source, so it cannot pick one,
and two bandits left in the same scope issue the same tickets.

**Restarts.** A fresh bandit counts from one again, so a ticket that outlived a
restart could redeem a new selection. Save `Bandit::issued` and rebuild with
`Bandit::resumed(config, scope, issued)`, or give each incarnation a new scope.
`resumed` restores only the serial sequence: every arm is closed with nothing
observed, no ticket is outstanding, every earlier ticket is refused, and
reopening arms and restoring what they learned is the caller's job. It is
`None` when `issued` is beyond `periscope_machines::SEQUENCE_MAX`.

**Nonstationarity.** Nothing ages on its own. `reset` / `reset_all` forget an
arm and fence its late rewards. `decay` / `decay_all` discount history by a
`Ratio`: count becomes `ceil(count * keep)`, sum `floor(sum * keep)`, tickets
and generations untouched, so a late reward still lands at full weight.

**Refusals.** Every refusal leaves the bandit unchanged. `TicketsFull` when all
slots are out, `Exhausted` when the scope's 2^48 - 1 serials or an arm's
generations run out,
`Saturated` when an arm's `u64` count or sum would overflow (decay, then retry
the same ticket), `RewardOutOfRange` above `max_reward`.

The regret bound of UCB1 assumes stationary rewards and immediate feedback.
Delays, resets and decay are operational tools; they void that proof, not the
arithmetic.
