# frozen_string_literal: true
require_relative 'support'

# Tickets are scoped ids: only the bandit that issued one honours it.
class ScopeTest < Minitest::Test
  include BanditSupport

  # The review's sequence: two bandits with identical config, the same calls on
  # both, and one rewarded the other's ticket. In different scopes each refuses.
  def test_ticket_from_another_scope_is_refused_and_changes_nothing
    a = scoped(1)
    b = scoped(2)
    assert_equal a.config, b.config
    from_a = a.select_ucb1(0).ticket
    from_b = b.select_ucb1(0).ticket
    assert_equal [1, 2, 1, 2], [a.scope, b.scope, from_a.scope, from_b.scope]
    assert_equal [(1 << 48) | 1, (2 << 48) | 1], [from_a.serial, from_b.serial]
    snapshot = ->(bandit) { [bandit.stats(0), bandit.outstanding, bandit.next_serial] }
    before = [snapshot.(a), snapshot.(b)]
    assert_equal :unknown_ticket, b.reward(from_a, 1)
    assert_equal :unknown_ticket, b.abandon(from_a)
    assert_equal :unknown_ticket, b.revoke_before(from_a.serial)
    assert_equal :unknown_ticket, b.revoke_before(a.next_serial)
    assert_equal :unknown_ticket, a.reward(from_b, 1)
    assert_equal :unknown_ticket, a.abandon(from_b)
    assert_equal :unknown_ticket, a.revoke_before(from_b.serial)
    assert_equal before, [snapshot.(a), snapshot.(b)]
    assert_equal 1, b.reset(0)
    assert_equal %i[stale_ticket unknown_ticket], [b.reward(from_b, 1), b.reward(from_a, 1)]
    assert_equal :rewarded, a.reward(from_a, 1)
  end

  # The core cannot pick a scope: two bandits left in scope zero issue the same
  # tickets, and one honours the other's.
  def test_bandits_left_in_the_same_scope_still_alias
    a = build
    b = build
    ticket = a.select_ucb1(0).ticket
    b.select_ucb1(0)
    assert_equal [0, 0, 0, 1], [a.scope, b.scope, ticket.scope, ticket.serial]
    assert_equal :rewarded, b.reward(ticket, 1)
    assert_equal 0, b.outstanding
  end

  def test_with_scope_takes_a_config_and_an_integer_scope
    bandit = BanditMachines::Bandit.with_scope(config, 65_535)
    assert_equal [65_535, config, (65_535 << 48) | 1], [bandit.scope, bandit.config, bandit.next_serial]
    assert_equal 0, BanditMachines::Bandit.with_scope(config, 0).scope
    [1.0, nil, '1', Object.new].each do |value|
      assert_raises(TypeError) { BanditMachines::Bandit.with_scope(config, value) }
      assert_raises(TypeError) { BanditMachines::Bandit.with_scope(value, 1) }
    end
    [-1, 65_536, 2**64].each { |scope| assert_raises(RangeError) { BanditMachines::Bandit.with_scope(config, scope) } }
    assert_raises(ArgumentError) { BanditMachines::Bandit.with_scope(config) }
  end

  # A fresh bandit counts from one again, so a ticket that outlived a restart
  # would redeem a new selection. A resumed one continues the scope.
  def test_resumed_bandit_continues_its_scope_and_refuses_earlier_tickets
    first = scoped(5, arms: 2)
    old = [first.select_ucb1(0).ticket]
    assert_equal :rewarded, first.reward(old.first, 3)
    old.concat(2.times.map { first.select_ucb1(0).ticket })
    assert_equal [3, (5 << 48) | 4], [first.issued, first.next_serial]

    bandit = BanditMachines::Bandit.resumed(first.config, first.scope, first.issued)
    assert_equal [5, 3, 0, config], [bandit.scope, bandit.issued, bandit.outstanding, bandit.config]
    assert_equal [false] * 16, 16.times.map { |arm| bandit.stats(arm).open }
    2.times { |arm| bandit.open(arm) }
    fresh = 2.times.map { bandit.select_ucb1(0).ticket }
    assert_equal [(5 << 48) | 4, (5 << 48) | 5, 5], [*fresh.map(&:serial), bandit.issued]
    old.each do |ticket|
      assert_equal :unknown_ticket, bandit.reward(ticket, 1)
      assert_equal :unknown_ticket, bandit.abandon(ticket)
      assert_equal 0, bandit.revoke_before(ticket.serial)
    end
    assert_equal [2, 5], [bandit.outstanding, bandit.issued]
    assert_equal %i[rewarded abandoned], [bandit.reward(fresh[0], 1), bandit.abandon(fresh[1])]
  end

  def test_resumed_at_the_last_sequence_is_spent_and_past_it_raises
    last = (2**48) - 1
    bandit = BanditMachines::Bandit.resumed(config, 3, last - 1)
    bandit.open(0)
    ticket = bandit.select_ucb1(0).ticket
    assert_equal [(3 << 48) | last, last, nil], [ticket.serial, bandit.issued, bandit.next_serial]
    assert_equal :exhausted, bandit.select_ucb1(0)
    assert_equal :exhausted, bandit.select_epsilon_greedy(ratio(1, 1), draws(0, 0))
    assert_equal [1, last], [bandit.outstanding, bandit.issued]
    spent = BanditMachines::Bandit.resumed(config, 3, last)
    spent.open(0)
    assert_equal [:exhausted, 0, nil], [spent.select_ucb1(0), spent.outstanding, spent.next_serial]
    assert_raises(RangeError) { BanditMachines::Bandit.resumed(config, 3, last + 1) }
    assert_raises(RangeError) { BanditMachines::Bandit.resumed(config, 3, -1) }
    assert_raises(RangeError) { BanditMachines::Bandit.resumed(config, 65_536, 0) }
    [1.0, nil, '1'].each do |value|
      assert_raises(TypeError) { BanditMachines::Bandit.resumed(value, 3, 0) }
      assert_raises(TypeError) { BanditMachines::Bandit.resumed(config, value, 0) }
      assert_raises(TypeError) { BanditMachines::Bandit.resumed(config, 3, value) }
    end
  end
end
