# frozen_string_literal: true

require_relative 'bandit_machines/version'
# Native is required: no algorithm fallback.
require 'bandit_machines_native/bandit_machines_native'

module BanditMachines
  Selection = Data.define(:ticket, :arm, :reason)
  ArmStats = Data.define(:open, :generation, :rewarded, :reward_sum, :pending) do
    def tries = rewarded + pending
  end

  class Bandit
    # Every arm starts closed: open the ones to explore.
    def self.new(max_reward:, prior:, tie_break: :lowest_index)
      _new(max_reward, prior, tie_break)
    end
    private_class_method :_new

    def select_ucb1(pick: 0)
      selection(_select_ucb1(pick))
    end

    def select_epsilon_greedy(numerator:, denominator:, gate:, pick:)
      selection(_select_epsilon_greedy(numerator, denominator, gate, pick))
    end

    def decay(arm, numerator:, denominator:)
      _decay(arm, numerator, denominator)
    end

    def decay_all(numerator:, denominator:)
      _decay_all(numerator, denominator)
    end

    def stats(arm)
      ArmStats.new(*_stats(arm))
    end

    private

    def selection(outcome)
      outcome.is_a?(Symbol) ? outcome : Selection.new(*outcome)
    end

    private :_select_ucb1, :_select_epsilon_greedy, :_decay, :_decay_all, :_stats
  end
end
