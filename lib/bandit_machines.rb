# frozen_string_literal: true

require_relative 'bandit_machines/version'

# A bounded multi-armed bandit. Every value mirrors the Rust core's type of
# the same name, field for field; each checks itself natively when it is built.
module BanditMachines
  # Reward range, warming prior and tie rule shared by every arm:
  # `0 < max_reward`, `prior <= max_reward`, both in 0..2**32-1, and
  # `tie_break` one of :lowest_index, :highest_index, :least_tried, :draw.
  Config = Data.define(:max_reward, :prior, :tie_break) do
    def initialize(max_reward:, prior:, tie_break:)
      super
      __validate
    end
  end

  # A fraction in 0..1: `numerator <= denominator`, `denominator > 0`, both in
  # 0..2**32-1. `Ratio::ZERO` and `Ratio::ONE` are the core's constants.
  Ratio = Data.define(:numerator, :denominator) do
    def initialize(numerator:, denominator:)
      super
      __validate
    end
  end

  # The two uniform draws one epsilon-greedy selection may consume, each in
  # 0..2**64-1: `gate` decides explore versus exploit, `pick` chooses the arm.
  Draws = Data.define(:gate, :pick) do
    def initialize(gate:, pick:)
      super
      __validate
    end
  end

  # A selected arm's ticket and the rule that picked it: `ticket` a Ticket,
  # `reason` one of :cold_start, :explore, :exploit, :upper_bound.
  Selection = Data.define(:ticket, :reason) do
    def initialize(ticket:, reason:)
      super
      __validate
    end

    def arm = ticket.arm
  end

  # Snapshot of one arm: `open` true or false, every count in 0..2**64-1.
  ArmStats = Data.define(:open, :generation, :rewarded, :reward_sum, :pending) do
    def initialize(open:, generation:, rewarded:, reward_sum:, pending:)
      super
      __validate
    end

    def tries = [rewarded + pending, (2**64) - 1].min
  end
end

# Native is required: no algorithm fallback. It adds the checks to the values
# above, so they must exist first.
require 'bandit_machines_native/bandit_machines_native'

module BanditMachines
  # The values are sealed. `initialize` is where each validates, and a
  # subclass could override it, or `allocate` build an instance that never ran
  # it: subclassing raises TypeError and `allocate` is gone. Sealing stops
  # accidents, not reflection (a named subclass's constant is bound before
  # `inherited` raises; `Marshal.load` skips `initialize`), so the native side
  # is the authority: a value it reads must be exactly its class, and every
  # field is converted again through the core's constructors.
  [Config, Ratio, Draws, Selection, ArmStats].each do |value|
    value.singleton_class.undef_method(:allocate)
    value.define_singleton_method(:inherited) { |_| raise TypeError, "#{value} cannot be subclassed" }
  end
end
