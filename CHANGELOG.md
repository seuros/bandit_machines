# Changelog

## [0.5.0](https://github.com/seuros/bandit_machines/compare/bandit_machines/v0.4.0...bandit_machines/v0.5.0) (2026-11-14)


### ⚠ BREAKING CHANGES

* seal values, check exact classes natively, clone-then-keep selection

### Features

* seal values, check exact classes natively, clone-then-keep selection ([b120e24](https://github.com/seuros/bandit_machines/commit/b120e242333aeff876a2b3a6cf4a51550ad96277))

## [0.4.0](https://github.com/seuros/bandit_machines/compare/bandit_machines/v0.3.0...bandit_machines/v0.4.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* handles are issued through periscope-machines. Every instance can be built with a scope; a handle from another scope is refused before any lookup; all handle kinds of an instance share one sequence; resumed restores the sequence after a restart. A ticket from another bandit answers UnknownTicket. Serials count from one; next_serial is an Option; revoke_before returns a Result.

### Features

* scope ticket serials through periscope-machines, add Bandit::resumed, issued and revoke_all ([1217d87](https://github.com/seuros/bandit_machines/commit/1217d879479a315a7407ed99d8b493283a347682))

## [0.3.0](https://github.com/seuros/bandit_machines/compare/bandit_machines/v0.2.0...bandit_machines/v0.3.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* see CONVENTIONS: one exhaustive Refusal per crate, Result from every mutator, honoured idempotent repeats, Ruby mirrors the core. No compatibility aliases. open/close return Opening/Closing; tie_break is required; Selection mirrors the core fields.

### Features

* name open and close outcomes, Config, Ratio and Draws as Ruby values, unknown arm answers :unknown_arm ([d94221c](https://github.com/seuros/bandit_machines/commit/d94221cbbf96f6a56638865e9f270a11f98c6077))

## [0.2.0](https://github.com/seuros/bandit_machines/compare/bandit_machines/v0.1.1...bandit_machines/v0.2.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* BanditMachines::Bandit.new no longer takes arms: and opens nothing; the caller opens arms explicitly, as in the Rust core.

### Features

* start every arm closed in the Ruby bandit and drop arms: ([4f4a0fe](https://github.com/seuros/bandit_machines/commit/4f4a0fef6e3aa39bf64dabca933753538f6e1238))

## [0.1.1](https://github.com/seuros/bandit_machines/compare/bandit_machines-v0.1.0...bandit_machines/v0.1.1) (2026-11-12)


### Features

* initial extraction ([850717f](https://github.com/seuros/bandit_machines/commit/850717fa9108a4c70fee063cae97407e1a383845))
