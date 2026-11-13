# Changelog

## [0.4.0](https://github.com/seuros/bandit_machines/compare/bandit-machines-v0.3.0...bandit-machines-v0.4.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* see CONVENTIONS: one exhaustive Refusal per crate, Result from every mutator, honoured idempotent repeats, Ruby mirrors the core. No compatibility aliases. open/close return Opening/Closing; tie_break is required; Selection mirrors the core fields.

### Features

* name open and close outcomes, Config, Ratio and Draws as Ruby values, unknown arm answers :unknown_arm ([d94221c](https://github.com/seuros/bandit_machines/commit/d94221cbbf96f6a56638865e9f270a11f98c6077))

## [0.3.0](https://github.com/seuros/bandit_machines/compare/bandit-machines-v0.2.0...bandit-machines-v0.3.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* BanditMachines::Bandit.new no longer takes arms: and opens nothing; the caller opens arms explicitly, as in the Rust core.

### Features

* start every arm closed in the Ruby bandit and drop arms: ([4f4a0fe](https://github.com/seuros/bandit_machines/commit/4f4a0fef6e3aa39bf64dabca933753538f6e1238))

## [0.2.0](https://github.com/seuros/bandit_machines/compare/bandit-machines-v0.1.0...bandit-machines-v0.2.0) (2026-11-12)


### Features

* initial extraction ([850717f](https://github.com/seuros/bandit_machines/commit/850717fa9108a4c70fee063cae97407e1a383845))
