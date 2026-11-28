# Changelog

## [0.3.1](https://github.com/0x67/observability/compare/observability-v0.3.0...observability-v0.3.1) (2026-11-28)


### Documentation

* **readme:** add operational notes and outage guidance ([#85](https://github.com/0x67/observability/issues/85)) ([4123114](https://github.com/0x67/observability/commit/4123114e0dcd557781b8627ebb56f0d1bb55bb3d))


### Tests

* **observability:** stop checking the core cell in the compat table ([#83](https://github.com/0x67/observability/issues/83)) ([0d13faa](https://github.com/0x67/observability/commit/0d13faa01e6b54788a2a7b23b88ffe07863dd92f))

## [0.3.0](https://github.com/0x67/observability/compare/observability-v0.2.2...observability-v0.3.0) (2026-11-27)


### ⚠ BREAKING CHANGES

* **otlp:** require an http or https endpoint and set the export protocol in code ([#71](https://github.com/0x67/observability/issues/71))
* **otlp:** reject user credentials in the otlp endpoint ([#69](https://github.com/0x67/observability/issues/69))
* **metrics:** export bucketed histograms with a service label and idle expiry ([#66](https://github.com/0x67/observability/issues/66))
* **pipeline:** bound the otlp log queue and allow one otlp log sink ([#65](https://github.com/0x67/observability/issues/65))
* **tracing:** replace TraceExporter with a span level and a sample ratio ([#60](https://github.com/0x67/observability/issues/60))
* **config:** add defaults and take headers and resource attributes as maps ([#57](https://github.com/0x67/observability/issues/57))
* **otlp:** send each http signal to its own path under the endpoint ([#56](https://github.com/0x67/observability/issues/56))
* **core:** read the metrics gate from one atomic ([#49](https://github.com/0x67/observability/issues/49))
* **pipeline:** return an error for an unwritable log dir and add file retention ([#33](https://github.com/0x67/observability/issues/33))
* **otlp:** add tls with native or bundled webpki roots ([#32](https://github.com/0x67/observability/issues/32))
* **pipeline:** return errors for config the build cannot honor ([#31](https://github.com/0x67/observability/issues/31))

### Features

* **config:** add defaults and take headers and resource attributes as maps ([#57](https://github.com/0x67/observability/issues/57)) ([d2e66d0](https://github.com/0x67/observability/commit/d2e66d0f1e4eeb30c8af44cf58e1c1aeef6ad0ec))
* **config:** read the config from files behind a serde feature ([#58](https://github.com/0x67/observability/issues/58)) ([3fe7ffa](https://github.com/0x67/observability/commit/3fe7ffa57e8d566eded46e337d17fc3cf810eabc))
* **metrics:** export bucketed histograms with a service label and idle expiry ([#66](https://github.com/0x67/observability/issues/66)) ([be8429f](https://github.com/0x67/observability/commit/be8429f4ab45288468b693677c89338e319c3a9b))
* **metrics:** replace metrics-exporter-otel with an in-crate otel recorder ([#74](https://github.com/0x67/observability/issues/74)) ([a35deae](https://github.com/0x67/observability/commit/a35deaed8ec94b8bfa9a4d4d97c6e388f0503295))
* **otlp:** add the os platform verifier as a tls roots option ([#39](https://github.com/0x67/observability/issues/39)) ([6ac6636](https://github.com/0x67/observability/commit/6ac66366a4b6dcbaef0942e63330437aae5d175a))
* **otlp:** add tls with native or bundled webpki roots ([#32](https://github.com/0x67/observability/issues/32)) ([f8534d3](https://github.com/0x67/observability/commit/f8534d3a620c9e722911aca751107a67dc3e7361))
* **otlp:** trust a custom ca file and present a client certificate ([#36](https://github.com/0x67/observability/issues/36)) ([85bb94d](https://github.com/0x67/observability/commit/85bb94d23ba292e091b99d38bfb24ab40760e1ee))
* **pipeline:** bound log sink queues and count dropped lines ([#62](https://github.com/0x67/observability/issues/62)) ([2ba678d](https://github.com/0x67/observability/commit/2ba678d60eb8f791aba19df601975c101f091c81))
* **pipeline:** bound the otlp log queue and allow one otlp log sink ([#65](https://github.com/0x67/observability/issues/65)) ([8ddd3b4](https://github.com/0x67/observability/commit/8ddd3b416e7cb97bd0cd75aee0a1f63aaf06046b))
* **pipeline:** bound the span export queue and count dropped spans ([#63](https://github.com/0x67/observability/issues/63)) ([8db8a42](https://github.com/0x67/observability/commit/8db8a426a273997872c46a61c30614141ae2dd3e))
* **pipeline:** return an error for an unwritable log dir and add file retention ([#33](https://github.com/0x67/observability/issues/33)) ([4603d92](https://github.com/0x67/observability/commit/4603d928e484a4aaa64dac6d9518496629fcd0fd))
* **tracing:** propagate w3c trace context through string headers ([#61](https://github.com/0x67/observability/issues/61)) ([88093d2](https://github.com/0x67/observability/commit/88093d20594b4de7d5547ed25182adcdfbeaca9f))
* **tracing:** replace TraceExporter with a span level and a sample ratio ([#60](https://github.com/0x67/observability/issues/60)) ([44e0571](https://github.com/0x67/observability/commit/44e05712433cdf2bcc14d9c9e81c8a11584d05ac))


### Bug fixes

* **observability:** flush guards on any runtime and drop metrics before the pipeline ([#38](https://github.com/0x67/observability/issues/38)) ([43159e2](https://github.com/0x67/observability/commit/43159e267dbf0f86b8c330b6b09e4ec47bd82e44))
* **observability:** mark the guards must_use ([#35](https://github.com/0x67/observability/issues/35)) ([0264798](https://github.com/0x67/observability/commit/02647987b725b2e3c1fa58247783e1fa99302929))
* **otlp:** hide header values in the otlp config debug output ([#26](https://github.com/0x67/observability/issues/26)) ([0e7e37a](https://github.com/0x67/observability/commit/0e7e37a2d816324933e7dd02077b6caad7c39312))
* **otlp:** reject user credentials in the otlp endpoint ([#69](https://github.com/0x67/observability/issues/69)) ([ab8da55](https://github.com/0x67/observability/commit/ab8da55d17cb52f23329a60f02db4b74a3a5da32))
* **otlp:** require an http or https endpoint and set the export protocol in code ([#71](https://github.com/0x67/observability/issues/71)) ([17950b9](https://github.com/0x67/observability/commit/17950b943c4f6d7388366ad43a1f3ddbab0bf684))
* **otlp:** run grpc and http export on a crate-owned tokio runtime ([#28](https://github.com/0x67/observability/issues/28)) ([e622ffa](https://github.com/0x67/observability/commit/e622ffae1581b89c3eae665dddd4b35c28de488a))
* **otlp:** send each http signal to its own path under the endpoint ([#56](https://github.com/0x67/observability/issues/56)) ([0247ea1](https://github.com/0x67/observability/commit/0247ea175f215bc33fd35aa5c76773af7b05b535))
* **otlp:** send headers as grpc metadata and reject invalid headers ([#34](https://github.com/0x67/observability/issues/34)) ([6ce7629](https://github.com/0x67/observability/commit/6ce7629b81b065f981a4fc58773ffec7effe66cf))
* **pipeline:** filter the otel layers by level and exporter thread ([#30](https://github.com/0x67/observability/issues/30)) ([669cc26](https://github.com/0x67/observability/commit/669cc269fcd880a01642815572f505c95d941f16))
* **pipeline:** return errors for config the build cannot honor ([#31](https://github.com/0x67/observability/issues/31)) ([0fcc8a2](https://github.com/0x67/observability/commit/0fcc8a223c2c8d92c366176f6665f78ed8c641ee))


### Refactor

* **core:** read the metrics gate from one atomic ([#49](https://github.com/0x67/observability/issues/49)) ([511734b](https://github.com/0x67/observability/commit/511734b8ef8c5862611decbbe2e5a6459d149203))
* **metrics:** move metrics.rs into a metrics module directory ([#54](https://github.com/0x67/observability/issues/54)) ([c1abb33](https://github.com/0x67/observability/commit/c1abb33ed9d30819b280200c9d629685bf3d1983))
* **observability:** split pipeline and otlp into module directories ([#27](https://github.com/0x67/observability/issues/27)) ([21a8eb1](https://github.com/0x67/observability/commit/21a8eb1994dc17c8fdce412c96c8d832e821d2e3))


### Documentation

* **changelog:** point links at 0x67/observability ([#18](https://github.com/0x67/observability/issues/18)) ([e659138](https://github.com/0x67/observability/commit/e659138a37eb49a3d19cea04c36db4edc85e856e))
* **examples:** add tokio quickstart example ([#73](https://github.com/0x67/observability/issues/73)) ([5936b3e](https://github.com/0x67/observability/commit/5936b3e21e748ede4f7b032756c4ef7e260e5fdb))
* **observability:** add redaction guidance and a collector scrubbing config ([#70](https://github.com/0x67/observability/issues/70)) ([b4d0f60](https://github.com/0x67/observability/commit/b4d0f6084659c79211b0c602383d2556a384298a))
* **observability:** describe the 0.33 stack, retry, tls provider and msrv scan ([#78](https://github.com/0x67/observability/issues/78)) ([6043151](https://github.com/0x67/observability/commit/604315106949bd4470da3e051c090a1d0b2df4c0))
* **observability:** document config files, delivery limits, and propagation ([#67](https://github.com/0x67/observability/issues/67)) ([8a81576](https://github.com/0x67/observability/commit/8a8157609e2375e2ad380baf73cc463e588d416e))
* **observability:** document the export runtime, tls, and the init contract ([#40](https://github.com/0x67/observability/issues/40)) ([df3f5bc](https://github.com/0x67/observability/commit/df3f5bcb298690cbdba0723a43e15fc87454c903))
* **readme:** add upstream compatibility table and lockfile drift test ([#81](https://github.com/0x67/observability/issues/81)) ([32c41ea](https://github.com/0x67/observability/commit/32c41ea32f4349e7d57c4a5ae78d07db64273080))


### Tests

* **docs:** compile readme snippets as doc-tests ([#55](https://github.com/0x67/observability/issues/55)) ([73c316f](https://github.com/0x67/observability/commit/73c316f2587bf8f4665a0b0693608a82fd3143e8))
* **observability:** integration tests for init and otlp http export ([#80](https://github.com/0x67/observability/issues/80)) ([eb56483](https://github.com/0x67/observability/commit/eb564833508ff73e66ce8e39d218cfed053a0e92))
* **otlp:** add tls modes and a generated pki to the mock collectors ([#37](https://github.com/0x67/observability/issues/37)) ([afa0b90](https://github.com/0x67/observability/commit/afa0b903469e38641b4fd0dce0c419064ad557d9))
* **otlp:** cover export retry on http and grpc ([#76](https://github.com/0x67/observability/issues/76)) ([5a37277](https://github.com/0x67/observability/commit/5a37277d367eae2e2d07d4b696d280ec6875779a))


### Build

* **deps:** drop unused rt-tokio feature and tokio dev-dependency ([#19](https://github.com/0x67/observability/issues/19)) ([13824b0](https://github.com/0x67/observability/commit/13824b071e1214526049dee1e9127e432d112485))
* **deps:** move metrics-exporter-prometheus to 0.18.3 ([#77](https://github.com/0x67/observability/issues/77)) ([fabbaf1](https://github.com/0x67/observability/commit/fabbaf1b841ff878693e64708419fe1cc4f14419))
* **deps:** move the opentelemetry stack to 0.33 and tracing-opentelemetry to 0.34 ([#75](https://github.com/0x67/observability/issues/75)) ([24a2772](https://github.com/0x67/observability/commit/24a27721d2a90aa6be288355b6ca9d50a1204211))

## [0.2.2](https://github.com/0x67/observability/compare/observability-v0.2.1...observability-v0.2.2) (2026-07-16)


### Bug fixes

* **pipeline:** honor RUST_LOG over config level in env filters ([#7](https://github.com/0x67/observability/issues/7)) ([7ac0c7f](https://github.com/0x67/observability/commit/7ac0c7facdcfdf96e82d8f09aae7c75b8c04e6cd))

## [0.2.1](https://github.com/0x67/observability/compare/observability-v0.2.0...observability-v0.2.1) (2026-07-11)


### Documentation

* update README ([#5](https://github.com/0x67/observability/issues/5)) ([8821540](https://github.com/0x67/observability/commit/8821540dc3681216b18b31cdd61e500ade49f5fc))

## [0.2.0](https://github.com/0x67/observability/compare/observability-v0.1.0...observability-v0.2.0) (2026-07-11)


### Features

* **backend:** tracing pipeline and metrics recorder over the gate ([745fd8e](https://github.com/0x67/observability/commit/745fd8e82882d39955ab8235bb56adad1189bfcb))


### Tests

* public-api otel-leak guard and ci feature matrix ([e7c8348](https://github.com/0x67/observability/commit/e7c834842d251cf29e9e2acc686501c130dc2827))


### CI

* **observability:** replace hand-listed feature builds with cargo-hack… ([#3](https://github.com/0x67/observability/issues/3)) ([9e364cb](https://github.com/0x67/observability/commit/9e364cb4ec0c3135f0d7427fb79beaefe3a14c33))
