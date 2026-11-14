# frozen_string_literal: true
require_relative 'support'

# Values built by hand are held to the rules of the ones the gem returns, and
# a selection that cannot be built afterwards loses no work.
class ValuesTest < Minitest::Test
  include BanditSupport

  def test_a_hand_built_selection_is_checked
    selection = build.select_ucb1(0)
    copy = BanditMachines::Selection.new(selection.ticket, :explore)
    assert_equal [selection.ticket, :explore, selection.arm], [copy.ticket, copy.reason, copy.arm]
    assert_equal selection, BanditMachines::Selection.new(selection.ticket, selection.reason)
    assert_raises(TypeError) { BanditMachines::Selection.new(0, :explore) }
    assert_raises(TypeError) { BanditMachines::Selection.new(selection.ticket, 'explore') }
    assert_raises(RangeError) { BanditMachines::Selection.new(selection.ticket, :hunch) }
  end

  def test_a_hand_built_arm_stats_is_checked
    stats = build.stats(0)
    assert_equal stats, BanditMachines::ArmStats.new(true, 0, 0, 0, 0)
    assert_equal [MAX, MAX], [BanditMachines::ArmStats.new(false, MAX, MAX, MAX, MAX).tries, MAX]
    assert_raises(TypeError) { BanditMachines::ArmStats.new(nil, 0, 0, 0, 0) }
    assert_raises(TypeError) { BanditMachines::ArmStats.new(1, 0, 0, 0, 0) }
    assert_raises(TypeError) { BanditMachines::ArmStats.new(true, 0.0, 0, 0, 0) }
    assert_raises(RangeError) { BanditMachines::ArmStats.new(true, 0, -1, 0, 0) }
    assert_raises(RangeError) { BanditMachines::ArmStats.new(true, 0, 0, 2**64, 0) }
    assert_raises(RangeError) { BanditMachines::ArmStats.new(true, 0, 0, 0, -1) }
  end

  # The classes are resolved once, at init: removing a constant later changes
  # nothing the binding builds or reads.
  def test_the_classes_are_resolved_once_at_init
    bandit = build
    selection = BanditMachines.send(:remove_const, :Selection)
    begin
      assert_equal 'BanditMachines::Selection', bandit.select_ucb1(0).class.name
    ensure
      BanditMachines.const_set(:Selection, selection)
    end
  end

  # Sealing stops accidents, not reflection: Ruby binds a named subclass's
  # constant before `inherited` raises, so the class leaks. Both forms raise.
  def test_values_are_sealed_and_a_named_subclass_leaks_its_constant
    [BanditMachines::Config, BanditMachines::Ratio, BanditMachines::Draws, BanditMachines::Selection,
     BanditMachines::ArmStats].each do |value|
      assert_raises(TypeError) { Class.new(value) }
      assert_raises(NoMethodError) { value.allocate }
    end
    assert_raises(TypeError) do
      self.class.module_eval('class LeakedRatio < BanditMachines::Ratio; end', __FILE__, __LINE__)
    end
    assert self.class.const_defined?(:LeakedRatio, false), 'the constant is bound before inherited raises'
  end

  # A forged value never reaches the core: a leaked subclass's instance is a
  # TypeError, a Marshal frame edited past the check a RangeError, and neither
  # earns a refusal symbol.
  def test_a_forged_value_is_refused_at_the_boundary
    assert_raises(TypeError) do
      self.class.module_eval('class ForgedDraws < BanditMachines::Draws; end', __FILE__, __LINE__)
    end
    ForgedDraws.class_eval { def initialize(**fields) = Data.instance_method(:initialize).bind_call(self, **fields) }
    bandit = build
    forged = ForgedDraws.new(gate: 0, pick: 0)
    assert_raises(TypeError) { bandit.select_epsilon_greedy(ratio(1, 2), forged) }
    edited = Marshal.load(Marshal.dump(ratio(1, 2)).sub("i\x06".b, "i\x0C".b))
    assert_equal [7, 2], [edited.numerator, edited.denominator]
    assert_raises(RangeError) { bandit.select_epsilon_greedy(edited, draws(0, 0)) }
    assert_raises(RangeError) { bandit.decay(0, edited) }
    assert_equal [0, 0], [bandit.outstanding, bandit.issued]
  end

  # The classes are pinned at init, so a compaction moves none of them: every
  # path that builds or reads a value still works after it.
  def test_every_value_path_survives_gc_compact
    skip 'GC.compact is not available' unless GC.respond_to?(:compact)
    GC.compact
    bandit = build
    chosen = bandit.select_ucb1(0)
    assert_instance_of BanditMachines::Selection, chosen
    assert_instance_of BanditMachines::Selection, bandit.select_epsilon_greedy(ratio(1, 2), draws(0, 0))
    assert_instance_of BanditMachines::ArmStats, bandit.stats(0)
    assert_instance_of BanditMachines::Config, bandit.config
    assert_equal :rewarded, bandit.reward(chosen.ticket, 1)
    assert_equal BanditMachines::Ratio::ONE, ratio(1, 1)
  end

  # A selection whose build freezes or re-enters the bandit is not applied.
  # The binding builds the Selection without running Ruby code, so neither
  # can happen through `new`; a frozen bandit raises before anything moves.
  def test_a_frozen_bandit_refuses_every_mutating_call
    bandit = build
    ticket = bandit.select_ucb1(0).ticket
    bandit.freeze
    [-> { bandit.select_ucb1(0) }, -> { bandit.select_epsilon_greedy(ratio(1, 2), draws(0, 0)) },
     -> { bandit.reward(ticket, 1) }, -> { bandit.abandon(ticket) }, -> { bandit.revoke_before(1) },
     -> { bandit.revoke_all }, -> { bandit.reset(0) }, -> { bandit.reset_all }, -> { bandit.decay(0, ratio(1, 2)) },
     -> { bandit.decay_all(ratio(1, 2)) }, -> { bandit.open(1) }, -> { bandit.close(0) }].each do |call|
      assert_raises(FrozenError) { call.call }
    end
    assert_equal 1, bandit.outstanding
  end

  # Frozen is checked before any argument is read: a bad argument to a frozen
  # bandit is FrozenError, not TypeError or RangeError.
  def test_a_frozen_bandit_raises_before_reading_arguments
    bandit = build.freeze
    [-> { bandit.select_ucb1('0') }, -> { bandit.select_epsilon_greedy(0.5, nil) }, -> { bandit.reward(nil, 1) },
     -> { bandit.abandon(1) }, -> { bandit.revoke_before(-1) }, -> { bandit.reset(1.5) },
     -> { bandit.decay(:arm, nil) }, -> { bandit.decay_all([1, 2]) }, -> { bandit.open('1') },
     -> { bandit.close(nil) }].each do |call|
      assert_raises(FrozenError) { call.call }
    end
  end

  # A singleton class opened on a genuine value does not change what it is.
  def test_an_opened_singleton_class_is_still_the_gems_value
    bandit = build
    keep = ratio(1, 2).tap(&:singleton_class)
    assert_equal :decayed, bandit.decay_all(keep)
    chosen = bandit.select_epsilon_greedy(ratio(0, 1).clone.tap(&:singleton_class), draws(0, 0).tap(&:singleton_class))
    ticket = chosen.ticket.tap(&:singleton_class)
    assert_equal chosen, BanditMachines::Selection.new(ticket, chosen.reason)
    config = bandit.config.tap(&:singleton_class)
    assert_equal config, BanditMachines::Bandit.new(config).config
  end

  # Fields are read from the struct itself: a redefined reader answers
  # nothing to the binding and never runs.
  def test_a_redefined_reader_is_ignored
    raising = -> { raise 'the binding ran a reader' }
    bandit = build
    readers(BanditMachines::Config, max_reward: -> { 0 }, prior: raising, tie_break: raising) do
      readers(BanditMachines::Ratio, numerator: -> { 9 }, denominator: raising) do
        readers(BanditMachines::Draws, gate: raising, pick: raising) do
          assert_equal 10, BanditMachines::Bandit.new(config).config.to_h.fetch(:max_reward)
          assert_equal :decayed, bandit.decay_all(ratio(1, 2))
          assert_equal :cold_start, bandit.select_epsilon_greedy(ratio(0, 1), draws(0, 0)).reason
        end
      end
    end
  end

  # A Ticket is read as exactly a Ticket. A subclass has no instance (the
  # allocator is gone and only a bandit issues tickets), and anything else is
  # a TypeError naming the Ticket.
  def test_a_ticket_is_read_as_exactly_a_ticket
    bandit = build
    assert_raises(TypeError) { Class.new(BanditMachines::Ticket).allocate }
    [-> { bandit.reward(bandit, 1) }, -> { bandit.abandon(Object.new) },
     -> { BanditMachines::Selection.new(bandit, :explore) }].each do |call|
      assert_equal 'ticket must be a BanditMachines::Ticket', assert_raises(TypeError, &call).message
    end
  end

  # A tie_break outside the core's `TieBreak` names every one it accepts.
  def test_an_unknown_tie_break_lists_the_accepted_ones
    error = assert_raises(RangeError) { config(tie_break: :coin) }
    assert_equal 'tie_break must be one of :lowest_index, :highest_index, :least_tried, :draw', error.message
  end

  # After the core issued a ticket the binding builds the Selection natively:
  # no Ruby `initialize` runs, so one that raises cannot lose the ticket.
  def test_a_selection_is_built_without_running_ruby
    bandit = build
    selection_class = BanditMachines::Selection
    selection_class.alias_method(:__checked_initialize, :initialize)
    selection_class.define_method(:initialize) { |*| raise 'never runs' }
    begin
      chosen = bandit.select_ucb1(0)
      explored = bandit.select_epsilon_greedy(ratio(1, 1), draws(0, 0))
    ensure
      selection_class.alias_method(:initialize, :__checked_initialize)
      selection_class.remove_method(:__checked_initialize)
    end
    assert_equal [[0, :cold_start, 1], [1, :cold_start, 2]],
                 [chosen, explored].map { [it.arm, it.reason, it.ticket.serial] }
    assert_predicate chosen, :frozen?
    assert_equal chosen, BanditMachines::Selection.new(chosen.ticket, :cold_start)
    assert_equal 2, bandit.outstanding
  end

  private

  # Runs the block with each reader of `value` redefined, then restores it.
  def readers(value, **bodies)
    bodies.each_key { |name| value.alias_method(:"__original_#{name}", name) }
    bodies.each { |name, body| value.define_method(name, &body) }
    yield
  ensure
    bodies.each_key do |name|
      value.alias_method(name, :"__original_#{name}")
      value.remove_method(:"__original_#{name}")
    end
  end
end
