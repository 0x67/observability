# Changelog

## [0.3.0](https://github.com/0x67/observability/compare/observability-core-v0.2.1...observability-core-v0.3.0) (2026-11-27)


### ⚠ BREAKING CHANGES

* **core:** return a flusher handle and publish interval windows ([#47](https://github.com/0x67/observability/issues/47))
* **core:** free per-thread slots when threads exit ([#45](https://github.com/0x67/observability/issues/45))
* **core:** read the metrics gate from one atomic ([#49](https://github.com/0x67/observability/issues/49))

### Features

* **pipeline:** bound log sink queues and count dropped lines ([#62](https://github.com/0x67/observability/issues/62)) ([2ba678d](https://github.com/0x67/observability/commit/2ba678d60eb8f791aba19df601975c101f091c81))


### Bug fixes

* **core:** free per-thread slots when threads exit ([#45](https://github.com/0x67/observability/issues/45)) ([ad20983](https://github.com/0x67/observability/commit/ad20983bad8849b0a04ee5ed639007d91cf6b2ec))
* **core:** return a flusher handle and publish interval windows ([#47](https://github.com/0x67/observability/issues/47)) ([fb83d38](https://github.com/0x67/observability/commit/fb83d389db7d8142791010548197e1b47cdcd0ed))


### Performance

* **core:** store latency histograms at 2 significant figures ([#50](https://github.com/0x67/observability/issues/50)) ([c94322d](https://github.com/0x67/observability/commit/c94322deb0de48b723fc46ce71bc04b0d558582d))


### Refactor

* **core:** move gate, timer, sampler and field keys into modules ([#41](https://github.com/0x67/observability/issues/41)) ([e0cbf01](https://github.com/0x67/observability/commit/e0cbf01d70e1d4576f76c57546993d1d9d23c13e))
* **core:** move hand-off and flusher into modules ([#42](https://github.com/0x67/observability/issues/42)) ([0e000a7](https://github.com/0x67/observability/commit/0e000a792852676de2cec59c7d86585a1b521a83))
* **core:** read the metrics gate from one atomic ([#49](https://github.com/0x67/observability/issues/49)) ([511734b](https://github.com/0x67/observability/commit/511734b8ef8c5862611decbbe2e5a6459d149203))


### Documentation

* **changelog:** point links at 0x67/observability ([#18](https://github.com/0x67/observability/issues/18)) ([e659138](https://github.com/0x67/observability/commit/e659138a37eb49a3d19cea04c36db4edc85e856e))
* **core:** record hot-path numbers after the memory fixes ([#48](https://github.com/0x67/observability/issues/48)) ([4b01192](https://github.com/0x67/observability/commit/4b01192a313e59e843d37a6c5259f5ca6d06dc6d))


### Tests

* **core:** add std-only hot-path bench ([#43](https://github.com/0x67/observability/issues/43)) ([6dce7fe](https://github.com/0x67/observability/commit/6dce7fe70713e0d51961ebe3e89d13b0f26e0f6f))
* **core:** fail when the hot path allocates ([#46](https://github.com/0x67/observability/issues/46)) ([a85bf29](https://github.com/0x67/observability/commit/a85bf295aa8a49ee4b04beeaaec93d849f247262))
* **docs:** compile readme snippets as doc-tests ([#55](https://github.com/0x67/observability/issues/55)) ([73c316f](https://github.com/0x67/observability/commit/73c316f2587bf8f4665a0b0693608a82fd3143e8))


### Build

* **core:** narrow tokio features behind flush-tokio ([#44](https://github.com/0x67/observability/issues/44)) ([067b057](https://github.com/0x67/observability/commit/067b057e0b72a9f8209962b03d2e669ad1d33d01))


### CI

* **release:** refresh cargo.lock and bump readme tags in release prs ([#82](https://github.com/0x67/observability/issues/82)) ([6c7f21a](https://github.com/0x67/observability/commit/6c7f21a6ee725b6e61d90c8ea92a859550682271))

## [0.2.1](https://github.com/0x67/observability/compare/observability-core-v0.2.0...observability-core-v0.2.1) (2026-07-11)


### Documentation

* update README ([#5](https://github.com/0x67/observability/issues/5)) ([8821540](https://github.com/0x67/observability/commit/8821540dc3681216b18b31cdd61e500ade49f5fc))

## [0.2.0](https://github.com/0x67/observability/compare/observability-core-v0.1.0...observability-core-v0.2.0) (2026-07-11)


### Features

* **core:** runtime metrics gate and hot-path telemetry helpers ([e48dc63](https://github.com/0x67/observability/commit/e48dc63eca88f7fc17ebedd493d8f639a6a7205d))
