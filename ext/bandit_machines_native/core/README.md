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
never report. Tickets are scoped to one bandit value and are not a wire format.

**Nonstationarity.** Nothing ages on its own. `reset` / `reset_all` forget an
arm and fence its late rewards. `decay` / `decay_all` discount history by a
`Ratio`: count becomes `ceil(count * keep)`, sum `floor(sum * keep)`, tickets
and generations untouched, so a late reward still lands at full weight.

**Refusals.** Every refusal leaves the bandit unchanged. `TicketsFull` when all
slots are out, `Exhausted` when serials or an arm's generations run out,
`Saturated` when an arm's `u64` count or sum would overflow (decay, then retry
the same ticket), `RewardOutOfRange` above `max_reward`.

The regret bound of UCB1 assumes stationary rewards and immediate feedback.
Delays, resets and decay are operational tools; they void that proof, not the
arithmetic.
