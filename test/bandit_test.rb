# frozen_string_literal: true
require 'minitest/autorun'
require 'bandit_machines'

class BanditTest < Minitest::Test
  MAX = 2**64 - 1

  def build(arms: 3, max_reward: 10, prior: 5, **options)
    bandit = BanditMachines::Bandit.new(max_reward:, prior:, **options)
    arms.times { |arm| assert_equal true, bandit.open(arm) }
    bandit
  end

  def feed(bandit, rewards)
    rewards.each { |reward| assert_equal true, bandit.reward(bandit.select_ucb1.ticket, reward) }
  end

  def test_capacity_constants_and_configuration
    bandit = build(tie_break: :draw)
    assert_equal 16, BanditMachines::ARM_CAPACITY
    assert_equal 64, BanditMachines::TICKET_CAPACITY
    assert_equal 16, bandit.arm_capacity
    assert_equal 64, bandit.ticket_capacity
    assert_equal [10, 5, :draw], [bandit.max_reward, bandit.prior, bandit.tie_break]
    assert_equal :lowest_index, build.tie_break
    assert_equal [true, true, true, false], 4.times.map { |arm| bandit.stats(arm).open }
  end

  def test_a_new_bandit_has_every_arm_closed
    bandit = BanditMachines::Bandit.new(max_reward: 10, prior: 5)
    assert_equal [false] * 16, 16.times.map { |arm| bandit.stats(arm).open }
    assert_equal :no_open_arm, bandit.select_ucb1
    assert_equal :no_open_arm, bandit.select_epsilon_greedy(numerator: 1, denominator: 1, gate: 0, pick: 0)
    assert_equal [0, 0], [bandit.outstanding, bandit.next_serial]
    assert_equal [0, 0, 0], bandit.stats(0).to_h.values_at(:generation, :pending, :rewarded)
    16.times { |arm| assert_nil bandit.ucb1_index_q56(arm) }
    assert_equal false, bandit.close(7)
    assert_equal true, bandit.open(7)
    chosen = bandit.select_ucb1(pick: MAX)
    assert_equal [7, :cold_start, 0], [chosen.arm, chosen.reason, chosen.ticket.serial]
    assert_equal [7], 16.times.select { |arm| bandit.stats(arm).open }
  end

  def test_ucb1_sweeps_cold_arms_then_follows_the_index
    bandit = build(arms: 2)
    first = bandit.select_ucb1
    assert_instance_of BanditMachines::Selection, first
    assert_instance_of BanditMachines::Ticket, first.ticket
    assert_equal [0, :cold_start, 0, 0, 0], [first.arm, first.reason, first.ticket.arm, first.ticket.serial,
                                             first.ticket.generation]
    assert_predicate first, :frozen?
    assert_equal true, bandit.reward(first.ticket, 0)
    second = bandit.select_ucb1(pick: MAX)
    assert_equal [1, :cold_start], [second.arm, second.reason]
    assert_equal true, bandit.reward(second.ticket, 10)
    assert_equal 2**56, bandit.ucb1_index_q56(1) - bandit.ucb1_index_q56(0)
    assert_nil bandit.ucb1_index_q56(2)
    third = bandit.select_ucb1
    assert_equal [1, :upper_bound, 2], [third.arm, third.reason, third.ticket.serial]
    assert_equal [1, 3], [bandit.outstanding, bandit.next_serial]
  end

  def test_epsilon_greedy_uses_only_the_supplied_draws
    bandit = build(arms: 4)
    feed(bandit, [1, 9, 9, 3])
    pick = lambda do |gate, draw|
      chosen = bandit.select_epsilon_greedy(numerator: 1, denominator: 4, gate: gate, pick: draw)
      assert_equal true, bandit.abandon(chosen.ticket)
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
      3.times { bandit.reward(bandit.select_ucb1.ticket, 7) }
      bandit.select_epsilon_greedy(numerator: 0, denominator: 1, gate: 0, pick: draw).arm
    end
    assert_equal 0, winner.call(:lowest_index, MAX)
    assert_equal 2, winner.call(:highest_index, 0)
    assert_equal 0, winner.call(:least_tried, MAX)
    assert_equal 2, winner.call(:draw, MAX)
    assert_equal 1, winner.call(:draw, 2**63)
  end

  def test_late_reward_for_a_reset_arm_is_refused
    bandit = build(arms: 2)
    stale = bandit.select_ucb1.ticket
    other = bandit.select_ucb1.ticket
    assert_equal 1, bandit.reset(0)
    assert_equal :stale_ticket, bandit.reward(stale, 10)
    assert_equal :stale_ticket, bandit.abandon(stale)
    assert_equal 1, bandit.stats(0).generation
    fresh = bandit.select_ucb1
    assert_equal [0, :cold_start, 1], [fresh.arm, fresh.reason, fresh.ticket.generation]
    assert_equal :reward_out_of_range, bandit.reward(other, 11)
    assert_equal true, bandit.reward(other, 3)
    assert_equal :unknown_ticket, bandit.reward(other, 3)
    assert_equal :unknown_ticket, bandit.abandon(other)
    assert_equal 1, bandit.reset_all
    assert_equal :stale_ticket, bandit.reward(fresh.ticket, 1)
    assert_equal 0, bandit.outstanding
  end

  def test_tickets_belong_to_one_bandit_and_cannot_be_forged
    a = build
    b = build
    ticket = a.select_ucb1.ticket
    b.select_ucb1
    assert_equal :unknown_ticket, b.reward(ticket, 1)
    assert_equal :unknown_ticket, b.abandon(ticket)
    assert_raises(TypeError) { BanditMachines::Ticket.allocate }
    assert_raises(TypeError) { BanditMachines::Bandit.allocate }
    assert_raises(TypeError) { ticket.dup }
    assert_raises(NoMethodError) { BanditMachines::Bandit._new(1, 0, :draw) }
    assert_raises(NoMethodError) { a._stats(0) }
    assert_raises(TypeError) { a.reward(Object.new, 1) }
    a = nil
    GC.start
    assert_equal :unknown_ticket, build.abandon(ticket)
  end

  def test_ticket_capacity_revoke_and_no_open_arm
    bandit = build
    tickets = 64.times.map { bandit.select_ucb1.ticket }
    assert_equal :tickets_full, bandit.select_ucb1
    assert_equal :tickets_full, bandit.select_epsilon_greedy(numerator: 1, denominator: 1, gate: 0, pick: 0)
    assert_equal 64, bandit.outstanding
    assert_equal 60, bandit.revoke_before(60)
    assert_equal :unknown_ticket, bandit.reward(tickets.first, 1)
    assert_equal true, bandit.reward(tickets.last, 1)
    assert_equal 3, bandit.outstanding
    assert_instance_of BanditMachines::Selection, bandit.select_ucb1
    3.times { |arm| assert_equal true, bandit.close(arm) }
    assert_equal false, bandit.close(0)
    assert_equal :no_open_arm, bandit.select_ucb1
    assert_equal :no_open_arm, build(arms: 0).select_ucb1
    assert_equal true, bandit.open(15)
    assert_equal false, bandit.open(15)
    assert_equal 15, bandit.select_ucb1.arm
  end

  def test_closing_keeps_statistics_and_decay_discounts_them
    bandit = build(arms: 1)
    feed(bandit, [9, 9, 9])
    late = bandit.select_ucb1.ticket
    assert_equal true, bandit.close(0)
    assert_equal true, bandit.decay(0, numerator: 1, denominator: 2)
    stats = bandit.stats(0)
    assert_instance_of BanditMachines::ArmStats, stats
    assert_equal [false, 0, 2, 13, 1, 3], [stats.open, stats.generation, stats.rewarded, stats.reward_sum,
                                           stats.pending, stats.tries]
    assert_equal true, bandit.reward(late, 10)
    assert_equal 23, bandit.stats(0).reward_sum
    assert_equal true, bandit.decay_all(numerator: 0, denominator: 1)
    assert_equal 0, bandit.stats(0).tries
  end

  def test_strict_inputs_raise_before_mutation
    bandit = build
    ticket = bandit.select_ucb1.ticket
    bad = [1.0, Float::NAN, nil, '1', Object.new]
    bad.each do |value|
      assert_raises(TypeError) { bandit.select_ucb1(pick: value) }
      assert_raises(TypeError) { bandit.reward(ticket, value) }
      assert_raises(TypeError) { bandit.reset(value) }
      assert_raises(TypeError) { bandit.revoke_before(value) }
      assert_raises(TypeError) { bandit.decay(0, numerator: value, denominator: 1) }
      assert_raises(TypeError) { bandit.select_epsilon_greedy(numerator: 0, denominator: 1, gate: value, pick: 0) }
      assert_raises(TypeError) { bandit.open(value) }
      assert_raises(TypeError) { build(max_reward: value) }
      assert_raises(TypeError) { build(prior: value) }
    end
    assert_raises(TypeError) { build(tie_break: 'draw') }
    assert_raises(ArgumentError) { build(tie_break: :coin) }
    assert_raises(ArgumentError) { BanditMachines::Bandit.new(max_reward: 1) }
    assert_raises(ArgumentError) { BanditMachines::Bandit.new(arms: 3, max_reward: 10, prior: 5) }
    [-1, 2**64].each { |n| assert_raises(RangeError) { bandit.select_ucb1(pick: n) } }
    [-1, 2**32].each { |n| assert_raises(RangeError) { bandit.reward(ticket, n) } }
    [-1, 16, 2**64].each do |arm|
      assert_raises(RangeError) { bandit.reset(arm) }
      assert_raises(RangeError) { bandit.open(arm) }
      assert_raises(RangeError) { bandit.close(arm) }
      assert_raises(RangeError) { bandit.stats(arm) }
      assert_raises(RangeError) { bandit.ucb1_index_q56(arm) }
      assert_raises(RangeError) { bandit.decay(arm, numerator: 1, denominator: 1) }
    end
    assert_raises(RangeError) { build(max_reward: 0, prior: 0) }
    assert_raises(RangeError) { build(max_reward: 4, prior: 5) }
    assert_raises(RangeError) { bandit.decay_all(numerator: 2, denominator: 1) }
    assert_raises(RangeError) { bandit.decay(0, numerator: 0, denominator: 0) }
    assert_raises(RangeError) { bandit.select_epsilon_greedy(numerator: 3, denominator: 2, gate: 0, pick: 0) }
    assert_equal [1, 1], [bandit.outstanding, bandit.next_serial]
    assert_equal true, bandit.reward(ticket, 10)
  end

  def test_freeze_guards
    bandit = build
    ticket = bandit.select_ucb1.ticket
    bandit.freeze
    assert_raises(FrozenError) { bandit.select_ucb1 }
    assert_raises(FrozenError) { bandit.select_epsilon_greedy(numerator: 0, denominator: 1, gate: 0, pick: 0) }
    assert_raises(FrozenError) { bandit.reward(ticket, 1) }
    assert_raises(FrozenError) { bandit.abandon(ticket) }
    assert_raises(FrozenError) { bandit.revoke_before(1) }
    assert_raises(FrozenError) { bandit.reset(0) }
    assert_raises(FrozenError) { bandit.reset_all }
    assert_raises(FrozenError) { bandit.decay(0, numerator: 1, denominator: 2) }
    assert_raises(FrozenError) { bandit.decay_all(numerator: 1, denominator: 2) }
    assert_raises(FrozenError) { bandit.open(5) }
    assert_raises(FrozenError) { bandit.close(0) }
    assert_equal [1, 1, 1], [bandit.outstanding, bandit.next_serial, bandit.stats(0).pending]
  end

  def test_gc_stress_and_upper_bounds
    previous = GC.stress
    GC.stress = true
    bandit = build(arms: 2, max_reward: 2**32 - 1, prior: 2**32 - 1, tie_break: :draw)
    chosen = bandit.select_epsilon_greedy(numerator: 2**32 - 1, denominator: 2**32 - 1, gate: MAX, pick: MAX)
    assert_equal [1, :cold_start], [chosen.arm, chosen.reason]
    GC.compact
    assert_equal true, bandit.reward(chosen.ticket, 2**32 - 1)
    assert_equal 2**32 - 1, bandit.stats(1).reward_sum
    assert_equal 0, bandit.revoke_before(MAX)
  ensure
    GC.stress = previous
  end
end
