# frozen_string_literal: true
require 'minitest/autorun'
require 'bandit_machines'

# Builders shared by the test classes.
module BanditSupport
  MAX = 2**64 - 1

  def config(max_reward: 10, prior: 5, tie_break: :lowest_index)
    BanditMachines::Config.new(max_reward, prior, tie_break)
  end

  def build(arms: 3, **options)
    bandit = BanditMachines::Bandit.new(config(**options))
    arms.times { |arm| assert_equal :opened, bandit.open(arm) }
    bandit
  end

  def ratio(numerator, denominator) = BanditMachines::Ratio.new(numerator, denominator)
  def draws(gate, pick) = BanditMachines::Draws.new(gate, pick)

  def feed(bandit, rewards)
    rewards.each { |reward| assert_equal :rewarded, bandit.reward(bandit.select_ucb1(0).ticket, reward) }
  end

  def scoped(scope, arms: 1)
    bandit = BanditMachines::Bandit.with_scope(config, scope)
    arms.times { |arm| bandit.open(arm) }
    bandit
  end
end
