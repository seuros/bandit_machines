# frozen_string_literal: true
require_relative 'support'

class BanditTest < Minitest::Test
  include BanditSupport

  def test_capacity_constants_and_configuration
    bandit = build(tie_break: :draw)
    assert_equal [16, 64], [BanditMachines::Bandit::ARMS, BanditMachines::Bandit::TICKETS]
    assert_equal BanditMachines::Config.new(max_reward: 10, prior: 5, tie_break: :draw), bandit.config
    assert_equal %i[max_reward prior tie_break], BanditMachines::Config.members
    assert_equal %i[lowest_index highest_index least_tried],
                 (%i[lowest_index highest_index least_tried].map { |rule| build(tie_break: rule).config.tie_break })
    assert_equal [true, true, true, false], 4.times.map { |arm| bandit.stats(arm).open }
    assert_predicate bandit.config, :frozen?
  end

  def test_values_mirror_the_core
    assert_equal %i[numerator denominator], BanditMachines::Ratio.members
    assert_equal %i[gate pick], BanditMachines::Draws.members
    assert_equal %i[open generation rewarded reward_sum pending], BanditMachines::ArmStats.members
    assert_equal [ratio(0, 1), ratio(1, 1)], [BanditMachines::Ratio::ZERO, BanditMachines::Ratio::ONE]
    assert_equal [1, 4], [ratio(1, 4).numerator, ratio(1, 4).denominator]
    assert_equal [MAX, 0], [draws(MAX, 0).gate, draws(MAX, 0).pick]
    assert_raises(RangeError) { ratio(2, 1) }
    assert_raises(RangeError) { ratio(0, 0) }
    assert_raises(RangeError) { ratio(-1, 1) }
    assert_raises(RangeError) { ratio(1, 2**32) }
    assert_raises(TypeError) { ratio(0.5, 1) }
    assert_raises(ArgumentError) { BanditMachines::Ratio.new(1) }
    assert_raises(RangeError) { draws(2**64, 0) }
    assert_raises(RangeError) { draws(0, -1) }
    assert_raises(TypeError) { draws(nil, 0) }
    assert_raises(RangeError) { config(max_reward: 0, prior: 0) }
    assert_raises(RangeError) { config(max_reward: 4, prior: 5) }
    assert_raises(RangeError) { config(max_reward: 2**32) }
    assert_raises(TypeError) { config(tie_break: 'draw') }
    assert_raises(TypeError) { config(tie_break: nil) }
    assert_raises(RangeError) { config(tie_break: :coin) }
    assert_raises(ArgumentError) { BanditMachines::Config.new(10, 5) }
    assert_raises(ArgumentError) { BanditMachines::Config.new(arms: 3, max_reward: 10, prior: 5, tie_break: :draw) }
    assert_raises(NoMethodError) { ratio(1, 2).__validate }
    assert_equal MAX, BanditMachines::ArmStats.new(true, 0, MAX, 0, 1).tries
  end

  def test_fixed_point_math_is_the_cores
    math = BanditMachines::Math
    assert_equal [56, 2**56], [math::FRACTION_BITS, math::ONE]
    assert_equal [0, 0, 3 * 2**56], [math.log2_q56(0), math.log2_q56(1), math.log2_q56(8)]
    assert_equal 0, math.ln_q56(1)
    assert_in_delta 1.0, math.ln_q56(3).fdiv(2**56) / ::Math.log(3), 1e-12
    assert_in_delta ::Math.sqrt(2 * ::Math.log(10) / 4), math.bonus_q56(10, 4).fdiv(2**56), 1e-6
    assert_equal math.bonus_q56(10, 1), math.bonus_q56(10, 0)
    assert_equal [2**55, 2**56, 2**56], [math.mean_q56(5, 1, 10), math.mean_q56(99, 1, 10), math.mean_q56(1, 0, 0)]
    assert_raises(TypeError) { math.log2_q56(1.0) }
    assert_raises(RangeError) { math.ln_q56(-1) }
    assert_raises(RangeError) { math.mean_q56(1, 1, 2**32) }
    bandit = build(arms: 2)
    feed(bandit, [0, 10])
    assert_equal math.mean_q56(10, 1, 10) + math.bonus_q56(2, 1), bandit.ucb1_index_q56(1)
  end

  def test_a_new_bandit_has_every_arm_closed
    bandit = BanditMachines::Bandit.new(config)
    assert_equal [false] * 16, 16.times.map { |arm| bandit.stats(arm).open }
    assert_equal :no_open_arm, bandit.select_ucb1(0)
    assert_equal :no_open_arm, bandit.select_epsilon_greedy(ratio(1, 1), draws(0, 0))
    assert_equal [0, 1, 0, 0], [bandit.outstanding, bandit.next_serial, bandit.scope, bandit.issued]
    assert_equal [0, 0, 0], bandit.stats(0).to_h.values_at(:generation, :pending, :rewarded)
    16.times { |arm| assert_nil bandit.ucb1_index_q56(arm) }
    assert_equal :already_closed, bandit.close(7)
    assert_equal :opened, bandit.open(7)
    chosen = bandit.select_ucb1(MAX)
    assert_equal [7, :cold_start, 1], [chosen.arm, chosen.reason, chosen.ticket.serial]
    assert_equal [7], 16.times.select { |arm| bandit.stats(arm).open }
  end

  def test_an_index_that_names_no_arm_is_answered_not_raised
    bandit = build
    [16, 17, MAX].each do |arm|
      assert_equal :unknown_arm, bandit.open(arm)
      assert_equal :unknown_arm, bandit.close(arm)
      assert_equal :unknown_arm, bandit.reset(arm)
      assert_equal :unknown_arm, bandit.decay(arm, ratio(1, 2))
      assert_nil bandit.stats(arm)
      assert_nil bandit.ucb1_index_q56(arm)
    end
    assert_equal [0, 1], [bandit.outstanding, bandit.next_serial]
    assert_equal [0] * 16, 16.times.map { bandit.stats(it).generation }
  end

  def test_ucb1_sweeps_cold_arms_then_follows_the_index
    bandit = build(arms: 2)
    first = bandit.select_ucb1(0)
    assert_instance_of BanditMachines::Selection, first
    assert_instance_of BanditMachines::Ticket, first.ticket
    assert_equal [0, :cold_start, 0, 1, 0], [first.arm, first.reason, first.ticket.arm, first.ticket.serial,
                                             first.ticket.generation]
    assert_predicate first, :frozen?
    assert_equal %i[ticket reason], BanditMachines::Selection.members
    assert_raises(ArgumentError) { bandit.select_ucb1 }
    assert_equal :rewarded, bandit.reward(first.ticket, 0)
    second = bandit.select_ucb1(MAX)
    assert_equal [1, :cold_start], [second.arm, second.reason]
    assert_equal :rewarded, bandit.reward(second.ticket, 10)
    assert_equal 2**56, bandit.ucb1_index_q56(1) - bandit.ucb1_index_q56(0)
    assert_nil bandit.ucb1_index_q56(2)
    third = bandit.select_ucb1(0)
    assert_equal [1, :upper_bound, 3], [third.arm, third.reason, third.ticket.serial]
    assert_equal [1, 4], [bandit.outstanding, bandit.next_serial]
  end

  def test_epsilon_greedy_uses_only_the_supplied_draws
    bandit = build(arms: 4)
    feed(bandit, [1, 9, 9, 3])
    pick = lambda do |gate, draw|
      chosen = bandit.select_epsilon_greedy(ratio(1, 4), draws(gate, draw))
      assert_equal :abandoned, bandit.abandon(chosen.ticket)
      [chosen.arm, chosen.reason]
    end
    assert_equal [0, :explore], pick.call(0, 0)
    assert_equal [3, :explore], pick.call(2**62 - 1, MAX)
    assert_equal [1, :exploit], pick.call(2**62, MAX)
    assert_equal [1, :exploit], pick.call(MAX, 0)
  end

  def test_tie_break_rules
    winner = lambda do |rule, draw|
      bandit = build(arms: 3, tie_break: rule)
      3.times { bandit.reward(bandit.select_ucb1(0).ticket, 7) }
      bandit.select_epsilon_greedy(ratio(0, 1), draws(0, draw)).arm
    end
    assert_equal 0, winner.call(:lowest_index, MAX)
    assert_equal 2, winner.call(:highest_index, 0)
    assert_equal 0, winner.call(:least_tried, MAX)
    assert_equal 2, winner.call(:draw, MAX)
    assert_equal 1, winner.call(:draw, 2**63)
  end

  def test_late_reward_for_a_reset_arm_is_refused
    bandit = build(arms: 2)
    stale = bandit.select_ucb1(0).ticket
    other = bandit.select_ucb1(0).ticket
    assert_equal 1, bandit.reset(0)
    assert_equal :stale_ticket, bandit.reward(stale, 10)
    assert_equal :stale_ticket, bandit.abandon(stale)
    assert_equal 1, bandit.stats(0).generation
    fresh = bandit.select_ucb1(0)
    assert_equal [0, :cold_start, 1], [fresh.arm, fresh.reason, fresh.ticket.generation]
    assert_equal :reward_out_of_range, bandit.reward(other, 11)
    assert_equal :rewarded, bandit.reward(other, 3)
    assert_equal :unknown_ticket, bandit.reward(other, 3)
    assert_equal :unknown_ticket, bandit.abandon(other)
    assert_equal 1, bandit.reset_all
    assert_equal :stale_ticket, bandit.reward(fresh.ticket, 1)
    assert_equal 0, bandit.outstanding
  end

  def test_tickets_cannot_be_forged_and_outlive_their_bandit
    a = scoped(1)
    ticket = a.select_ucb1(0).ticket
    assert_raises(TypeError) { BanditMachines::Ticket.allocate }
    assert_raises(TypeError) { BanditMachines::Bandit.allocate }
    assert_raises(TypeError) { ticket.dup }
    assert_raises(NoMethodError) { BanditMachines::Bandit._new(1, 0, :draw) }
    assert_raises(TypeError) { a.reward(Object.new, 1) }
    a = nil
    GC.start
    assert_equal 1, ticket.scope
    assert_equal :unknown_ticket, build.abandon(ticket)
  end

  def test_ticket_capacity_revoke_and_no_open_arm
    bandit = build
    tickets = 64.times.map { bandit.select_ucb1(0).ticket }
    assert_equal :tickets_full, bandit.select_ucb1(0)
    assert_equal :tickets_full, bandit.select_epsilon_greedy(ratio(1, 1), draws(0, 0))
    assert_equal 64, bandit.outstanding
    assert_equal 60, bandit.revoke_before(61)
    assert_equal :unknown_ticket, bandit.reward(tickets.first, 1)
    assert_equal :rewarded, bandit.reward(tickets.last, 1)
    assert_equal 3, bandit.outstanding
    assert_instance_of BanditMachines::Selection, bandit.select_ucb1(0)
    3.times { |arm| assert_equal :closed, bandit.close(arm) }
    assert_equal :already_closed, bandit.close(0)
    assert_equal :no_open_arm, bandit.select_ucb1(0)
    assert_equal :no_open_arm, build(arms: 0).select_ucb1(0)
    assert_equal :opened, bandit.open(15)
    assert_equal :already_open, bandit.open(15)
    assert_equal 15, bandit.select_ucb1(0).arm
  end

  def test_repeated_open_and_close_change_nothing
    bandit = build(arms: 2)
    feed(bandit, [4, 6])
    pending = bandit.select_ucb1(0).ticket
    snapshot = -> { [16.times.map { |arm| bandit.stats(arm) }, bandit.outstanding, bandit.next_serial] }
    before = snapshot.call
    assert_equal :already_open, bandit.open(0)
    assert_equal :already_closed, bandit.close(9)
    assert_equal before, snapshot.call
    assert_equal :closed, bandit.close(0)
    closed = snapshot.call
    assert_equal :already_closed, bandit.close(0)
    assert_equal closed, snapshot.call
    assert_equal :opened, bandit.open(0)
    assert_equal before, snapshot.call
    assert_equal :rewarded, bandit.reward(pending, 1)
  end

  def test_closing_keeps_statistics_and_decay_discounts_them
    bandit = build(arms: 1)
    feed(bandit, [9, 9, 9])
    late = bandit.select_ucb1(0).ticket
    assert_equal :closed, bandit.close(0)
    assert_equal :decayed, bandit.decay(0, ratio(1, 2))
    stats = bandit.stats(0)
    assert_instance_of BanditMachines::ArmStats, stats
    assert_equal [false, 0, 2, 13, 1, 3], [stats.open, stats.generation, stats.rewarded, stats.reward_sum,
                                           stats.pending, stats.tries]
    assert_equal :rewarded, bandit.reward(late, 10)
    assert_equal 23, bandit.stats(0).reward_sum
    assert_equal :decayed, bandit.decay_all(ratio(0, 1))
    assert_equal 0, bandit.stats(0).tries
  end

  def test_strict_inputs_raise_before_mutation
    bandit = build
    ticket = bandit.select_ucb1(0).ticket
    bad = [1.0, Float::NAN, nil, '1', Object.new]
    bad.each do |value|
      assert_raises(TypeError) { bandit.select_ucb1(value) }
      assert_raises(TypeError) { bandit.reward(ticket, value) }
      assert_raises(TypeError) { bandit.reset(value) }
      assert_raises(TypeError) { bandit.revoke_before(value) }
      assert_raises(TypeError) { bandit.decay(0, ratio(value, 1)) }
      assert_raises(TypeError) { bandit.select_epsilon_greedy(ratio(0, 1), draws(value, 0)) }
      assert_raises(TypeError) { bandit.open(value) }
      assert_raises(TypeError) { build(max_reward: value) }
      assert_raises(TypeError) { build(prior: value) }
      assert_raises(TypeError) { BanditMachines::Bandit.new(value) }
      assert_raises(TypeError) { bandit.select_epsilon_greedy(value, draws(0, 0)) }
      assert_raises(TypeError) { bandit.select_epsilon_greedy(ratio(0, 1), value) }
      assert_raises(TypeError) { bandit.decay(0, value) }
      assert_raises(TypeError) { bandit.decay_all(value) }
    end
    assert_raises(TypeError) { BanditMachines::Bandit.new(max_reward: 10, prior: 5, tie_break: :draw) }
    assert_raises(TypeError) { bandit.select_epsilon_greedy([1, 4], [0, 0]) }
    assert_raises(ArgumentError) { BanditMachines::Bandit.new }
    assert_raises(TypeError) { bandit.select_ucb1(pick: 0) }
    assert_raises(ArgumentError) { bandit.select_ucb1(0, 0) }
    assert_raises(ArgumentError) { bandit.select_epsilon_greedy(ratio(0, 1)) }
    [-1, 2**64].each { |n| assert_raises(RangeError) { bandit.select_ucb1(n) } }
    [-1, 2**32].each { |n| assert_raises(RangeError) { bandit.reward(ticket, n) } }
    [-1, 2**64].each do |arm|
      assert_raises(RangeError) { bandit.reset(arm) }
      assert_raises(RangeError) { bandit.open(arm) }
      assert_raises(RangeError) { bandit.close(arm) }
      assert_raises(RangeError) { bandit.stats(arm) }
      assert_raises(RangeError) { bandit.ucb1_index_q56(arm) }
      assert_raises(RangeError) { bandit.decay(arm, ratio(1, 1)) }
    end
    assert_equal [1, 2], [bandit.outstanding, bandit.next_serial]
    assert_equal :rewarded, bandit.reward(ticket, 10)
  end

  def test_revoke_all_revokes_every_ticket_and_keeps_the_statistics
    bandit = scoped(3, arms: 2)
    assert_equal :rewarded, bandit.reward(bandit.select_ucb1(0).ticket, 7)
    assert_equal 0, bandit.revoke_all
    held = 3.times.map { bandit.select_ucb1(0).ticket }
    stats = 2.times.map { |arm| bandit.stats(arm) }
    # The old "everything" serial is another scope's, and revokes nothing.
    assert_equal :unknown_ticket, bandit.revoke_before(MAX)
    assert_equal 3, bandit.outstanding
    assert_equal 3, bandit.revoke_all
    assert_equal 0, bandit.outstanding
    held.each { |ticket| assert_equal :unknown_ticket, bandit.reward(ticket, 1) }
    2.times do |arm|
      assert_equal stats[arm].with(pending: 0), bandit.stats(arm)
    end
    assert_equal [1, 7, 0], [stats[0].rewarded, stats[0].reward_sum, bandit.stats(0).generation]
    assert_raises(ArgumentError) { bandit.revoke_all(1) }
  end

  def test_freeze_guards
    bandit = build
    ticket = bandit.select_ucb1(0).ticket
    bandit.freeze
    assert_raises(FrozenError) { bandit.select_ucb1(0) }
    assert_raises(FrozenError) { bandit.select_epsilon_greedy(ratio(0, 1), draws(0, 0)) }
    assert_raises(FrozenError) { bandit.reward(ticket, 1) }
    assert_raises(FrozenError) { bandit.abandon(ticket) }
    assert_raises(FrozenError) { bandit.revoke_before(1) }
    assert_raises(FrozenError) { bandit.revoke_all }
    assert_raises(FrozenError) { bandit.reset(0) }
    assert_raises(FrozenError) { bandit.reset_all }
    assert_raises(FrozenError) { bandit.decay(0, ratio(1, 2)) }
    assert_raises(FrozenError) { bandit.decay_all(ratio(1, 2)) }
    assert_raises(FrozenError) { bandit.open(5) }
    assert_raises(FrozenError) { bandit.close(0) }
    assert_equal [1, 2, 1], [bandit.outstanding, bandit.next_serial, bandit.stats(0).pending]
  end

  def test_gc_stress_and_upper_bounds
    previous = GC.stress
    GC.stress = true
    bandit = build(arms: 2, max_reward: 2**32 - 1, prior: 2**32 - 1, tie_break: :draw)
    chosen = bandit.select_epsilon_greedy(ratio(2**32 - 1, 2**32 - 1), draws(MAX, MAX))
    assert_equal [1, :cold_start], [chosen.arm, chosen.reason]
    GC.compact
    assert_equal :rewarded, bandit.reward(chosen.ticket, 2**32 - 1)
    assert_equal 2**32 - 1, bandit.stats(1).reward_sum
    assert_equal 0, bandit.revoke_before((2**48) - 1)
    assert_equal :unknown_ticket, bandit.revoke_before(MAX)
  ensure
    GC.stress = previous
  end
end
