# frozen_string_literal: true

require 'minitest/autorun'
require 'open3'
require 'rbconfig'

# The classes of the values the extension wraps (Bandit, Ticket) are fixed
# when it loads. Nothing a caller does later, redefining a constant or
# `Object.const_get`, reaches a value being built. Each case runs in a fresh
# process: the class is fixed once per process, at load.
class ClassLookupTest < Minitest::Test
  LIB = File.expand_path('../lib', __dir__)

  PRELUDE = <<~RUBY
    require 'bandit_machines'
    B = BanditMachines
    CONFIG = B::Config.new(10, 5, :lowest_index)
  RUBY

  # Runs `body` after PRELUDE in a new Ruby; answers its last line of output.
  def isolated(body)
    out, err, status = Open3.capture3(RbConfig.ruby, '-I', LIB, '-e', PRELUDE + body)
    assert status.success?, "exited #{status.exitstatus}: #{err}"
    out.lines.last&.chomp
  end

  # Bug: a Bandit constant redefined before the first `new` made the value an
  # instance of the new class, without #open.
  def test_a_redefined_bandit_constant_does_not_change_the_bandit_class
    assert_equal 'true opened', isolated(<<~RUBY)
      original = B::Bandit
      B.send(:remove_const, :Bandit)
      B.const_set(:Bandit, Class.new)
      bandit = original.new(CONFIG)
      puts [bandit.instance_of?(original), bandit.open(0)].join(' ')
    RUBY
  end

  # Bug: a Ticket constant redefined before the first selection made the
  # ticket an instance of the new class, which Selection and #reward refused
  # after the core had issued it: the ticket was stranded.
  def test_a_redefined_ticket_constant_does_not_change_the_ticket_class
    assert_equal 'true rewarded 0', isolated(<<~RUBY)
      bandit = B::Bandit.new(CONFIG)
      bandit.open(0)
      original = B::Ticket
      B.send(:remove_const, :Ticket)
      B.const_set(:Ticket, Class.new)
      ticket = bandit.select_ucb1(0).ticket
      puts [ticket.instance_of?(original), bandit.reward(ticket, 3), bandit.outstanding].join(' ')
    RUBY
  end

  # Bug: building the first bandit and ticket ran `Object.const_get`, so a
  # redefined one could freeze the bandit after the core issued the ticket.
  def test_building_a_value_runs_no_const_get
    assert_equal '0 false 1 Bandit Ticket', isolated(<<~RUBY)
      hits = 0
      bandit = nil
      Object.define_singleton_method(:const_get) do |*args|
        hits += 1
        bandit&.freeze
        super(*args)
      end
      bandit = B::Bandit.new(CONFIG)
      bandit.open(0)
      ticket = bandit.select_ucb1(0).ticket
      puts [hits, bandit.frozen?, bandit.outstanding, bandit.class.name.split('::').last,
            ticket.class.name.split('::').last].join(' ')
    RUBY
  end

  # Bug: a redefined `Object.const_get` that raised hit an `unwrap` in the
  # extension: an uncatchable fatal, and the process exited.
  def test_a_raising_const_get_does_not_kill_the_process
    assert_equal 'opened rewarded', isolated(<<~RUBY)
      Object.define_singleton_method(:const_get) { |*| raise IOError, 'lookup ran after the core decided' }
      bandit = B::Bandit.new(CONFIG)
      opened = bandit.open(0)
      puts [opened, bandit.reward(bandit.select_ucb1(0).ticket, 3)].join(' ')
    RUBY
  end
end
