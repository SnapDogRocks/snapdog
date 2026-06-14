# Changelog

## [0.19.0](https://github.com/SnapDogRocks/snapdog/compare/v0.18.0...v0.19.0) (2026-06-14)


### Features

* **demo-backend:** add express-rate-limit middleware and configure trust proxy ([9615a80](https://github.com/SnapDogRocks/snapdog/commit/9615a80d0c9b12c99f78ba25ceef51885a7065f2))
* **demo-backend:** expand zones with premium clients and update client count ([7901ec9](https://github.com/SnapDogRocks/snapdog/commit/7901ec9896d6579dfa783340a78da1214fdcaac5))


### Bug Fixes

* **demo:** disable strict parsing in express json to support primitives ([75ee0f1](https://github.com/SnapDogRocks/snapdog/commit/75ee0f12b051cf36289e2b2447b3fc4c751e0448))

## [0.18.0](https://github.com/SnapDogRocks/snapdog/compare/v0.17.0...v0.18.0) (2026-06-13)


### Features

* add unified snapdog mock demo backend for Render deployment ([a464a18](https://github.com/SnapDogRocks/snapdog/commit/a464a18bfc1a3be4191437d7b8b3a86119f2e227))
* make cover art 1:1 aspect-ratio, update client zone routing to match production API ([3cc677f](https://github.com/SnapDogRocks/snapdog/commit/3cc677f6f7b2c86ad51efc43ea9ae176e0c1ad02))
* set initial mock zone presence to false ([d89398b](https://github.com/SnapDogRocks/snapdog/commit/d89398b7b5546a711ea4238ba7587ae7f9f35d43))
* **webui:** premium multi-zone layout refinements and EQ presets ([10d5143](https://github.com/SnapDogRocks/snapdog/commit/10d5143633c5ebe3d55c7a50a882645bf1211a67))


### Bug Fixes

* **demo:** resolve syntax error in zone EQ preset endpoint ([3463a7f](https://github.com/SnapDogRocks/snapdog/commit/3463a7ff1e898e83f1ead95a2d3a2bb259688a0f))
* **webui:** update github repository link in AboutButton ([9b1a833](https://github.com/SnapDogRocks/snapdog/commit/9b1a833d6e1b12d5878f0448651bf9b8f5ebc014))

## [0.17.0](https://github.com/SnapDogRocks/snapdog/compare/v0.16.1...v0.17.0) (2026-06-06)


### ⚠ BREAKING CHANGES

* **ws:** rename playlist fields, add playlist navigation

### Features

* annotate system status endpoints and register ApiDoc OpenAPI specs ([3f42400](https://github.com/SnapDogRocks/snapdog/commit/3f42400a7c743092f20998a0542b0dddc5564756))
* **api:** add PlayPlaylist command for explicit playlist playback ([61de429](https://github.com/SnapDogRocks/snapdog/commit/61de429fb28e37b03ad8004a704a350111dc08e7))
* **api:** add relative seek (offset_ms) to position endpoint ([86c2361](https://github.com/SnapDogRocks/snapdog/commit/86c2361281627155523ca8ccd5c15c54caf116e8))
* **api:** catch audio decoding errors and propagate via websockets ([dee5e0b](https://github.com/SnapDogRocks/snapdog/commit/dee5e0bb2735bca68854435070b4b09b89b2bc03))
* **api:** embed Scalar documentation behind api-docs compile feature ([dbc2d29](https://github.com/SnapDogRocks/snapdog/commit/dbc2d29e5c234a304835492bec682699e20010eb))
* **client/dbus:** complete MPRIS2 wiring with metadata and controls ([28f68da](https://github.com/SnapDogRocks/snapdog/commit/28f68da2c6bafffc73f1acccb21160f79b9c3663))
* **client/dbus:** MPRIS2 Volume interface for snapdog-client ([ade5019](https://github.com/SnapDogRocks/snapdog/commit/ade5019356746121d02f633d4f71e30f34fbd461))
* **client:** receive Type 14 metadata, send Type 13 controls via D-Bus ([37475fe](https://github.com/SnapDogRocks/snapdog/commit/37475fe81e1b74c594f7dfa54d0abed416f2fa95))
* **common:** define Custom Message types 13/14/15 ([85f02f9](https://github.com/SnapDogRocks/snapdog/commit/85f02f9fbcde3dca4e5ecd886805849b04aeef26))
* **config:** add api_docs TOML key to dynamically toggle Scalar API docs route ([317f95f](https://github.com/SnapDogRocks/snapdog/commit/317f95f598feab430a94848894ac0410e0bdeedc))
* **dbus:** MPRIS2 interface per zone ([b8c350f](https://github.com/SnapDogRocks/snapdog/commit/b8c350f2a324d38c60fa3a1c191f7272d58b5193))
* implement gen-api-spec subcommand in xtask tool ([4083129](https://github.com/SnapDogRocks/snapdog/commit/40831299958f7630a226daa53d9c051845def128))
* server sends Type 14 metadata to snapdog clients, receives Type 13 controls ([cea7389](https://github.com/SnapDogRocks/snapdog/commit/cea7389daa167e924818bf52deb0cd14956b4873))
* Type 15 cover art binary push to snapdog clients ([5cb4bce](https://github.com/SnapDogRocks/snapdog/commit/5cb4bcec0d1be107f81b4c99bee6e29445434e26))
* **webui:** handle websocket playback errors and manage store error state ([28d4ea7](https://github.com/SnapDogRocks/snapdog/commit/28d4ea77cb97c49a8111635944370636b28a726f))
* **webui:** render warning banner in zone detail and error icon in sidebar ([f982c80](https://github.com/SnapDogRocks/snapdog/commit/f982c804c8b7f210afe28711295330263e23ebe8))
* **webui:** use server-provided can_next/can_prev for nav buttons ([27fe953](https://github.com/SnapDogRocks/snapdog/commit/27fe953481ba0510a4c73f580ca72c582c8bf35a))
* **ws:** add metadata and navigation fields to ZoneTrackChanged ([fe9f0da](https://github.com/SnapDogRocks/snapdog/commit/fe9f0dab23b536219a23758f1f459b2435bf77b2))


### Bug Fixes

* **dbus:** only attempt D-Bus on Linux (macOS has no D-Bus) ([14ab5df](https://github.com/SnapDogRocks/snapdog/commit/14ab5df47aa9fd5b716b22864bba6926590b5284))
* **player:** Apple UX for playlist switching ([1cf6318](https://github.com/SnapDogRocks/snapdog/commit/1cf6318cdd027c910d5cdc1851c1c4997ab1ae28))
* **player:** track playlist state on auto-advance and radio next ([12e3b1a](https://github.com/SnapDogRocks/snapdog/commit/12e3b1aaa0d7e7c67affa99f5bc239951e177a59))
* **webui:** wrap seek position value in SeekPayload object ([4b09f46](https://github.com/SnapDogRocks/snapdog/commit/4b09f46cbe02ac7af81119d2727cd543b0e4bcc8))


### Code Refactoring

* **ws:** rename playlist fields, add playlist navigation ([ccab89a](https://github.com/SnapDogRocks/snapdog/commit/ccab89ad8df09ae9dd16c0e29f960ac98fa5f76e))

## [0.16.1](https://github.com/SnapDogRocks/snapdog/compare/v0.16.0...v0.16.1) (2026-05-30)


### Bug Fixes

* **ci:** use lowercase org name for Docker image tags ([b627dcb](https://github.com/SnapDogRocks/snapdog/commit/b627dcb09262e7a0a1dba8375b85937a4522bb78))

## [0.16.0](https://github.com/metaneutrons/snapdog/compare/v0.15.0...v0.16.0) (2026-05-29)


### Features

* **config:** add validate() for cross-field constraints ([c168c10](https://github.com/metaneutrons/snapdog/commit/c168c10a430c0aebe95f8ddb4725e55979ea662b))
* **config:** validate audio ranges, port collision, max_volume ([20c3769](https://github.com/metaneutrons/snapdog/commit/20c3769506124597a8a99e1513f5f6c2e9f04ffb))


### Bug Fixes

* **config:** audit fixes for TOML/struct consistency ([fe33643](https://github.com/metaneutrons/snapdog/commit/fe33643b282c7635b9599fbdfb9699c1e5c0e6c8))
* **config:** clamp fade durations to 1000ms maximum ([16bc14f](https://github.com/metaneutrons/snapdog/commit/16bc14f574305736fd5a822dc00ec62ca77b5e36))
* **config:** uncomment required client name in example config ([6a87b39](https://github.com/metaneutrons/snapdog/commit/6a87b39b80e4b239c71a3cf2c9d45a2425896f6b))

## [0.15.0](https://github.com/metaneutrons/snapdog/compare/v0.14.2...v0.15.0) (2026-05-24)


### Features

* **http:** TLS support for REST API and WebUI ([4a9d5d3](https://github.com/metaneutrons/snapdog/commit/4a9d5d3d6203f25e4abbc5f4f54acdafa015427e))
* **mdns:** add persistent server_id to TXT records ([f50f69c](https://github.com/metaneutrons/snapdog/commit/f50f69c0577ee2899f03f951fdac06a077481b70))
* **mdns:** auto-derive base_url from bind address ([6ce913f](https://github.com/metaneutrons/snapdog/commit/6ce913fffe19ceeb68438486d32609797a360ee7))
* **webui:** QR code connect dialog with share link ([06720df](https://github.com/metaneutrons/snapdog/commit/06720df99d9dce6c161de199466208247b7522ea))
* **webui:** show custom server name above zones in sidebar ([5ece34c](https://github.com/metaneutrons/snapdog/commit/5ece34c5451ca8306fdf41de79b006eaf81c9e23))
* **webui:** show server name in title, about dialog, and sidebar ([4712681](https://github.com/metaneutrons/snapdog/commit/471268122fa6e179b7d4916b33ef17d2b5f7ef03))


### Bug Fixes

* **ci:** correct YAML structure for concurrency block ([ae75d50](https://github.com/metaneutrons/snapdog/commit/ae75d50d0e2af0999c54f0c7dc05a320e6563d4a))
* **packaging:** add avahi as runtime dependency ([5daaf72](https://github.com/metaneutrons/snapdog/commit/5daaf725c407c8f63675c48ddac793ca084f32a9))
* **webui:** increase header icon to size-8, fix locale menu clipping ([f39d896](https://github.com/metaneutrons/snapdog/commit/f39d896f66b548d57d0c2cabe5768240f9a65ec5))
* **webui:** resolve ESLint setState-in-effect error ([acb2651](https://github.com/metaneutrons/snapdog/commit/acb265124b66fb8972acf6f21b514a3cc00e3ba4))

## [0.14.2](https://github.com/metaneutrons/snapdog/compare/v0.14.1...v0.14.2) (2026-05-24)


### Bug Fixes

* **docker:** use webui/out path for radio-cover.png include_bytes ([2006a48](https://github.com/metaneutrons/snapdog/commit/2006a48b586d6b0315abce1d2236cd191cd838de))
* move radio-cover.png to assets/ for include_bytes ([2d030c4](https://github.com/metaneutrons/snapdog/commit/2d030c4a55515a474155331c86ff2d9314e7da97))
* **webui:** remove overflow-hidden from glow container (Safari artifact) ([1bcd250](https://github.com/metaneutrons/snapdog/commit/1bcd250e6ffa91d617e91cbaf2637736d5aa95d5))

## [0.14.1](https://github.com/metaneutrons/snapdog/compare/v0.14.0...v0.14.1) (2026-05-24)


### Bug Fixes

* **docker:** add libavahi-compat-libdnssd-dev to build stages, libavahi-client3 to runtime ([283d7f5](https://github.com/metaneutrons/snapdog/commit/283d7f53e0aff73328a2c95aaf3ee6a98c3e9361))

## [0.14.0](https://github.com/metaneutrons/snapdog/compare/v0.13.0...v0.14.0) (2026-05-24)


### Features

* **airplay:** runtime protocol mode selection, default pairing persistence ([29863fd](https://github.com/metaneutrons/snapdog/commit/29863fd080f35cd6e8651205728939fff77749cb))
* **airplay:** runtime protocol mode selection, default pairing persistence ([a06c2b7](https://github.com/metaneutrons/snapdog/commit/a06c2b71c09bf962766f1b95398a831bdc01c28a))
* **client:** discover _snapdog._tcp via mDNS with TXT records ([7bab448](https://github.com/metaneutrons/snapdog/commit/7bab448b6f548a61fe52be9cdd2904d1e301e783))
* **mdns:** enterprise-grade mDNS with docker detection ([669beec](https://github.com/metaneutrons/snapdog/commit/669beecfaf87382b3181f0d50b2289de391489c7))
* **mdns:** upgrade snapcast-rs to 0.14 (OS-native mDNS) ([41704b5](https://github.com/metaneutrons/snapdog/commit/41704b5b43486818d9befb01aebe77991376df05))


### Bug Fixes

* **api:** serve radio-cover.png for /playlists/0/cover ([72c8811](https://github.com/metaneutrons/snapdog/commit/72c88118c29c022a5bd9208cc483395d4cbf8277))
* **webui:** cover glow blocking playlist back button ([0593618](https://github.com/metaneutrons/snapdog/commit/0593618c84c8a3b15f575ba6755529d956e70f81))
* **webui:** move cover glow to ZoneDetail background layer ([a3e8f26](https://github.com/metaneutrons/snapdog/commit/a3e8f26870ea98175924beb360fee68002440a96))
* **webui:** stack PlaylistBrowser above cover glow to ensure back button clickability ([3f99b93](https://github.com/metaneutrons/snapdog/commit/3f99b933ae0b33c4af07bf2a388004f1df8f9336))

## [0.13.0](https://github.com/metaneutrons/snapdog/compare/v0.12.4...v0.13.0) (2026-05-23)


### Features

* **macos-helper:** add About SnapDog Server window ([77e4e25](https://github.com/metaneutrons/snapdog/commit/77e4e25d7f6258d9fd23e2fd5cec32152bc9dd07))
* **macos-helper:** add Subsonic enable/disable toggle in ConfigView ([a483a37](https://github.com/metaneutrons/snapdog/commit/a483a379a1b9727f6d5c4a157f3d9a49ab0eeecc))


### Bug Fixes

* **release:** set MARKETING_VERSION from release tag in macOS app build ([f7699c5](https://github.com/metaneutrons/snapdog/commit/f7699c5a9359fb9adb33fbd234160504b7edb29d))

## [0.12.4](https://github.com/metaneutrons/snapdog/compare/v0.12.3...v0.12.4) (2026-05-23)


### Bug Fixes

* **release:** fix Sparkle appcast R2 deployment and foreground activation focus ([16c8b1a](https://github.com/metaneutrons/snapdog/commit/16c8b1ab7162d09f4414893f3a8d8526049c9cf7))
* **release:** preserve manual Xcode project changes and user-defined AppIcon.icon ([75770fd](https://github.com/metaneutrons/snapdog/commit/75770fd073a045c75ae6e19f798995b6584bb223))
* **release:** resolve missing macOS app icon and dark DMG background issue ([4084a0b](https://github.com/metaneutrons/snapdog/commit/4084a0b1532e7cbc3edcffc86a1f6edfab253a8c))
* **webui:** keep sidebar visible at xl when 3+ zones overflow the grid ([36f4432](https://github.com/metaneutrons/snapdog/commit/36f44323a8fb7690333870826b3eddb2b1163616))

## [0.12.3](https://github.com/metaneutrons/snapdog/compare/v0.12.2...v0.12.3) (2026-05-23)


### Bug Fixes

* **snapcast:** resolve double-lock deadlock in event handler when accepting unknown clients ([35159fa](https://github.com/metaneutrons/snapdog/commit/35159fa92fe862103ede2a21b0fd717cb6e8e9f7))

## [0.12.2](https://github.com/metaneutrons/snapdog/compare/v0.12.1...v0.12.2) (2026-05-23)


### Bug Fixes

* **ci:** fix rust format and webui react-hooks lint warning ([59d25ca](https://github.com/metaneutrons/snapdog/commit/59d25ca2b30d22a6f493ab574955199ac724f397))
* **deps:** upgrade shairplay crate to 0.3.1 ([66dc0fb](https://github.com/metaneutrons/snapdog/commit/66dc0fb1b7aa666bd56c75d0d5c5ea72693f4404))

## [0.12.1](https://github.com/metaneutrons/snapdog/compare/v0.12.0...v0.12.1) (2026-05-22)


### Bug Fixes

* fail fast with actionable error on port conflict at startup ([a5b373e](https://github.com/metaneutrons/snapdog/commit/a5b373ec4790e07ae1347a35872d4e219d9c79d9))

## [0.12.0](https://github.com/metaneutrons/snapdog/compare/v0.11.4...v0.12.0) (2026-05-22)


### Features

* add top-level `name` field — single server identity ([9309922](https://github.com/metaneutrons/snapdog/commit/9309922bd79e22b9c47b06142285986dff0ce8e8))
* redesign About view with responsive sheets, tactile swipe close, and hardware spec board ([be137b6](https://github.com/metaneutrons/snapdog/commit/be137b64381ba65a087f9cf96306bd7b01649dbd))
* **webui:** localize About view modal labels using next-intl ([33f93d5](https://github.com/metaneutrons/snapdog/commit/33f93d5f61cc7888f05aa22024cdb7c7ff1ae0f3))
* **webui:** redesign light theme with premium warm sand and oatmeal aesthetic ([2562daa](https://github.com/metaneutrons/snapdog/commit/2562daae4ede6c4fdb89efb339d3452f4dc28b2e))
* **webui:** remove redundant system status and display real connected hostname ([ad56cb3](https://github.com/metaneutrons/snapdog/commit/ad56cb35ce23aeb8a3ea9861bbb48f4d8e51359c))
* **webui:** rename firmware spec label to version ([fe8798d](https://github.com/metaneutrons/snapdog/commit/fe8798d7782c9c8be4d03a75ce69c5ba73dbc139))
* **webui:** translate tech telemetry labels for de, fr, es, nl ([c6c6c28](https://github.com/metaneutrons/snapdog/commit/c6c6c28d470ac1d8af160f60ad9b7bb7026be6ff))
* **webui:** update about dialog subclaim description with dynamic integration info ([ae240f0](https://github.com/metaneutrons/snapdog/commit/ae240f093d21a321547aa0ebcb9b1cc002658ea3))
* **webui:** update source link text to metaneutrons/snapdog ([cc469e3](https://github.com/metaneutrons/snapdog/commit/cc469e33aa96eb71180e6a8e6e0f807057eb45d3))


### Bug Fixes

* prevent About modal vertical overflow and ensure Done button visibility ([bd3b206](https://github.com/metaneutrons/snapdog/commit/bd3b2065aec2faf08dcebec85c6bc14d8d54790b))
* **webui:** resolve double-press theme toggle glitch and add premium micro-animations ([1a78921](https://github.com/metaneutrons/snapdog/commit/1a789218041e848cee0514e3e8d04bcbe55231d4))
* **webui:** resolve playback detail layout shifts and enhance EQ UX ([46a7205](https://github.com/metaneutrons/snapdog/commit/46a7205fae2ac9b86e0fb4458e043ba4dad6032d))

## [0.11.4](https://github.com/metaneutrons/snapdog/compare/v0.11.3...v0.11.4) (2026-05-21)


### Bug Fixes

* **client:** adapt to sd-notify 0.5 API (removed unset_environment param) ([841911d](https://github.com/metaneutrons/snapdog/commit/841911de8145f8117bbc5fe694b93e0f6fa50545))
* move use statement before code (items_after_statements) ([a806528](https://github.com/metaneutrons/snapdog/commit/a806528039a760763f3775b102bc3dc898f45f6b))

## [0.11.3](https://github.com/metaneutrons/snapdog/compare/v0.11.2...v0.11.3) (2026-05-19)


### Performance Improvements

* tighten lock scopes — fix significant_drop_tightening (15 fixes) ([691a304](https://github.com/metaneutrons/snapdog/commit/691a304b345b0217a5ecfb86892f31e08bde7165))

## [0.11.2](https://github.com/metaneutrons/snapdog/compare/v0.11.1...v0.11.2) (2026-05-19)


### Bug Fixes

* **release:** skip post-build binary copy on CI (injected by workflow) ([46e35cd](https://github.com/metaneutrons/snapdog/commit/46e35cdec041d8c39072eb626244869cea66b402))

## [0.11.1](https://github.com/metaneutrons/snapdog/compare/v0.11.0...v0.11.1) (2026-05-19)


### Bug Fixes

* **release:** retry artifact download for macOS DMG build ([5af5262](https://github.com/metaneutrons/snapdog/commit/5af5262b46754efdbc06c7d0e24877a35d32d95b))

## [0.11.0](https://github.com/metaneutrons/snapdog/compare/v0.10.0...v0.11.0) (2026-05-18)


### Features

* **client:** allow bare host:port without tcp:// prefix ([75b8c64](https://github.com/metaneutrons/snapdog/commit/75b8c6447d8fd75c5ff040a88c5225b733a6f2f0))


### Performance Improvements

* buffer progress throttle 500ms (2x/sec) ([2aa3c42](https://github.com/metaneutrons/snapdog/commit/2aa3c42daf38683978c57a673724b440561e6ec2))
* throttle buffer progress WebSocket updates to 1x/second ([801c8ac](https://github.com/metaneutrons/snapdog/commit/801c8acc04d8ba3f137b660bd59ca058e017e98e))

## [0.10.0](https://github.com/metaneutrons/snapdog/compare/v0.9.3...v0.10.0) (2026-05-17)


### Features

* add --bind CLI flag and [http].bind config for listen address ([d2ec70f](https://github.com/metaneutrons/snapdog/commit/d2ec70f5989868ffc7f96912b9a223edf906dc9e))
* **macos-helper:** read port from config for Open WebUI, validate binary exists ([7df0b27](https://github.com/metaneutrons/snapdog/commit/7df0b27e3a620776671e1160d38a3ea9f6b2ac68))
* **webui:** empty state with KNX programming mode toggle ([a0d3ea2](https://github.com/metaneutrons/snapdog/commit/a0d3ea2d919228ec004adfd0ac4f48c625b9fa05))
* **webui:** package updates, eslint config, UX fixes ([00ec689](https://github.com/metaneutrons/snapdog/commit/00ec68916d10da5f1df365c0a187acb936598131))


### Bug Fixes

* allow KNX device mode without zones/clients, default bind to :: ([de3f144](https://github.com/metaneutrons/snapdog/commit/de3f144af09f02b5494780240ca675386539a07f))
* human-friendly API listen log (clickable localhost URL) ([8ea09a6](https://github.com/metaneutrons/snapdog/commit/8ea09a60316c925928ebf8e4222d411c47c1dea6))
* pass base_url to MqttBridge::connect in integration tests ([a5bab6a](https://github.com/metaneutrons/snapdog/commit/a5bab6aa791d9d65f3a7adaddf618b2b1e518211))
* **player:** respect pause state on prev/next — Apple Music UX ([f4d9d16](https://github.com/metaneutrons/snapdog/commit/f4d9d16f0cba4041736d82e5daea322b53ff8e11))
* **player:** set playback=Playing on prev/next track change ([02bb256](https://github.com/metaneutrons/snapdog/commit/02bb2561a275b72a39daeab03ad8bb7cdf36cc0b))
* serve placeholder cover art as pre-rendered 1024px PNG ([d945ff8](https://github.com/metaneutrons/snapdog/commit/d945ff81d7bc026509b314a515a235087c104a9d))
* serve radio playlist cover as 1024px PNG instead of SVG ([fc3cfc9](https://github.com/metaneutrons/snapdog/commit/fc3cfc92a43351faa95a726ff9d6ce77a47a3086))

## [0.9.3](https://github.com/metaneutrons/snapdog/compare/v0.9.2...v0.9.3) (2026-05-16)


### Bug Fixes

* **release:** remove xcodegen from CI — xcodeproj is committed ([22baa6d](https://github.com/metaneutrons/snapdog/commit/22baa6df834e4e43c72110d2ca1bbc37d3872687))

## [0.9.2](https://github.com/metaneutrons/snapdog/compare/v0.9.1...v0.9.2) (2026-05-16)


### Bug Fixes

* **release:** decouple macOS DMG from upload — AUR/Homebrew no longer blocked by DMG failure ([6f66390](https://github.com/metaneutrons/snapdog/commit/6f66390037a7c6a43234f7be6410f28ffe4ed600))
* **release:** fix universal binary find pattern for versioned tar directories ([5ff33fe](https://github.com/metaneutrons/snapdog/commit/5ff33fe828b9ba74dd9d5ed5db5e99ca5b548d68))

## [0.9.1](https://github.com/metaneutrons/snapdog/compare/v0.9.0...v0.9.1) (2026-05-16)


### Bug Fixes

* **release:** Docker missing assets/, KNX skip webui, Windows sha256sum ([9a412b2](https://github.com/metaneutrons/snapdog/commit/9a412b28578256274c49838aebd29183d4de149d))

## [0.9.0](https://github.com/metaneutrons/snapdog/compare/v0.8.0...v0.9.0) (2026-05-16)


### Features

* add macOS menu bar helper app (Swift 6, SwiftUI) ([e201f7b](https://github.com/metaneutrons/snapdog/commit/e201f7bcfb578b6ac3f5b0f562e3704d15609ea4))
* add radio playlist cover art placeholder (styled SVG) ([3950474](https://github.com/metaneutrons/snapdog/commit/3950474569087743ddfcb5abd327def07c3a81ee))
* add Windows target support (icon, version metadata, release + CI) ([39df9ce](https://github.com/metaneutrons/snapdog/commit/39df9cedbba5e082260796710438b39fc4cbff12))
* **api:** add subsonic single track playback endpoint ([753c496](https://github.com/metaneutrons/snapdog/commit/753c49647e6c5c2ba9b0ee9b741e42268b1bfa67))
* **api:** serve embedded SnapDog icon as placeholder when no cover art ([48486dc](https://github.com/metaneutrons/snapdog/commit/48486dcced0063426f00dc3023b3937c641c4714))
* **audio:** add decode_cached_file with symphonia seek support ([13b3846](https://github.com/metaneutrons/snapdog/commit/13b38464c4105dac96c213f26ebc34ae04c6c34e))
* **audio:** add decode_http_stream_cached — tees bytes to disk during streaming ([fc16b8c](https://github.com/metaneutrons/snapdog/commit/fc16b8c45467df7c7778fb915d2b94a999fcb9c9))
* **audio:** add pcm codec option for uncompressed streaming ([a7e2f70](https://github.com/metaneutrons/snapdog/commit/a7e2f70ceb15028eff519a59fbba2f6a5133bdba))
* **audio:** add PcmMessage::BufferProgress variant for cache progress ([647e884](https://github.com/metaneutrons/snapdog/commit/647e884e042ed851aea31537bba80390dd5700e4))
* **audio:** implement TrackCache with disk-backed LRU eviction ([c8a050f](https://github.com/metaneutrons/snapdog/commit/c8a050f090e335f51e3c27117c3a3df6e4a3cec5))
* **client:** add --test-tone flag to verify audio output ([9dd72d3](https://github.com/metaneutrons/snapdog/commit/9dd72d351a8f6dec8402dc732bcfb12a9d4e7bc3))
* **client:** dynamic format handling, device-probing, sd-notify, robust shutdown ([a23e70a](https://github.com/metaneutrons/snapdog/commit/a23e70ae9963403e9040f5e68d505f6577024525))
* **client:** dynamic format handling, sd-notify, resampler fallback ([608132e](https://github.com/metaneutrons/snapdog/commit/608132ef2d13a8a3464ff7117994fa681e2a049f))
* **client:** improve graceful and forced shutdown handling ([48cb01c](https://github.com/metaneutrons/snapdog/commit/48cb01ca597ebaafe8e98d8d1c64c7f493dcad67))
* **client:** log debug message on silence→signal transition ([6bd5d09](https://github.com/metaneutrons/snapdog/commit/6bd5d093c4fc14b389c82b4505358c47fd50d735))
* **client:** refactor shutdown for maximum robustness ([2536836](https://github.com/metaneutrons/snapdog/commit/25368362d122132c1dbb1b416a5a7f5c79de9bda))
* **config:** add SecretString for sensitive field masking ([7099e65](https://github.com/metaneutrons/snapdog/commit/7099e65184bced5d71549f10a951d56b25121c39))
* **config:** implement rigorous validation and env var overrides ([b36b4eb](https://github.com/metaneutrons/snapdog/commit/b36b4eb7fb031928d26440599be317469168fdf3))
* **eq:** interactive Logic Pro-style EQ curve with drag-to-delete ([e8793a1](https://github.com/metaneutrons/snapdog/commit/e8793a165c5a81b1f299b238ab2a4f9c34a8d7e7))
* **macos-helper:** add AppIcon (Apple Icon Composer format) ([42f1d83](https://github.com/metaneutrons/snapdog/commit/42f1d8377af23c7f70a2f1c2d2824f9d21ac0f20))
* **macos-helper:** add Sparkle auto-update framework ([4fe55bd](https://github.com/metaneutrons/snapdog/commit/4fe55bd98da30377be963410de6ada480a193955))
* **macos-helper:** add TOMLKit for proper config parsing and serialization ([7ed0fb5](https://github.com/metaneutrons/snapdog/commit/7ed0fb5bf2a6775b3b8f53760de43704b4ed3eda))
* **macos-helper:** auto-copy snapdog binary into app bundle (post-build script) ([da19f45](https://github.com/metaneutrons/snapdog/commit/da19f455874d9112b25680857bc5d925b418df47))
* **macos-helper:** auto-restart server on crash with exponential backoff ([7025af4](https://github.com/metaneutrons/snapdog/commit/7025af4885951526e926899442bed1de037d0d60))
* **macos-helper:** custom dog silhouette menu bar icon (template image) ([11b1c1c](https://github.com/metaneutrons/snapdog/commit/11b1c1cff8b84058e366ad2b431215bb315bfb00))
* **macos-helper:** show encryption PSK field when F32+LZ4 encrypted codec selected ([70dbba9](https://github.com/metaneutrons/snapdog/commit/70dbba9f7d77283e928dc7659473b8c3f28c22fb))
* **macos-helper:** show error in menu when server crashes (exit code != 0) ([9f839ca](https://github.com/metaneutrons/snapdog/commit/9f839ca1ff82e80d875ca092338c6fc32b04a33c))
* **macos-helper:** structured config editor with Apple UX ([37ec378](https://github.com/metaneutrons/snapdog/commit/37ec37866a09d31ee6418bdca95e2e24e50e8dfd))
* **macos-helper:** tap emoji field opens macOS Character Palette (emoji picker) ([0b79f0e](https://github.com/metaneutrons/snapdog/commit/0b79f0ef6ec64e5856027b381de17126b0a01b49))
* **media:** enable station-specific covers with favicon fallback for radio ([3ae05c0](https://github.com/metaneutrons/snapdog/commit/3ae05c0a68120a4c9e2ede5b8ca651f2c2e73307))
* **mqtt:** add configurable client_id (default: "snapdog") ([630205d](https://github.com/metaneutrons/snapdog/commit/630205d50326c20e2b01c64e099a0525a849052a))
* **mqtt:** add Last Will and Testament for offline availability ([7637444](https://github.com/metaneutrons/snapdog/commit/76374444ecf75b63a97abb4d10bd708770cc89bc))
* **mqtt:** Home Assistant MQTT Discovery + JSON state topics ([e970a61](https://github.com/metaneutrons/snapdog/commit/e970a6172714ac32f30743e5995a947d012cad2d))
* **player:** add prefetch_next_tracks for look-ahead cache warming ([f9b2c76](https://github.com/metaneutrons/snapdog/commit/f9b2c7681985c33cf5f5c4cedf1bb5af44baf237))
* **player:** use TrackCache for Subsonic playback, seek, and resume ([13fc86b](https://github.com/metaneutrons/snapdog/commit/13fc86b986eb88051c1eb888f9aa00765a257eb3))
* **radio:** favicon fallback for cover art with ICO→PNG conversion ([66ea530](https://github.com/metaneutrons/snapdog/commit/66ea53076693f98bae7d8089c8315d2e62260e18))
* **radio:** harden favicon extraction and manage fetch handles ([406f366](https://github.com/metaneutrons/snapdog/commit/406f366a7b67e52cf5cb8960542cd7491601f491))
* **release:** add codesigning and notarization for macOS app ([6fad672](https://github.com/metaneutrons/snapdog/commit/6fad67202b3523e54ab3190342f1ab2dcc1678b6))
* **release:** add Homebrew tap update for snapdog + snapdog-client ([3edd3ee](https://github.com/metaneutrons/snapdog/commit/3edd3ee7c35615be9021b54758e3db051dd1dfdf))
* **release:** add macOS app job — universal binary + .dmg packaging ([b27da25](https://github.com/metaneutrons/snapdog/commit/b27da252a1521159febea2a2bad8365df976a297))
* **release:** professional DMG with background and app-drop-link ([4d3a92b](https://github.com/metaneutrons/snapdog/commit/4d3a92b9412e10eebc48c2c549c93fc33febb090))
* **release:** serve Sparkle appcast via Cloudflare R2 ([f3caae2](https://github.com/metaneutrons/snapdog/commit/f3caae27a7b1ce217dfadaaf3d149ff2e143875f))
* **server:** implement robust graceful shutdown with watchdog timeout ([eb89e4d](https://github.com/metaneutrons/snapdog/commit/eb89e4df3b06e1bfbaa5947f4d01a3ddabd2085b))
* **state:** implement debounced auto-save for runtime changes ([ced09c0](https://github.com/metaneutrons/snapdog/commit/ced09c0045c9a94379cb4e2f4b6253d19e22c96c))
* **subsonic:** add track cache configuration (enabled, path, max_size_mb, lookahead) ([6b1021d](https://github.com/metaneutrons/snapdog/commit/6b1021dff64eb3673ea96be4440d14ff69070420))
* **system:** platform-aware state_dir for all persistent state files ([b44bc2f](https://github.com/metaneutrons/snapdog/commit/b44bc2f6505f6998431f186fe940c4dae755127e))
* **webui:** add About overlay with version, GitHub link, and license info ([b3da9dd](https://github.com/metaneutrons/snapdog/commit/b3da9dd616037b9319ab66357ec859f6aca84b56))
* **webui:** add build.rs to compile frontend during cargo build ([2cc503e](https://github.com/metaneutrons/snapdog/commit/2cc503e6a741904f29680aa1b04a5985a568492b))
* **webui:** add missing playlist i18n keys and refine grid typography ([d7384b4](https://github.com/metaneutrons/snapdog/commit/d7384b4b9c0c35a099de706ed3a16e536219bea7))
* **webui:** distinguish 'Server Restarting' from 'Connection Lost' on disconnect ([145c284](https://github.com/metaneutrons/snapdog/commit/145c284cbec08ad8c14f8fbcac700255bd95a46a))
* **webui:** improve transport control long-press reliability ([bb7c921](https://github.com/metaneutrons/snapdog/commit/bb7c9216d3ca10ef7ef558b2610e72f1c3e73de5))
* **webui:** inline playlist browser with search and show-more grid ([0503777](https://github.com/metaneutrons/snapdog/commit/050377789a775a3370410db89a51a06885015ee4))
* **webui:** keyboard shortcuts, scroll-wheel volume, playlist highlight, theme toggle, reconnect countdown ([a411fae](https://github.com/metaneutrons/snapdog/commit/a411fae5522d90220ceaf3d4b4e8867459c34823))
* **webui:** optimize playlist browser with modal grid and search ([09f2474](https://github.com/metaneutrons/snapdog/commit/09f2474d5ebd3c18fcd285551898a8a9486475c4))
* **webui:** remove stop button, add long-press to pause; fix client chip order ([8de3847](https://github.com/metaneutrons/snapdog/commit/8de38477168d517e4b15f12b99932faede83cc0b))
* **webui:** show buffer progress as secondary track on seek bar ([d14ddee](https://github.com/metaneutrons/snapdog/commit/d14ddee876a1baa0aae3c04f8ca98635789a07c6))
* **webui:** show playlists before clients, remove client list collapse ([3ccab61](https://github.com/metaneutrons/snapdog/commit/3ccab61ec6e784b20d1feb8fecb9cec9e8f0fa36))
* **webui:** show SnapDog icon as cover placeholder when idle or no cover art ([1a7e199](https://github.com/metaneutrons/snapdog/commit/1a7e19932d58d44a82ef25772752e90e761b3d2a))
* **webui:** sync EQ state, refactor curves, and improve accessibility ([6466928](https://github.com/metaneutrons/snapdog/commit/6466928bd8a4c066c8b0b1bf1d32e66849805888))
* **webui:** use marquee for track info in sidebar when text overflows ([5df396c](https://github.com/metaneutrons/snapdog/commit/5df396c8195872503d8df2262f0d149fc96a3a5f))
* **webui:** use SnapDog icon as favicon ([297ec12](https://github.com/metaneutrons/snapdog/commit/297ec12ef1ef463e78517d1fc5d6087c08213e20))
* **windows:** native Windows service support via --service flag ([30598a8](https://github.com/metaneutrons/snapdog/commit/30598a84d22a142e6b9dedf1d8f896eeba745fea))
* **ws:** add buffered_ms to ZoneProgress notification for cache progress ([c3c6915](https://github.com/metaneutrons/snapdog/commit/c3c6915a7bd0f5e9e88e9cac19ae1300f23a987a))


### Bug Fixes

* add buffered_ms to test ZoneState initialization ([4cdec91](https://github.com/metaneutrons/snapdog/commit/4cdec9186fa7fe81fe34c66f6bc6c202cac14fc8))
* **api:** improve startup log to show full listen address ([f1347f1](https://github.com/metaneutrons/snapdog/commit/f1347f1a67397102afb922341fc41e207397a846))
* **api:** return 409 Conflict for KNX programming mode when not in device mode ([caaed77](https://github.com/metaneutrons/snapdog/commit/caaed77cfd221a88643bb04b2d17c189b0d9b237))
* **api:** return absolute URLs for cover art using system.base_url ([b4b50cf](https://github.com/metaneutrons/snapdog/commit/b4b50cf555058a43ed4a1f4478e81ddcc1eb87b3))
* **api:** return absolute URLs for track cover_art in playlist tracks ([b703c9f](https://github.com/metaneutrons/snapdog/commit/b703c9f0571336049def16475517ce2c99fe9969))
* **audio:** use SeekMode::Coarse for cached file seek (avoids AAC decode errors) ([cf74d80](https://github.com/metaneutrons/snapdog/commit/cf74d802cc7cb9b157a54b816b7d7ec676480816))
* **build:** skip webui build in build.rs if out/ already exists (CI compatibility) ([d414803](https://github.com/metaneutrons/snapdog/commit/d41480387a91322c015bccb93b9c7b9175eeb6c2))
* **cache:** call prefetch from runner + invalidate cache on decode failure ([cf20b97](https://github.com/metaneutrons/snapdog/commit/cf20b977bdc7de90b49f98bcd74ea27ce7d11a1e))
* **ci:** add missing fields to integration tests + placeholder for KNX job ([df0e1e6](https://github.com/metaneutrons/snapdog/commit/df0e1e629ae2e4ec443f2e71daaf73a6a68f9d7f))
* **client:** clarify log message — audio output stream, not playback ([1f1794c](https://github.com/metaneutrons/snapdog/commit/1f1794cae59a0d694c315ee34078332e520b268e))
* clippy map().unwrap_or_else() → map_or_else() ([d7b230c](https://github.com/metaneutrons/snapdog/commit/d7b230c0a71ecf6dac09d1b418ef0ec1e9446af5))
* **config:** adapt integrations to SecretString and validation ([d40dba7](https://github.com/metaneutrons/snapdog/commit/d40dba7afae582221813b03721c7e490f5fa88ef))
* **config:** platform-appropriate default cache paths ([096cf05](https://github.com/metaneutrons/snapdog/commit/096cf0576fef0b0dffc62c5b20ac821d4f289963))
* **config:** Windows state dir uses LOCALAPPDATA (not Roaming), CamelCase ([d115d97](https://github.com/metaneutrons/snapdog/commit/d115d972aa9924253cfe30cb5ba7f82d549c6280))
* format + move test module to end of file (clippy items_after_test_module) ([7a04e97](https://github.com/metaneutrons/snapdog/commit/7a04e978cece6ca37ca54c414eeaf0acc4e97220))
* integration tests — use SecretString for password fields ([06880f4](https://github.com/metaneutrons/snapdog/commit/06880f4fa7446af423bcbb8edb3146e3b14b6162))
* **knx:** prevent write starvation by releasing store lock before bus I/O ([dc42167](https://github.com/metaneutrons/snapdog/commit/dc42167817f318a9438ced05106337c8066f0f99))
* **macos-helper:** add cover URL field to radio stations, fix editability ([ddf7ff0](https://github.com/metaneutrons/snapdog/commit/ddf7ff08dbf242bccd6b3ddf2fb8ebbe51d0908b))
* **macos-helper:** add placeholder to Station Name, uniform text weight for all radio fields ([add0cdf](https://github.com/metaneutrons/snapdog/commit/add0cdf4640759433d642958d8f8e4af420d7968))
* **macos-helper:** bring windows to front when opened from menu bar ([98711c8](https://github.com/metaneutrons/snapdog/commit/98711c8ec99e291f87ac75870da533a7157695b5))
* **macos-helper:** change bundle ID to com.metaneutrons.snapdog.helper ([5fa5717](https://github.com/metaneutrons/snapdog/commit/5fa5717f1d267c57c3effe74a9e0a0d8d0dd8c2a))
* **macos-helper:** disable App Sandbox (not needed for non-App Store distribution) ([8bc951e](https://github.com/metaneutrons/snapdog/commit/8bc951ebe11c3bed98386f27a9acb0c889a4490a))
* **macos-helper:** make radio/zones/clients text fields editable (remove List wrapper) ([691fcc5](https://github.com/metaneutrons/snapdog/commit/691fcc57722644085ddc3ae1a31bbd5d3c505c09))
* **macos-helper:** open built-in config editor instead of external app ([39598cc](https://github.com/metaneutrons/snapdog/commit/39598cc76ca05c4c00d0ec7e44ac249363b42537))
* **macos-helper:** port fields show integers without decimal point or grouping ([3e7d9ac](https://github.com/metaneutrons/snapdog/commit/3e7d9acdea9b870dae9b5df4307c92444fd02612))
* **macos-helper:** reduce padding in config dialog (remove .grouped form style) ([c671d37](https://github.com/metaneutrons/snapdog/commit/c671d374310fc038994ffaa6bc20d4c867ca4142))
* **macos-helper:** remove 'Optional' placeholder from MQTT username ([1faf3c0](https://github.com/metaneutrons/snapdog/commit/1faf3c01ec4dd2c00c7401e6c96f5a475c79067c))
* **macos-helper:** remove KNX from Integration tab (not relevant for macOS helper) ([f8e7b05](https://github.com/metaneutrons/snapdog/commit/f8e7b058defe940f303e6ed4490a2e4e360293ec))
* **macos-helper:** remove user-facing system config, use Apple conventions ([2e53ad5](https://github.com/metaneutrons/snapdog/commit/2e53ad59a6201afa6a4647279887a862b8621d21))
* **macos-helper:** restore .grouped form style with tighter fixed frame (480×360) ([2794ca7](https://github.com/metaneutrons/snapdog/commit/2794ca7387191973254ea5bfd41b0dfe79db2922))
* **macos-helper:** separate Zones and Clients into own tabs, non-collapsible sidebar ([a6021b5](https://github.com/metaneutrons/snapdog/commit/a6021b534a0f8a37b276c96f0e0cc6cf91f08116))
* **macos-helper:** simplify section heading to 'Subsonic' ([6b87c8f](https://github.com/metaneutrons/snapdog/commit/6b87c8fda494b2769683e0ef8de3a344b2869933))
* **macos-helper:** uniform text size for all radio station fields ([1864097](https://github.com/metaneutrons/snapdog/commit/18640970779642c19306d716b96b6653f795fd5b))
* **macos-helper:** use $(PRODUCT_BUNDLE_IDENTIFIER) in Info.plist ([9cf6032](https://github.com/metaneutrons/snapdog/commit/9cf603289bbbb32df93f07d61dda6de5f669babe))
* **macos-helper:** use $(PRODUCT_NAME) for CFBundleDisplayName ([07737f7](https://github.com/metaneutrons/snapdog/commit/07737f7d6955a8db56ca43f288078c5472aa05c1))
* **macos-helper:** use build setting variables for version and product name in Info.plist ([d295a80](https://github.com/metaneutrons/snapdog/commit/d295a8056ab42f3b614aba263a22144565a397cd))
* **macos-helper:** use Contents/Helpers/ for embedded binary (correct app bundle structure) ([008b41e](https://github.com/metaneutrons/snapdog/commit/008b41e3a181b5201c4764df41c6f9b444da8bfd))
* **macos-helper:** use native macOS menu style instead of custom window popover ([8d76a45](https://github.com/metaneutrons/snapdog/commit/8d76a45bbcaba224c6089d9f1c0b7970e8a616df))
* **macos-helper:** use TabView with sidebarAdaptable style for Settings ([1136108](https://github.com/metaneutrons/snapdog/commit/1136108a5e239847591830266eb4517e5754af9d))
* **media:** resolve missing radio cover art and add fallback logic ([bcb8a88](https://github.com/metaneutrons/snapdog/commit/bcb8a88235e8d53f39bc4eb1c23a7520ed9a1428))
* **media:** restore generic placeholder for radio playlist cover ([2669768](https://github.com/metaneutrons/snapdog/commit/26697681f4e30103cc5cd9b5b7e498d8f5085566))
* **media:** use relative cover URLs and improve radio fallback logic ([ac308d5](https://github.com/metaneutrons/snapdog/commit/ac308d52121d72944965151fba8efaec5967a1f0))
* **mqtt:** republish zone/client state on every change (not just on connect) ([85fe346](https://github.com/metaneutrons/snapdog/commit/85fe346d509c91f5da561e590ea3f907aa5c4f40))
* prevent potential deadlocks by dropping store lock before async sends ([528ae71](https://github.com/metaneutrons/snapdog/commit/528ae71dabe42efce37bae8fe3c989421da5b19b))
* **radio:** harden favicon fallback — SVG support, redirect limit, size validation ([47904da](https://github.com/metaneutrons/snapdog/commit/47904daa50fc5cdf37c82673c9b16053a40769a4))
* **release:** robust Sparkle sign_update discovery and appcast generation ([8103aaa](https://github.com/metaneutrons/snapdog/commit/8103aaa2c9a3a4e0f1a5e53db71c55e9f4dbaf8e))
* resolve all clippy warnings — const fn, map_or, unused params, docs ([86c562f](https://github.com/metaneutrons/snapdog/commit/86c562f3d63effc8b9222b2c7b98172b4f868a0a))
* resolve clippy lints (const fn, map_or_else, From cast) ([100c0cd](https://github.com/metaneutrons/snapdog/commit/100c0cd417d5879af88a578edb6dd03b0709a2fc))
* **shutdown:** handle second Ctrl+C as force-exit during graceful shutdown ([ea24a55](https://github.com/metaneutrons/snapdog/commit/ea24a550264675daf4870876ff41551f89bdebe7))
* **snapcast:** clear stale group IDs in sync_group_ids before re-assigning ([408effd](https://github.com/metaneutrons/snapdog/commit/408effd10afb3014bb69de364fe27506bbc400f7))
* **snapcast:** fix zone switch — detect wrong stream after eviction and fix it ([b699b17](https://github.com/metaneutrons/snapdog/commit/b699b17a9f0c8487349954da679e52ab5cd6f42f))
* **snapcast:** push persisted volume/mute to snapcast on client connect ([8004e84](https://github.com/metaneutrons/snapdog/commit/8004e8415031e1cfdb94eb37123eec79bcbcfcc5))
* **snapcast:** ReconcileZones handles zones without existing groups ([4989967](https://github.com/metaneutrons/snapdog/commit/498996794fae13675fa2a6bacd364ba5debbb59e))
* **snapcast:** sync zone mute state from snapcast group on startup and updates ([2a1eceb](https://github.com/metaneutrons/snapdog/commit/2a1ecebbc6fd22dd099d46bcaf7f4abbe4ed5130))
* **snapcast:** unmute group on client connect to prevent stale mute state ([22019c4](https://github.com/metaneutrons/snapdog/commit/22019c44f189eda5f469d9e2612a8d9873de21d5))
* **spinorama:** add User-Agent header and handle GitHub rate limiting ([33a5ba2](https://github.com/metaneutrons/snapdog/commit/33a5ba2fc989e5dc3e47960d137e07d5975aad06))
* unify macOS state path to ~/Library/Application Support/SnapDog/ ([b1a6a95](https://github.com/metaneutrons/snapdog/commit/b1a6a951ca40cb153f02045bdb8d18bd36cc9213))
* use NotifySender type in snapcast module (fixes snapcast-process feature build) ([4786bcd](https://github.com/metaneutrons/snapdog/commit/4786bcd289c8a3e9a8540e540f0e04d727c6bdbc))
* **webui:** consistent spacing between EQ and Speaker tabs ([bbd5337](https://github.com/metaneutrons/snapdog/commit/bbd5337250e6a82a2b3d8e0d2db39dd17da85f43))
* **webui:** consistent spacing between tabs and graph — use hidden instead of h-0 ([3cc746b](https://github.com/metaneutrons/snapdog/commit/3cc746b07724061b7892b89ae4d28ee552b3e1cc))
* **webui:** double placeholder cover art icon size ([b99818f](https://github.com/metaneutrons/snapdog/commit/b99818ff0067fdc112f783473c76d35241bdf074))
* **webui:** group icon + title + controls in header (no full-width spread) ([8eeecd1](https://github.com/metaneutrons/snapdog/commit/8eeecd17189fcf878944127ee6bf85d0ad1bdc3f))
* **webui:** prevent seek bar bounce — hold position until server confirms ([48cb195](https://github.com/metaneutrons/snapdog/commit/48cb19543dd7e089089af33c41898aa8ac6be321))
* **webui:** remove separator line between Clients heading and client chips ([b02b1a5](https://github.com/metaneutrons/snapdog/commit/b02b1a5df79c5c185a4bb0ea11554d3881d0f157))
* **webui:** remove unused import, name magic number in SeekBar ([7d96484](https://github.com/metaneutrons/snapdog/commit/7d964840ded31f02e9523ca7da1d855122e257c5))
* **webui:** SeekBar confirm uses abs() distance — fixes backward seek false-confirm ([12283cd](https://github.com/metaneutrons/snapdog/commit/12283cd587adab90c9ce6cea6fc58f3d5b999a80))
* **webui:** show 'Licensed under GPL-3.0' with link to full license text ([29378e4](https://github.com/metaneutrons/snapdog/commit/29378e40ba47aeccb0a7d5f0582d7886f70cfb5b))
* **webui:** show sidebar from md (768px) instead of lg (1024px) ([79d4f3d](https://github.com/metaneutrons/snapdog/commit/79d4f3dcdd8945985273bd0313f771cd77935682))
* **webui:** speaker list shows all results with infinite scroll pagination ([459d3db](https://github.com/metaneutrons/snapdog/commit/459d3dbc65adf0b6670b16a97564178502319ff6))
* **webui:** theme toggle now properly switches to light mode ([8481d70](https://github.com/metaneutrons/snapdog/commit/8481d7049e83cfa595a88eb23d5406e6f9b9663b))
* **webui:** vertically center header action buttons ([a577b2b](https://github.com/metaneutrons/snapdog/commit/a577b2b2b4de40231ce1521d1a1dc317a6d0e4c2))
* **windows:** use select! instead of spawn for service to avoid Send bound ([2d6bd75](https://github.com/metaneutrons/snapdog/commit/2d6bd7518a18b2e0a19530cb8047af185a684af9))
* **ws:** send Close frame with 1001 Going Away on server shutdown ([a914e36](https://github.com/metaneutrons/snapdog/commit/a914e365063178ebca4ffe708d09c10f5fc5ada3))


### Performance Improvements

* **ws:** pre-serialize notifications and implement connection limits ([3401e7c](https://github.com/metaneutrons/snapdog/commit/3401e7cb0d64215dc44f0857af6afa116470beca))

## [0.8.0](https://github.com/metaneutrons/snapdog/compare/v0.7.0...v0.8.0) (2026-05-04)


### Features

* audio crossfade on zone switch ([954a026](https://github.com/metaneutrons/snapdog/commit/954a0268f419e473e63152dada00c20a4b16605d))
* **audio:** server-side fade on source change within a zone ([f453399](https://github.com/metaneutrons/snapdog/commit/f453399fe287eba79ab3b2bb76d114f0c29c5cef))
* **audio:** source_conflict policy — receiver_wins vs last_wins ([1982690](https://github.com/metaneutrons/snapdog/commit/1982690f449e60ce7c9d03a24303907922f2382c))
* **ci:** APT repository on GitHub Pages ([93e04bd](https://github.com/metaneutrons/snapdog/commit/93e04bd8cad16bdadd24f7beae95f215857451f4))
* **config:** add system.base_url, use absolute cover URL in MQTT ([85fee9f](https://github.com/metaneutrons/snapdog/commit/85fee9f79edacf27fe30050134fcd94e7254cb4f))
* **config:** zone_switch_fade_ms — configurable fade duration ([47dc66e](https://github.com/metaneutrons/snapdog/commit/47dc66e8eb15837f6330d4b026f2a1b8892b0fd5))
* create snapdog-common crate — shared EQ types, constants, volume curve ([936b64d](https://github.com/metaneutrons/snapdog/commit/936b64d0bc7393332cc50f31386f8ff1ee05c328))
* **eq:** Apple-grade EQ UX — segmented toggle, preset chips, genre presets ([2e32bab](https://github.com/metaneutrons/snapdog/commit/2e32babdb59d24f07b645ac3d0cd11ad8caddb86))
* **knx:** publish initial state for all zones and clients on start ([1b06bb9](https://github.com/metaneutrons/snapdog/commit/1b06bb97f861e21e297a60eaff05114233d7561c))
* **mqtt:** publish initial state for all zones and clients on connect ([21af7f0](https://github.com/metaneutrons/snapdog/commit/21af7f027fd723b99433a03c464055fba1c16474))
* **player:** CD-player previous behavior (restart before skip) ([08a1c57](https://github.com/metaneutrons/snapdog/commit/08a1c57a8fbde72e075a6138a4023ffc3806f234))
* **radio:** dynamic cover art from ICY StreamUrl ([b7c3fe3](https://github.com/metaneutrons/snapdog/commit/b7c3fe3653a98db703de63a52b003e5aaf7cb8cf))
* **radio:** parse ICY StreamTitle into artist + title ([2c507e4](https://github.com/metaneutrons/snapdog/commit/2c507e4ae1fbe8bea385b4b000c3d637c8e902f4))
* Spinorama speaker correction EQ ([eed0f86](https://github.com/metaneutrons/snapdog/commit/eed0f86d81b167537c0c8e0e9f1aa800fad042f3))
* **xtask:** add 'cargo xtask ci' — run all CI checks locally ([6ff1543](https://github.com/metaneutrons/snapdog/commit/6ff1543c5df0fc1e29f94e9cea7f5f0b5502e4ce))


### Bug Fixes

* **audio:** resolve per-frame fade bug, DRY gain calc, magic numbers, and code quality issues ([f72b730](https://github.com/metaneutrons/snapdog/commit/f72b7306570557c273485b85e3850db8ddca73ed))
* CI failures — unused import, integration test SubsonicFormat ([7ba24ab](https://github.com/metaneutrons/snapdog/commit/7ba24ab41f1d06723d9fc5eb2c8668aa9150ed7d))
* **ci:** APT Release file with Date + SHA256/MD5Sum hashes ([9a7cc00](https://github.com/metaneutrons/snapdog/commit/9a7cc009e1c651c31700e955090fecbee46e90fb))
* **ci:** npm ci --omit=optional to avoid platform-specific failures ([5024219](https://github.com/metaneutrons/snapdog/commit/502421921d14f5cd36558564545d5551acc52011))
* **ci:** use npm ci --force to skip platform checks ([7887591](https://github.com/metaneutrons/snapdog/commit/7887591a3e2b3d45e84d6ca959078f9dd40b7124))
* clippy pedantic warnings in snapdog-client (Linux-only code) ([5d5bc8f](https://github.com/metaneutrons/snapdog/commit/5d5bc8fefb8dd6f5386f47d7f3cbb0ffe41472bb))
* enterprise-grade Rust audit — all remaining findings ([242b60b](https://github.com/metaneutrons/snapdog/commit/242b60bcfcdf4753e7934c4375852712776d5922))
* final audit findings — SSOT, tests, cleanup ([fede454](https://github.com/metaneutrons/snapdog/commit/fede454b1b8633756b2455dd373c9ca0c2cd69e6))
* force exit on shutdown (mDNS/AirPlay threads block clean exit) ([44f681e](https://github.com/metaneutrons/snapdog/commit/44f681e7205e0ba23a686ee85b44c43e2a14b12c))
* **mixer:** quadratic curve instead of cubic (better for amplifiers) ([a4ea894](https://github.com/metaneutrons/snapdog/commit/a4ea894cb2f317ecc7a55d2ca40c8a4a7ad3361f))
* only send fade-out to SnapDog clients (non-SnapDog clients ignore custom messages) ([1e27ec8](https://github.com/metaneutrons/snapdog/commit/1e27ec893d238de8c60673cd87857d982951ae41))
* remove .await on sync SnapserverHandle::start() in integration tests ([060cc6c](https://github.com/metaneutrons/snapdog/commit/060cc6c77f44b591434e4457e02e6c6370528c29))
* **snapdog-client:** perceptual volume curve for hardware mixer ([06bbb6a](https://github.com/metaneutrons/snapdog/commit/06bbb6a2c1a0c3edf8f10206c1c05e81760fd056))
* Spotify Connect uses zone name without '(Spotify)' suffix ([6c680c4](https://github.com/metaneutrons/snapdog/commit/6c680c4e149c0c21b2c072d7d205098d19c25349))
* **webui:** Apple-grade tab crossfade — no flash on switch ([5474cfc](https://github.com/metaneutrons/snapdog/commit/5474cfc05ac83bded5252ffccd76e527a5c0c32d))
* **webui:** consistent A/B position + empty curve when Speaker off ([3a93e5e](https://github.com/metaneutrons/snapdog/commit/3a93e5e7261bdd35a068e9d7ca1ccee7b7464c52))
* **webui:** disable A/B bypass button when EQ is off ([bd3b764](https://github.com/metaneutrons/snapdog/commit/bd3b764f8bf0d2d73aa8febf722b10f99f251220))
* **webui:** enterprise-grade audit — all remaining findings ([2380fd3](https://github.com/metaneutrons/snapdog/commit/2380fd30a7ad9e5c1248368bfecb1e6314b255ec))
* **webui:** EQ button visible on initial render ([2a31541](https://github.com/metaneutrons/snapdog/commit/2a315418184b32bb3b3354b50a86909ee402649b))
* **webui:** EQ chip indicator — overlay returns enabled state on close ([9be414c](https://github.com/metaneutrons/snapdog/commit/9be414ca39ea3371e23f8aa85683caa214746467))
* **webui:** EQ indicator — orange bold text instead of dot ([67fe71f](https://github.com/metaneutrons/snapdog/commit/67fe71f2bbacf935651449de97fb4092da12d6aa))
* **webui:** EQ On/Off now works, Off shows only coordinate grid ([d5ff5fa](https://github.com/metaneutrons/snapdog/commit/d5ff5fa9b4f60af275fc4e82d67977d8c31c4792))
* **webui:** EQ overlay — On/Off in header, fix A/B bypass logic ([9fd6c3e](https://github.com/metaneutrons/snapdog/commit/9fd6c3e7988a0ad68c8f9ae9498892305ab2cc6a))
* **webui:** EQ preset dropdown label → 'Presets' (all languages) ([4cc73e8](https://github.com/metaneutrons/snapdog/commit/4cc73e8679b29c6a331b778ef0eb6f2b20507fda))
* **webui:** EQ toggle-on no longer overwrites server bands with empty ([6f0ad53](https://github.com/metaneutrons/snapdog/commit/6f0ad53dc47910339a70f425cc9a206d0166d1c9))
* **webui:** remove transition-opacity from EQ tab content — instant state ([bf60aa3](https://github.com/metaneutrons/snapdog/commit/bf60aa3e492420fb515622393c37bc1aa7dffc3e))
* **webui:** Speaker On/Off in header next to title (like EQ tab) ([a9196dd](https://github.com/metaneutrons/snapdog/commit/a9196dd360fda82ac3d2dd2b1bce50e66670a63e))
* **webui:** Speaker tab — fix 'flat' lookup, hide content when off ([eb40457](https://github.com/metaneutrons/snapdog/commit/eb40457cfec10bf157d36df6da9cfc0ba58d5dd2))
* **webui:** Speaker tab gets independent On/Off + A/B controls ([5c5ef27](https://github.com/metaneutrons/snapdog/commit/5c5ef27b66d592013f8999ed88d91fbe46149f3d))
* **webui:** Speaker tab On button works without pre-selected speaker ([992b35a](https://github.com/metaneutrons/snapdog/commit/992b35a5413486b47451673dcaf0c9140789ea3f))
* **webui:** volume slider no longer flickers on release ([e32f240](https://github.com/metaneutrons/snapdog/commit/e32f24088b838e5f2d4d2df7bbfebfd869dc24cc))
* **xtask:** backward-compatible subcommand dispatch ([89e07b3](https://github.com/metaneutrons/snapdog/commit/89e07b30c81f2eb7c9bc6364d17985d9bdd95c45))
* zone volume/mute changes now reach all clients (stale group_id bug) ([4762ec4](https://github.com/metaneutrons/snapdog/commit/4762ec4fd9fcaceb175d24c8656367a95f16a0c0))

## [0.7.0](https://github.com/metaneutrons/snapdog/compare/v0.6.2...v0.7.0) (2026-05-02)


### Features

* **knx:** enterprise logging for device connections and ETS sessions ([d91aa6f](https://github.com/metaneutrons/snapdog/commit/d91aa6f8ea09c894a18673cd441d99fdf75c6ab8))
* **snapcast:** configurable unknown client policy (accept/ignore/reject) ([bd19a3d](https://github.com/metaneutrons/snapdog/commit/bd19a3dceb202e7ed540ae3bd6a2f180ace198e0))
* **snapdog-client:** ALSA mixer auto-detection and startup validation ([cabe48a](https://github.com/metaneutrons/snapdog/commit/cabe48ae869d36c13764b939d1956a9207587595))
* **webui:** EQ enabled indicator on zone and client buttons ([c6b2f8b](https://github.com/metaneutrons/snapdog/commit/c6b2f8bb4dcce954ac7351192a70ad4f9020087e))


### Bug Fixes

* **aur:** install /etc/default/ files for both packages ([b0405bb](https://github.com/metaneutrons/snapdog/commit/b0405bb3fcefb9c8401ebd86ad38c969f4e304bf))
* **deb:** client auto-starts with mDNS discovery (no config needed) ([5436263](https://github.com/metaneutrons/snapdog/commit/5436263775e970bdd39e522ddf2126687a827dab))
* **deb:** enterprise-grade systemd units and packaging ([01eb14a](https://github.com/metaneutrons/snapdog/commit/01eb14abcf9ea0bc55b3fdfda9aec3aaaf6ac97e))
* **deb:** snapdog server uses EnvironmentFile for flexible startup ([e4bc24f](https://github.com/metaneutrons/snapdog/commit/e4bc24faa9464c0b676086f9050b50ca6f807f09))
* **snapcast:** log ALL client connections, including unknown clients ([c279de6](https://github.com/metaneutrons/snapdog/commit/c279de6295fc0a2ff736b247f1eee3233f6e0b9b))
* **snapdog-client:** hostID CLI — remove misleading [default: ""] ([6bffe72](https://github.com/metaneutrons/snapdog/commit/6bffe72171956e4d650a5c814a1ae552eba53405))
* **ws:** client volume notification sends base_volume, not effective ([1b8b1bc](https://github.com/metaneutrons/snapdog/commit/1b8b1bc9e8bf04f5aeef591cab8897005e7e4637))

## [0.6.2](https://github.com/metaneutrons/snapdog/compare/v0.6.1...v0.6.2) (2026-05-01)


### Bug Fixes

* **docker:** enterprise-grade Dockerfiles with persistence volume ([f308442](https://github.com/metaneutrons/snapdog/commit/f308442e84bfae6a67f6f1639ecdaa8252ba9f10))

## [0.6.1](https://github.com/metaneutrons/snapdog/compare/v0.6.0...v0.6.1) (2026-04-30)


### Bug Fixes

* **ci:** create webui/out stub for knxprod job (rust-embed needs directory) ([67c65e9](https://github.com/metaneutrons/snapdog/commit/67c65e988bb79d46be04edacfdb0439429b24cf4))

## [0.6.0](https://github.com/metaneutrons/snapdog/compare/v0.5.0...v0.6.0) (2026-04-30)


### Features

* **knx:** complete KNX overhaul — knx-rs, ETS device mode, presence ([#12](https://github.com/metaneutrons/snapdog/issues/12)) ([b945394](https://github.com/metaneutrons/snapdog/commit/b94539486207dc59ce7a3db4e7d24f048f50d612))


### Bug Fixes

* **webui:** restore EQ enabled state when closing with A/B bypass active ([4c5af55](https://github.com/metaneutrons/snapdog/commit/4c5af5584256b3b792928ad9fb712dcfb689b0a7))

## [0.5.0](https://github.com/metaneutrons/snapdog/compare/v0.4.1...v0.5.0) (2026-04-18)


### Features

* **client:** add mixer support — software, hardware, midi, none ([b87951e](https://github.com/metaneutrons/snapdog/commit/b87951e20aec77395fae57dbc52e08deca1772c7))


### Bug Fixes

* **ci:** use native arm64 runners for Docker builds ([b5ff760](https://github.com/metaneutrons/snapdog/commit/b5ff760895016b4d4a06afe88ed43719c5e4263e))

## [0.4.1](https://github.com/metaneutrons/snapdog/compare/v0.4.0...v0.4.1) (2026-04-18)


### Bug Fixes

* **ci:** add actions:write permission for release dispatch ([4fba6e7](https://github.com/metaneutrons/snapdog/commit/4fba6e7c4230f5ee772f064190e2dcc94d7b7723))
* restore knx-rs workspace members (accidentally deleted) ([a89c643](https://github.com/metaneutrons/snapdog/commit/a89c643efdf73eecd7640ec0cfe08cc1d1b2b9f4))

## [0.4.0](https://github.com/metaneutrons/snapdog/compare/v0.3.1...v0.4.0) (2026-04-18)


### Features

* add .deb and AUR packaging for snapdog + snapdog-client ([e98f8b5](https://github.com/metaneutrons/snapdog/commit/e98f8b5be5bd4e6813599580e36f03afea0c3ba4))


### Bug Fixes

* **ci:** add --repo to gh workflow dispatch ([2406b40](https://github.com/metaneutrons/snapdog/commit/2406b4043ef494169b8224916495cc83c72ffbbe))

## [0.3.1](https://github.com/metaneutrons/snapdog/compare/v0.3.0...v0.3.1) (2026-04-18)


### Bug Fixes

* **ci:** include snapdog-client binary in release archives ([b240046](https://github.com/metaneutrons/snapdog/commit/b2400462ef1a0d80053368dfc9a398129d796ea5))
* **ci:** trigger release workflow from release-please ([e028a56](https://github.com/metaneutrons/snapdog/commit/e028a56a5be552623c5dc62d3311a73c3308ae6e))

## [0.3.0](https://github.com/metaneutrons/snapdog/compare/v0.2.0...v0.3.0) (2026-04-16)


### Features

* wire Spotify Connect into ZonePlayer, update README ([745a2e7](https://github.com/metaneutrons/snapdog/commit/745a2e717bbafad7eda15bde713bd23e21910c7c))


### Bug Fixes

* collapse if into match guard (clippy) ([d1a36e6](https://github.com/metaneutrons/snapdog/commit/d1a36e6cba843299502629d2ec146c3a61e73564))

## [0.2.0](https://github.com/metaneutrons/snapdog/compare/v0.1.0...v0.2.0) (2026-04-16)


### Features

* add --help and --version via clap ([9a4f0bd](https://github.com/metaneutrons/snapdog/commit/9a4f0bd5f88de7a2ab4ccc9f08bde4585bc171a1))
* add production Dockerfile ([fed8acc](https://github.com/metaneutrons/snapdog/commit/fed8acc8f73f02269c637c9fb3c6aad2d97d0705))
* **airplay:** add DACP debug logging, graceful fallback when unavailable ([b1f6b71](https://github.com/metaneutrons/snapdog/commit/b1f6b712bae947bb9f011bdbcdf847f4330a9587))
* **airplay:** AirPlay 1 receiver via libshairplay FFI ([5f45654](https://github.com/metaneutrons/snapdog/commit/5f4565439ddbbeef2003f7bf0a06050ad49a469c))
* **airplay:** align with shairplay-rust pure Rust crate ([a7d3bce](https://github.com/metaneutrons/snapdog/commit/a7d3bcee00b53179caa07e1753a457089ab67cf7))
* **airplay:** gate AP2 support behind 'ap2' feature flag ([3b37d41](https://github.com/metaneutrons/snapdog/commit/3b37d41bd3facf788971fdc65cc3586fcc414e02))
* **airplay:** replace C shairplay FFI with pure Rust shairplay crate ([ff7f192](https://github.com/metaneutrons/snapdog/commit/ff7f19260f8ec459cb94e385cf960851c8963a4b))
* **airplay:** switch to shairplay 0.1.0 from crates.io ([7fdc2dd](https://github.com/metaneutrons/snapdog/commit/7fdc2dd8da3f48f6a1817f5d47010ee301ec807e))
* **airplay:** upgrade to shairplay 0.2 — metadata off audio path ([6e51e59](https://github.com/metaneutrons/snapdog/commit/6e51e5982b0b22161e07321a56a6d9c0754b5aee))
* **airplay:** wire all callbacks — metadata, cover art, progress, disconnect ([f00ffe1](https://github.com/metaneutrons/snapdog/commit/f00ffe1dcab359f4e168697a83e5ef00b9220c70))
* **airplay:** wire DACP remote control for AirPlay transport commands ([e078e18](https://github.com/metaneutrons/snapdog/commit/e078e1846f28cfa4f1d916742655c6e575126e77))
* **api,state:** wire API endpoints to live state store ([c95b020](https://github.com/metaneutrons/snapdog/commit/c95b020c0c4b1fa3cf6d392fa33d930e13a801b4))
* **api:** add seekable flag to TrackInfo ([074218b](https://github.com/metaneutrons/snapdog/commit/074218b2ed630df65bf8b95b50bc84ab98830791))
* **api:** API key auth, playlist caching, volume coalescing ([7b0b94f](https://github.com/metaneutrons/snapdog/commit/7b0b94fc3562b15744aca6ca3966bc5395f231e4))
* **api:** cover art cache and endpoint (ADR-012) ([15c3dd6](https://github.com/metaneutrons/snapdog/commit/15c3dd62fea85f1f3c61d2c7a7eb921119b5099b))
* **api:** expose all metadata in track/metadata endpoint and zone source ([4f7bc24](https://github.com/metaneutrons/snapdog/commit/4f7bc2411aa27452381e400d41f1a167a326c683))
* **api:** REST API with full endpoint surface ([dc6490b](https://github.com/metaneutrons/snapdog/commit/dc6490bf634294d23a8ef08a1681482c25b2d1e7))
* **api:** route client volume/mute through Snapcast command channel ([6e3a34e](https://github.com/metaneutrons/snapdog/commit/6e3a34ee6e73487406856b1a9be9abce09f93a1a))
* **api:** support multiple API keys ([e088858](https://github.com/metaneutrons/snapdog/commit/e0888586f21e36290188292c21b1f73c3a1fa82d))
* **api:** support relative volume values (+5, -3) on PUT endpoints ([be93476](https://github.com/metaneutrons/snapdog/commit/be9347689f6c18b8e56394b3cc5f2b6f0cf020fc))
* **api:** WebSocket endpoint for real-time state notifications ([faa03d6](https://github.com/metaneutrons/snapdog/commit/faa03d6f8b558704ed9290b4e2c00ce451d46882))
* **api:** wire zone endpoints to ZonePlayer via command channel ([3ca083b](https://github.com/metaneutrons/snapdog/commit/3ca083bba36477630dc23b2d457500bcd2a377ef))
* **audio:** configurable output bit depth (16/24/32-bit) ([b075825](https://github.com/metaneutrons/snapdog/commit/b0758257d8e8a3d0a71d7929aa26023426f9f031))
* **audio:** HLS metadata, encrypted HLS detection, timeout/retry resilience ([1304871](https://github.com/metaneutrons/snapdog/commit/13048716811f5836c6b348fdc525b9233c7f2d9d))
* **audio:** HLS segment streaming support ([055848c](https://github.com/metaneutrons/snapdog/commit/055848c882616f020668788979c8821a6a9bc976))
* **audio:** HTTP stream decoding and MVP playback pipeline ([fb25f5c](https://github.com/metaneutrons/snapdog/commit/fb25f5c915570813f04e4d8b0856a3bf63e21737))
* **audio:** ICY metadata parsing for live radio title updates ([7750eca](https://github.com/metaneutrons/snapdog/commit/7750ecafa29180a85df90871d982f2acb2fc5fd0))
* **audio:** persistent streaming resampler via rubato ([804910a](https://github.com/metaneutrons/snapdog/commit/804910a9b1dffb273067fe69872fe772d940740e))
* **config:** add encryption_psk to audio config ([b6e0600](https://github.com/metaneutrons/snapdog/commit/b6e0600bb4065791dab14685f362922e5040dae6))
* **config:** add TOML parsing with convention-over-config ([b342e2d](https://github.com/metaneutrons/snapdog/commit/b342e2d476f71a326180cb49a49eaf1a9819f3d3))
* **config:** optional snapserver console output via verbose flag ([0205d33](https://github.com/metaneutrons/snapdog/commit/0205d33fdc3d5ca32ea4711f03da129ce838ef9f))
* **cover:** unified cover art with hash-based cache busting ([3a12108](https://github.com/metaneutrons/snapdog/commit/3a12108501582b22b6e191a68776e66b5e96896b))
* **docker:** add knx-monitor to dev rig ([128ddda](https://github.com/metaneutrons/snapdog/commit/128dddaa78dd9ab3ed8e052aba9eb885dc65804e))
* **docker:** embedded-only image, remove C++ snapserver build ([a765b9f](https://github.com/metaneutrons/snapdog/commit/a765b9f9194338ff23f16a8088a4efd7ea88a0a5))
* **docker:** enable MQTT verbose logging in dev rig ([69c46ee](https://github.com/metaneutrons/snapdog/commit/69c46ee7784471f87fcdf53db1554da9126df5bf))
* **dsp:** per-zone parametric EQ with WebUI overlay ([4651583](https://github.com/metaneutrons/snapdog/commit/46515838d6f3b625a27384efc6ccd900e25fadc0))
* **knx:** complete bidirectional KNX bridge, wired into main.rs ([ceca6c0](https://github.com/metaneutrons/snapdog/commit/ceca6c046e0605a798521d50f8bc4fba2c9fdd02))
* **knx:** KNX bridge with tunnel connection and status publishing ([24f653b](https://github.com/metaneutrons/snapdog/commit/24f653b5ce6368413f9a8ec05538800fb2ea0485))
* **knx:** KNX/IP address parsing, DPT encoding, and connection setup ([cc70d96](https://github.com/metaneutrons/snapdog/commit/cc70d96304908db8a4897ac5719ea003bbe2f429))
* **knx:** support tunnel and router via single URL config ([509e16f](https://github.com/metaneutrons/snapdog/commit/509e16f7ecc2e0a876ac6f40cc9ddb4967f06015))
* **knx:** wire remaining KNX group addresses ([bac564e](https://github.com/metaneutrons/snapdog/commit/bac564e2b920702d289d854182cb5d75a74be824))
* **mqtt:** bidirectional MQTT bridge with command/status topics ([347bb47](https://github.com/metaneutrons/snapdog/commit/347bb47e1b3aa1e9099e7404944ca3fe60ff290c))
* **player:** full ZonePlayer implementation with all commands and sources ([591a2a5](https://github.com/metaneutrons/snapdog/commit/591a2a53e78648bbfd2de78bc2493b1bbb2cc6e6))
* **player:** implement Seek and Playlist Navigation ([995cb41](https://github.com/metaneutrons/snapdog/commit/995cb414b658145a0859b08fee5c07860e05cb9a))
* **player:** resume Subsonic playback from last position after pause ([9447f91](https://github.com/metaneutrons/snapdog/commit/9447f9196ba60b39464070ac22d575ae927059c2))
* **player:** wire AirPlay into ZonePlayer — receiver, events, cover cache ([fcbf053](https://github.com/metaneutrons/snapdog/commit/fcbf053d510741e8bd946da043b04234246f4178))
* **process:** snapserver lifecycle management ([5c04c49](https://github.com/metaneutrons/snapdog/commit/5c04c498d4938330ae095aaa8c7ee275aebb5785))
* **receiver:** add Spotify Connect receiver skeleton (feature-gated) ([d42a928](https://github.com/metaneutrons/snapdog/commit/d42a9285510f4c0cf08f38178612bbc1b45bfb6a))
* **snapcast:** add event handler for embedded backend ([441404c](https://github.com/metaneutrons/snapdog/commit/441404c1651a20bf127f7c7184cbe0868a8c3760))
* **snapcast:** add snapcast-server dep with feature gates ([cc9376d](https://github.com/metaneutrons/snapdog/commit/cc9376d3171778db9f339ad12d36e880015d3148))
* **snapcast:** add SnapcastBackend trait + SnapcastEvent enum ([7fbc667](https://github.com/metaneutrons/snapdog/commit/7fbc66778fc3210ce6d6af1b26996698102aa175))
* **snapcast:** client zone reassignment via Group.SetClients + logging ([1680287](https://github.com/metaneutrons/snapdog/commit/1680287b24827550754dc1528eb063fea2816c46))
* **snapcast:** dynamic zone group setup on client connect ([4a9e284](https://github.com/metaneutrons/snapdog/commit/4a9e28476da50bdfdddc2eab77e7d6f87ed6ee66))
* **snapcast:** enable mDNS for embedded server ([77e20dc](https://github.com/metaneutrons/snapdog/commit/77e20dc50926656c40d11bd05693473ad33a2795))
* **snapcast:** enable state persistence for embedded server ([2f472fd](https://github.com/metaneutrons/snapdog/commit/2f472fdcbebcfee749248bec66a1390058bbac61))
* **snapcast:** event-driven client state + startup fixes ([5225818](https://github.com/metaneutrons/snapdog/commit/522581887a357e40c0c1fba1efcde87685844ab9))
* **snapcast:** fix embedded audio routing and pacing ([d8913b4](https://github.com/metaneutrons/snapdog/commit/d8913b4efd321b8c60e810a3b0672c28b8648431))
* **snapcast:** handle server-pushed notifications in main loop ([bdf338b](https://github.com/metaneutrons/snapdog/commit/bdf338b01b8ec93fadd4736e72d7e988f6fa3bc7))
* **snapcast:** implement EmbeddedBackend using snapcast-server crate ([f3172bf](https://github.com/metaneutrons/snapdog/commit/f3172bf9ea615975b70ba35f5d957d764ece0ecf))
* **snapcast:** implement ProcessBackend wrapping JSON-RPC + TCP sinks ([136cfa5](https://github.com/metaneutrons/snapdog/commit/136cfa580fe1198349c8c786ac833d1e4bdbf6e2))
* **snapcast:** JSON-RPC connection and control API ([f0f98bb](https://github.com/metaneutrons/snapdog/commit/f0f98bba0892907fe5d2ecc9f7b235f1010b2538))
* **snapcast:** per-zone audio streams via add_stream() ([b7264e7](https://github.com/metaneutrons/snapdog/commit/b7264e74fe11843a8ffcdfa71414ccbcb73b9e29))
* **snapcast:** use MAC address from embedded server for client matching ([b0f9740](https://github.com/metaneutrons/snapdog/commit/b0f9740213d1d31d5f65a8105eb3378a558516cf))
* **snapcast:** wire GroupNameChanged, GroupClientsChanged, ServerUpdated events ([657478d](https://github.com/metaneutrons/snapdog/commit/657478dc8496f91398ed3d81f2fdd106dd26fab6))
* snapdog-client with per-client EQ ([#5](https://github.com/metaneutrons/snapdog/issues/5)) ([21c7fdb](https://github.com/metaneutrons/snapdog/commit/21c7fdbe15be31348d09221d31c85d685f641643))
* **spotify:** migrate librespot from git dev to crates.io 0.8 ([e242b89](https://github.com/metaneutrons/snapdog/commit/e242b898233b1a6e122fe2a8ab42875cdd81e7cc))
* **state:** in-memory state store with JSON persistence ([e7e5c2b](https://github.com/metaneutrons/snapdog/commit/e7e5c2b2500a47ca61d49d1d709150032ab088c0))
* **subsonic:** add tls_skip_verify for self-signed certs ([ef9c72f](https://github.com/metaneutrons/snapdog/commit/ef9c72ff55c74d536e13e52c927cc9b566adb9cb))
* **subsonic:** configurable stream format, default flac ([cbc3b0b](https://github.com/metaneutrons/snapdog/commit/cbc3b0b480f1a623967cb867f239d4b1b1642226))
* **subsonic:** Subsonic API client for music library access ([6f54f79](https://github.com/metaneutrons/snapdog/commit/6f54f79d733e6bf1e488aa63c2cec1fc395af165))
* **test:** add 3 MQTT integration tests (conditional on .env.test) ([c0cbfe1](https://github.com/metaneutrons/snapdog/commit/c0cbfe1ccafe04276f17a2330704bde272925877))
* **test:** add 8 API integration tests with real HTTP + real snapserver ([0f769e8](https://github.com/metaneutrons/snapdog/commit/0f769e8cba663d33e94c52134bb9d0c795c5722a))
* **test:** comprehensive integration test suite ([69b88ae](https://github.com/metaneutrons/snapdog/commit/69b88aee823f24c5395bb5b04337eae5d961d41d))
* **test:** integration tests with real snapserver, fix config generation ([dc38ff3](https://github.com/metaneutrons/snapdog/commit/dc38ff30779267ac8fa1f30f4d7ecf5586a15590))
* **webui:** 4-tier responsive layout, remove connection dot ([1046f97](https://github.com/metaneutrons/snapdog/commit/1046f9752c8d9ad9f780c12d45ededeaf658d1bd))
* **webui:** add i18n infrastructure with next-intl ([b8ac408](https://github.com/metaneutrons/snapdog/commit/b8ac408133fa532947f2d5f040d2cf000cea92f4))
* **webui:** API key prompt with localStorage persistence ([60243a3](https://github.com/metaneutrons/snapdog/commit/60243a3b425676afa6f02d7873fb3c968d3a8002))
* **webui:** Apple-style locale picker ([74bdfa3](https://github.com/metaneutrons/snapdog/commit/74bdfa3257a58944eabdc9b896eb1bceb668a03d))
* **webui:** complete all UI components and wire to API ([26c4228](https://github.com/metaneutrons/snapdog/commit/26c4228b9abe2d28ee388056501b0d66cd493e04))
* **webui:** cover art matches controls height on desktop ([c3c9ef6](https://github.com/metaneutrons/snapdog/commit/c3c9ef65375f42a89adb648f911b1e92a2aa037f))
* **webui:** drag-and-drop client chips to mobile zone tabs ([1d0490e](https://github.com/metaneutrons/snapdog/commit/1d0490e2fd01f173059762d1e1b1aaa480fb3f03))
* **webui:** elapsed time counter for endless streams (radio) ([8650c9d](https://github.com/metaneutrons/snapdog/commit/8650c9d9d8b260adfa5a757e71c96237ca78f82f))
* **webui:** flexible zone card width with min/max constraints ([29e68eb](https://github.com/metaneutrons/snapdog/commit/29e68eb5db3636105a63a0d6882af482c4ea2dec))
* **webui:** hide offline clients by default with toggle ([81b2132](https://github.com/metaneutrons/snapdog/commit/81b2132a9ac720b08ccd572d9e98695dd6048936))
* **webui:** improve zone card layout + drag-and-drop clients ([1ffe57e](https://github.com/metaneutrons/snapdog/commit/1ffe57e8598c721e82b37107ec6278ee763ee9a5))
* **webui:** integrate i18n in all components ([51fa72d](https://github.com/metaneutrons/snapdog/commit/51fa72d124e7db04ad0b5c3474f1e7e365cee5cd))
* **webui:** marquee effect for overflowing track metadata ([03b72e7](https://github.com/metaneutrons/snapdog/commit/03b72e786eb3a2d8ae2eaad95f8d13cc32df2a86))
* **webui:** move track metadata above controls, always show controls ([cde305c](https://github.com/metaneutrons/snapdog/commit/cde305c4542052a581db564d4c22c671ee26d1e8))
* **webui:** multi-zone grid layout on desktop (xl+) ([e169a4e](https://github.com/metaneutrons/snapdog/commit/e169a4ee511f4c0d537df6d1962bb430f00175d3))
* **webui:** polish, rust-embed integration, and build tooling ([dc3f9ff](https://github.com/metaneutrons/snapdog/commit/dc3f9ff25bc08b6b3ff441c489a6738af6526910))
* **webui:** polished cover art fallback with gradient and zone name ([dcdf054](https://github.com/metaneutrons/snapdog/commit/dcdf054d649f64615a22b56add1be08a6956ed97))
* **webui:** scaffold Next.js 16 SPA with full data layer and responsive shell ([d4ca2c6](https://github.com/metaneutrons/snapdog/commit/d4ca2c663d03173b9785e75be729184e3c339d37))
* **webui:** show track cover art in playlist browser ([9e9472a](https://github.com/metaneutrons/snapdog/commit/9e9472a739efc1814434429ef6ec578aa8d10d6c))
* **webui:** three-dots menu on client chips for zone reassignment ([e1abff4](https://github.com/metaneutrons/snapdog/commit/e1abff4a56ae87f6ff670d461236a5cb2faeb3f3))
* **webui:** use desktop-style zone card on tablet, pin max width ([03bd856](https://github.com/metaneutrons/snapdog/commit/03bd85623b760562031f0c311f9c3e2d96999563))
* wire MQTT bridge into main loop, fix parallel test flakiness ([d4b7670](https://github.com/metaneutrons/snapdog/commit/d4b767074e01dd1a5df61d26f101edafdeda7112))


### Bug Fixes

* add wildcard arm for non_exhaustive ServerEvent ([64417cb](https://github.com/metaneutrons/snapdog/commit/64417cbfd37fd7ae787ea65767616ac0490adaac))
* **airplay:** AirPlay working on macOS ([4257b5a](https://github.com/metaneutrons/snapdog/commit/4257b5a445a7c694e40dd136f6f2eda250f90944))
* **airplay:** sequential port search from 5000, IPv4+IPv6 safe ([d1bf714](https://github.com/metaneutrons/snapdog/commit/d1bf7140111dcfe4c5c087762c601ecfad4a7acc))
* align cfg gate on snap_notifications with process_backend ([75e7cd7](https://github.com/metaneutrons/snapdog/commit/75e7cd768dacdd1e1f6536eb28918269cfa84f6a))
* **api:** eliminate all dead stubs — every endpoint wired to real logic ([b0dbb4c](https://github.com/metaneutrons/snapdog/commit/b0dbb4cf4db23fc72beced48fa86c0dc5fbe15b6))
* **api:** handle 'radio' playlist ID in media endpoints ([b9a68fe](https://github.com/metaneutrons/snapdog/commit/b9a68fe4137b5266cc54cd807aee42e127e96158))
* **api:** health endpoint returns real zone/client count from config ([4b9aa5c](https://github.com/metaneutrons/snapdog/commit/4b9aa5c251abe6123a43a6ae3c320aebba37787b))
* **api:** MIME dedup, compression, set_zone fire-and-forget ([6b27ab3](https://github.com/metaneutrons/snapdog/commit/6b27ab3486b0b94775a05d5fad6b56a9423e5869))
* **audio:** 4s buffer for smooth playback + instant EOF on disconnect ([8e19d2a](https://github.com/metaneutrons/snapdog/commit/8e19d2af14c74b91153625c927d3774462531d11))
* **audio:** CRITICAL — passthrough resampler was dropping all audio ([d466210](https://github.com/metaneutrons/snapdog/commit/d4662101a110c3b5173c877a8bdaa6d1035ed47c))
* **audio:** don't pass through unresampled AirPlay audio during buffering ([713f32b](https://github.com/metaneutrons/snapdog/commit/713f32b883142c7a1d052773e4047c6591a8502f))
* **audio:** faster radio stream restart, downgrade stream error to WARN ([f661fc5](https://github.com/metaneutrons/snapdog/commit/f661fc5e6e4106b8fd090e4d8052e6ae93317707))
* **audio:** output PCM at configured bit depth, not hardcoded S16LE ([cd296d1](https://github.com/metaneutrons/snapdog/commit/cd296d1cfaedbac5b7d318e91727d318d61505c8))
* **audio:** proper resampling for active sources (radio, Subsonic, URL) ([dc63041](https://github.com/metaneutrons/snapdog/commit/dc63041f7f6e8863f5c6700fcd3927e7a75bd883))
* **audio:** robust playlist URL resolution with all edge cases handled ([a3ed35f](https://github.com/metaneutrons/snapdog/commit/a3ed35f08394139313b8f4e6317a2912114e9974))
* **audio:** suppress unused variable warning in resolve_playlist_url ([9532b9b](https://github.com/metaneutrons/snapdog/commit/9532b9b5284d7cb21eae02d90f43083d0de8786c))
* **audio:** use decoded sample rate, not container metadata ([7eac256](https://github.com/metaneutrons/snapdog/commit/7eac256de48ecade267553583f3f04f36d304bea))
* **audio:** use final URL after redirects for playlist extension detection ([ef9c72f](https://github.com/metaneutrons/snapdog/commit/ef9c72ff55c74d536e13e52c927cc9b566adb9cb))
* **audio:** use HLS-specific tags for media playlist detection ([d3ca760](https://github.com/metaneutrons/snapdog/commit/d3ca760068eae2e44d0c2657d945ff8e46626ae0))
* **ci:** add pkg-config to Docker, openssl cross-compile for aarch64 ([3e35190](https://github.com/metaneutrons/snapdog/commit/3e35190b3b50558c4690514260bfe8e0ab5569b5))
* **ci:** cmake in Docker rust stage, fix aarch64 apt sources ([680788e](https://github.com/metaneutrons/snapdog/commit/680788e25097490828f69b12f5b43352aa686dd7))
* **ci:** fix integration tests — gate broken tests, fix feature flags ([c165cab](https://github.com/metaneutrons/snapdog/commit/c165cab7ef99e3e493f53ca6a8c79510573186f3))
* **ci:** fix snapcast-process compilation + mosquitto native install ([67c8249](https://github.com/metaneutrons/snapdog/commit/67c824989e51d02d80cd70e16a0f26255cfe3fa9))
* **ci:** gate integration tests behind snapcast-process, fix items-after-test-module ([4d3ca95](https://github.com/metaneutrons/snapdog/commit/4d3ca957b22632edc8fe5475dbbc85f9703248f3))
* **ci:** make subsonic tests graceful when server not configured ([8fff978](https://github.com/metaneutrons/snapdog/commit/8fff9780505a684b632ec691b55f16fa6f7e34f5))
* **ci:** pin existing apt to amd64, add arm64 security from ports ([59e32ce](https://github.com/metaneutrons/snapdog/commit/59e32ce276ef88a3e732b4508243dab339ab846c))
* **ci:** regenerate Cargo.lock without path patches ([1fb7441](https://github.com/metaneutrons/snapdog/commit/1fb74411e72dfc40f4910dd46ec58fe2e0f66ee8))
* **ci:** remove committed .cargo/config.toml with path patch ([6fd420b](https://github.com/metaneutrons/snapdog/commit/6fd420b258cfe7057a6b07867b705d0243f32a74))
* **ci:** replace mosquitto Docker service with native install ([c32c9f4](https://github.com/metaneutrons/snapdog/commit/c32c9f4edb04bf1fdb64566114b3023b3a6d1a2b))
* **ci:** scope aarch64 env vars to cross-compile step only ([a94a38e](https://github.com/metaneutrons/snapdog/commit/a94a38e6fc7e97a90c95a236c53b3020ab861c58))
* **ci:** use Node 25 in Dockerfile, regenerate lockfile ([ef5bf8d](https://github.com/metaneutrons/snapdog/commit/ef5bf8d3ef93d0be487b05ceed7f68ee2e87c07f))
* **ci:** use Node 25 to match lockfile, regenerate package-lock.json ([68a7ef2](https://github.com/metaneutrons/snapdog/commit/68a7ef2d4ed9811860e1cfe4d341c73da2fa7f53))
* clippy — use contains_key instead of get().is_none() ([ea76c1b](https://github.com/metaneutrons/snapdog/commit/ea76c1bc987a1526c73e61ab75327030da12e390))
* commit Cargo.lock, fix release-please config ([e302791](https://github.com/metaneutrons/snapdog/commit/e302791e7f3eaa1848259dcdc11391b7def33247))
* comprehensive WebUI and backend fixes from live testing ([1d130a0](https://github.com/metaneutrons/snapdog/commit/1d130a0349e2a18605df6fdb0f673a394c96299d))
* **cover:** add User-Agent header to cover art fetcher ([1f6f6f9](https://github.com/metaneutrons/snapdog/commit/1f6f6f9cec96fab446d0e6bf69b0b268e5a06940))
* **cover:** clear cover cache when switching to station without cover URL ([81a809b](https://github.com/metaneutrons/snapdog/commit/81a809b6545161744cea208556746dcbc7cc7efd))
* **cover:** fetch radio covers synchronously to prevent race condition ([1adbfc6](https://github.com/metaneutrons/snapdog/commit/1adbfc6161847dfd2e49ae072ab9aea8516f8671))
* **cover:** fetch radio station cover on next/previous, return 404 for missing covers ([dec9740](https://github.com/metaneutrons/snapdog/commit/dec9740244810d402cc4dbcc7bcfdd14bc76c1e0))
* **cover:** prevent browser caching of stale cover art ([dfa946f](https://github.com/metaneutrons/snapdog/commit/dfa946f186a5a0ebac5951519c940802c541dd66))
* **cover:** skip SetPlaylist when already in radio mode ([1d46330](https://github.com/metaneutrons/snapdog/commit/1d463305866cea0c45459c55709a949df5e5c784))
* **dev:** expose KNX port on UDP (KNX/IP tunneling uses UDP, not TCP) ([7a316b2](https://github.com/metaneutrons/snapdog/commit/7a316b248acd0a1346f494b6e6057ee352be177d))
* **docker:** add libstdc++ to snapserver, null player for clients, default config ([687a7f2](https://github.com/metaneutrons/snapdog/commit/687a7f2e5f42a48957ca85e0e8291273b34ce4b1))
* **docker:** add WebUI build stage, remove obsolete dependencies ([2770c78](https://github.com/metaneutrons/snapdog/commit/2770c782c43c797f0fa21ddeb4e0f214d56ccdae))
* **docker:** build Snapcast from source, fix non-existent images ([2c73752](https://github.com/metaneutrons/snapdog/commit/2c737523527d4a167169d56f82209adcb08ad9eb))
* **docker:** expose TCP source ports 4953/4954 for dev mode ([4c56737](https://github.com/metaneutrons/snapdog/commit/4c56737ba245e7e8ad2aa7450202bfd7bc94a4f3))
* **docker:** use latest stable Rust in production Dockerfile ([814dd04](https://github.com/metaneutrons/snapdog/commit/814dd0439256408e18b6e710f0ee35946a5e18dc))
* eliminate group churn on client reconnect ([05ab4df](https://github.com/metaneutrons/snapdog/commit/05ab4df853f3c5af3d25a72f86aa8a4f2facc871))
* **eq:** merge EQ routes into zones nest, always show frequency curve ([dec5fbb](https://github.com/metaneutrons/snapdog/commit/dec5fbbb6304f600183dde9dfb7af2c2a81cf5dd))
* **http:** use Chrome user-agent for external HTTP requests ([808edb7](https://github.com/metaneutrons/snapdog/commit/808edb7826a4c48e2336ea21a3f4f60eb8ca4f5a))
* make snapcast types available for both backends ([c6a8b76](https://github.com/metaneutrons/snapdog/commit/c6a8b760f5d345260a17bee0ac3ad756f37126c9))
* make snapcast-embedded and snapcast-process mutually exclusive ([f1927d6](https://github.com/metaneutrons/snapdog/commit/f1927d6c716c1b2379c6a04b4a4c65a46696babb))
* **mqtt:** route all zone commands through ZonePlayer channels ([ab5b186](https://github.com/metaneutrons/snapdog/commit/ab5b186d7d8478de0531af47654c458b4ae15a90))
* **player:** eliminate two-step SetPlaylist+SetTrack race condition ([f1455e0](https://github.com/metaneutrons/snapdog/commit/f1455e0df38632a8ddba338b04414c3d88ea6274))
* **player:** preserve seek position — add offset for server-side seek ([bcf8f49](https://github.com/metaneutrons/snapdog/commit/bcf8f492319b525ad315ee52b32a5acd4a88f507))
* **player:** reset position offset on every stop_decode ([d77a3d0](https://github.com/metaneutrons/snapdog/commit/d77a3d0e6f20365223c12610e99ff71a4efeed9b))
* **player:** resume radio after pause ([6a52653](https://github.com/metaneutrons/snapdog/commit/6a5265361a440d903679bee7356bc31bead6fd18))
* **player:** seek while paused auto-resumes playback ([ff8ba2f](https://github.com/metaneutrons/snapdog/commit/ff8ba2f7b75126391b93cb68f7901bc5aeb394d4))
* **player:** set position offset on Subsonic resume after pause ([c0a7ab0](https://github.com/metaneutrons/snapdog/commit/c0a7ab0747f03ac6a568df71abee962f8d05fa89))
* **player:** SetTrack now works for radio sources ([f45dcba](https://github.com/metaneutrons/snapdog/commit/f45dcbaa0fe9700b0200ad2785144d09083b3d35))
* **player:** track playback position from symphonia packet timestamps ([ed43064](https://github.com/metaneutrons/snapdog/commit/ed4306461f5fe196d6badea5a3e0e883fb1a5a9f))
* **process:** fix snapserver config — port collision, dual-stack, retry ([12269e1](https://github.com/metaneutrons/snapdog/commit/12269e177eb7376c13b4d0c9c6376f6058ad879a))
* **radio:** preserve station name, show ICY data as artist ([9c29981](https://github.com/metaneutrons/snapdog/commit/9c29981eafd22eae3f1c6ca1e824dcecc0b6dd84))
* realtime pacing with now_usec() timestamp after sleep ([f0e0fc5](https://github.com/metaneutrons/snapdog/commit/f0e0fc571f808ef4b7ab5ac86716a6055e664c7c))
* remove hardcoded auto-start of first radio station ([77ec768](https://github.com/metaneutrons/snapdog/commit/77ec76837c764fbe1e73b4feb31a8bdf1803c5e0))
* resolve 10 TODOs — media endpoints, play/url bug, WS commands, MQTT dispatch ([7f86034](https://github.com/metaneutrons/snapdog/commit/7f860343301b01faebb11037c932df660e3c1e43))
* **snapcast:** add client_set_mute for mute-only commands ([bc07f89](https://github.com/metaneutrons/snapdog/commit/bc07f89e37579b8436e0dc87f63ab65ae5c3b248))
* **snapcast:** add group management to embedded event handler ([f54e2fb](https://github.com/metaneutrons/snapdog/commit/f54e2fb1fbad3d4885679ca67430b924cadde6b4))
* **snapcast:** handle Result from add_f32_stream ([898cc91](https://github.com/metaneutrons/snapdog/commit/898cc914d325800546a61edb91a307ff8ea4395c))
* **snapcast:** match groups to zones by stream ID, not group UUID ([332e74a](https://github.com/metaneutrons/snapdog/commit/332e74ad40f665789f0c67c7700af4210887b894))
* **snapcast:** per-zone mutex to avoid cross-zone contention on send_audio ([211d7b4](https://github.com/metaneutrons/snapdog/commit/211d7b475a5028812e9c309b8d637819acd356ae))
* **snapcast:** prefer connected client when same MAC has multiple entries ([11c147b](https://github.com/metaneutrons/snapdog/commit/11c147b505e392eea992cf4e86049c50afdc54d1))
* **snapcast:** preserve volume on mute and mute on volume change ([221959f](https://github.com/metaneutrons/snapdog/commit/221959f8eb4d2d00b45c65fd6b54fcbe41456179))
* **snapcast:** remove GroupClientsChanged, match C++ snapserver events ([5cf7906](https://github.com/metaneutrons/snapdog/commit/5cf79064fea7ea0c701aba33219bd6a4c11fbb5a))
* **snapcast:** resolve hostname for JSON-RPC connection, add startup delay ([dee1c15](https://github.com/metaneutrons/snapdog/commit/dee1c15d89768ba77762a80d1a5a5f2573face44))
* **snapcast:** retry initial JSON-RPC connect (10 attempts, 500ms backoff) ([0e98e7c](https://github.com/metaneutrons/snapdog/commit/0e98e7c3102f5401994f09e62456e8806cac827f))
* **snapcast:** serialize typed ServerStatus to JSON for backend trait ([0fef067](https://github.com/metaneutrons/snapdog/commit/0fef067f8b93c22b52b71094e9df952eb113bad0))
* **snapcast:** ServerOnUpdate only syncs group IDs, not zone_index ([19d248d](https://github.com/metaneutrons/snapdog/commit/19d248db8562872787dfef6f27fd1a90c903f77e))
* **snapcast:** sync client state after command execution ([b3e048f](https://github.com/metaneutrons/snapdog/commit/b3e048f6ad589c429b611c25fd9711701857b126))
* **snapcast:** sync client state at startup + handle all notifications ([1cf434e](https://github.com/metaneutrons/snapdog/commit/1cf434eaba2a35eea3af79afb02f5e61018fdb44))
* **snapcast:** sync client zone assignments from group membership at startup ([1459afa](https://github.com/metaneutrons/snapdog/commit/1459afa1f06e0f2272c8baf93e54cd2b71ace1a8))
* **snapcast:** sync client zone assignments on ServerOnUpdate ([ca5e7a0](https://github.com/metaneutrons/snapdog/commit/ca5e7a0c353f9c57774f06a96e3cd05d5b5ad711))
* **snapcast:** sync client zones after Group.SetClients via Server.GetStatus ([e2d0f7c](https://github.com/metaneutrons/snapdog/commit/e2d0f7ce2a98dcf988d8fbf96b23f4abe1f229dd))
* **snapcast:** sync group IDs on ServerUpdated in embedded event handler ([fe13b5d](https://github.com/metaneutrons/snapdog/commit/fe13b5d2a76d73ec8660694ae5fb9bdc85bd68bf))
* **snapcast:** use jsonrpc_port from config instead of streaming_port+1 ([0c12e12](https://github.com/metaneutrons/snapdog/commit/0c12e12b53874d871d04d7c2d42742cd8a643292))
* **subsonic:** request raw format to preserve original quality ([9df8029](https://github.com/metaneutrons/snapdog/commit/9df802928b818cc05d5cdcc4f740e43aa80803d4))
* use simple release type with toml jsonpath for workspace version ([cbd00f1](https://github.com/metaneutrons/snapdog/commit/cbd00f19733a127ac8b39c03e162d529e449caf3))
* **webui:** add 16px margin on each side for mobile zone card ([4936f5e](https://github.com/metaneutrons/snapdog/commit/4936f5e00addeb18f3ceda7fdae0ff0afc0bcd53))
* **webui:** always show controls in idle mode, hide sidebar on desktop ([07224ac](https://github.com/metaneutrons/snapdog/commit/07224acae8db7094c39325c71ab0dce886279caa))
* **webui:** auto-reload state on WebSocket reconnect + steering rules ([6472172](https://github.com/metaneutrons/snapdog/commit/647217211fd2d932cbfeec517e5defe4f22d8f25))
* **webui:** auto-retry cover art load after 1.5s for late-arriving AirPlay covers ([7963d0f](https://github.com/metaneutrons/snapdog/commit/7963d0f24cf2383d5ef872937070ac7322154e48))
* **webui:** cap client chip width at 256px (max-w-64) ([4ce0ad5](https://github.com/metaneutrons/snapdog/commit/4ce0ad59a469b7d0f3f91952b0a1331ff1b8f712))
* **webui:** client cards side by side with auto line-break ([5c288e1](https://github.com/metaneutrons/snapdog/commit/5c288e136587810bc61ecb2b23da8a4b4b6d789a))
* **webui:** client chip UX improvements ([10b41c7](https://github.com/metaneutrons/snapdog/commit/10b41c71b6629b2b7c2b526d10160ea890fc89e2))
* **webui:** clients collapsible but open by default on all viewports ([ba4da9b](https://github.com/metaneutrons/snapdog/commit/ba4da9b8617f2328f3fd3327e5773118f64b07f9))
* **webui:** collapse client list by default on mobile ([a7ac3a1](https://github.com/metaneutrons/snapdog/commit/a7ac3a12daea8afab4b809f2d98a08323b5ce153))
* **webui:** consistent controls column height across zone cards ([ec814aa](https://github.com/metaneutrons/snapdog/commit/ec814aa6491a464b5965fa3123aa19a22cf4e8e3))
* **webui:** consistent TrackInfo height across all zone cards ([b282bfb](https://github.com/metaneutrons/snapdog/commit/b282bfb8e9a1307421c8b139f8965943e4366c6b))
* **webui:** controls max-width drives zone card width limit ([45d478e](https://github.com/metaneutrons/snapdog/commit/45d478e4a9c18e4a3199b8d140118abcdfedc378))
* **webui:** handle API key change/removal — clear stale key on 401 ([1ed4881](https://github.com/metaneutrons/snapdog/commit/1ed4881f3755af350b26d050c69c19ae63372399))
* **webui:** hide playlist duration when 0:00 (e.g. radio playlist) ([2b3ef6d](https://github.com/metaneutrons/snapdog/commit/2b3ef6d5d933c71af27a87d01e0cc24a6fdc0deb))
* **webui:** hide ZoneHeader on mobile — tab bar already shows zone name ([0c79a24](https://github.com/metaneutrons/snapdog/commit/0c79a248e2966b5e11e6ac8da76910e6b8d659df))
* **webui:** increase cover art size from w-56 to w-64 for better proportion ([c61a264](https://github.com/metaneutrons/snapdog/commit/c61a264bf1c494dac7cdc6eca1b14dd1ec104be9))
* **webui:** make slider track visible against muted backgrounds ([6cfd211](https://github.com/metaneutrons/snapdog/commit/6cfd2119b2ca5dc2d03179b05372824b8d0bee0a))
* **webui:** mobile zone card takes full display width ([c20c69f](https://github.com/metaneutrons/snapdog/commit/c20c69fdfeaf0d2e5277df14cc62634842766b9e))
* **webui:** move clients and playlists to full-width below cover+controls ([3e9f81f](https://github.com/metaneutrons/snapdog/commit/3e9f81fa43bb7bd0934d5b66ba5daece996463ce))
* **webui:** move PlaylistBrowser below horizontal layout for full card width ([57b479b](https://github.com/metaneutrons/snapdog/commit/57b479b0cf309a626e2928dbf8a2b38829867952))
* **webui:** persist selected zone across refresh via sessionStorage ([e48ea52](https://github.com/metaneutrons/snapdog/commit/e48ea5298e8f284edb8cb80572a49669103d3356))
* **webui:** pin zone card max width to exactly 660px ([b176f5d](https://github.com/metaneutrons/snapdog/commit/b176f5d60f11e2138d199dd2778662f2efbaa0ee))
* **webui:** player controls use full width on mobile ([f3d8801](https://github.com/metaneutrons/snapdog/commit/f3d8801dce7e004c9aa4412a3414d4696958c7a1))
* **webui:** prevent chip drag when interacting with volume slider ([1d99201](https://github.com/metaneutrons/snapdog/commit/1d99201ca0d41c9133dfb6a6dd9cc52037eb5d61))
* **webui:** push cover_url update when cover fetch completes ([3e79ac8](https://github.com/metaneutrons/snapdog/commit/3e79ac80c3228cb50a7b16771a8029ca5b9885d9))
* **webui:** push cover_url via WebSocket on track change ([2bfaf22](https://github.com/metaneutrons/snapdog/commit/2bfaf220814be1390ba52997dd408ff57fbae936))
* **webui:** reliable cover art updates, always-square cover, plain img ([38bae54](https://github.com/metaneutrons/snapdog/commit/38bae54f3ed7ba723d3802f8f822acd7523af760))
* **webui:** responsive desktop grid with auto-fill columns ([ee2ec6b](https://github.com/metaneutrons/snapdog/commit/ee2ec6bd10b3130196bd16b8d375fd458fb2e6e9))
* **webui:** stepped reconnect backoff (1/2.5/5/10/15s) and wire onReconnect ([323bf2a](https://github.com/metaneutrons/snapdog/commit/323bf2a384e23dae572772716e1cd734f29be8b5))
* **webui:** stop cover art flickering on radio ICY metadata updates ([cdf93c8](https://github.com/metaneutrons/snapdog/commit/cdf93c8d9eada42fe067598fd69225067c1d4e03))
* **webui:** stronger sunken effect for client cards ([be73cdf](https://github.com/metaneutrons/snapdog/commit/be73cdff9ecbca4341b58c36344686ba38a6c492))
* **webui:** sunken client cards with drag handle for visual affordance ([5c9456f](https://github.com/metaneutrons/snapdog/commit/5c9456f85433e36ea95ed5445c8eef573d81ec9e))
* **webui:** sync seek bar on playback state change ([855a61b](https://github.com/metaneutrons/snapdog/commit/855a61b5100637289df90ea13f3268c487267ebc))
* **webui:** truly responsive zone cards with flex-wrap layout ([6fc1730](https://github.com/metaneutrons/snapdog/commit/6fc1730be7fd653b9ca1c5c0522c463cf011a339))
* **webui:** use CSS grid for equal-width client chips ([0f2cec1](https://github.com/metaneutrons/snapdog/commit/0f2cec16c96e89f091d4b7c690dc80c5ca96bed7))
* **webui:** use object-contain for cover art + update NDR 1 cover URL ([3405a8e](https://github.com/metaneutrons/snapdog/commit/3405a8e9464b9605b90a46fe0409fb0eb2c5801d))
* **webui:** zone card layout audit improvements ([5dc964f](https://github.com/metaneutrons/snapdog/commit/5dc964f217acf58f0d6fb3de42be2e9d55f2f0ea))
* **webui:** zone card max width to 600px ([e40c9eb](https://github.com/metaneutrons/snapdog/commit/e40c9ebd143b7331cbe188e3554b8ba315197581))
* **ws:** trigger ZoneTrackChanged on artist change too ([2496d3c](https://github.com/metaneutrons/snapdog/commit/2496d3c309dd75a3afac9cc5c36b7579a0a35a83))


### Performance Improvements

* **audio:** eliminate unnecessary allocations in PCM pipeline ([f450e0b](https://github.com/metaneutrons/snapdog/commit/f450e0b5b0b2ee197f4100fbbed5554b0acd6af5))
* **player:** reduce config cloning in ZonePlayer select loop ([de76b46](https://github.com/metaneutrons/snapdog/commit/de76b461534b2fa3ec2d8277c8a0202ad3805b69))

## [0.1.0](https://github.com/metaneutrons/snapdog/compare/snapdog-v0.1.0...snapdog-v0.1.0) (2026-04-14)


### Features

* add --help and --version via clap ([9a4f0bd](https://github.com/metaneutrons/snapdog/commit/9a4f0bd5f88de7a2ab4ccc9f08bde4585bc171a1))
* add production Dockerfile ([fed8acc](https://github.com/metaneutrons/snapdog/commit/fed8acc8f73f02269c637c9fb3c6aad2d97d0705))
* **airplay:** add DACP debug logging, graceful fallback when unavailable ([b1f6b71](https://github.com/metaneutrons/snapdog/commit/b1f6b712bae947bb9f011bdbcdf847f4330a9587))
* **airplay:** AirPlay 1 receiver via libshairplay FFI ([5f45654](https://github.com/metaneutrons/snapdog/commit/5f4565439ddbbeef2003f7bf0a06050ad49a469c))
* **airplay:** align with shairplay-rust pure Rust crate ([a7d3bce](https://github.com/metaneutrons/snapdog/commit/a7d3bcee00b53179caa07e1753a457089ab67cf7))
* **airplay:** gate AP2 support behind 'ap2' feature flag ([3b37d41](https://github.com/metaneutrons/snapdog/commit/3b37d41bd3facf788971fdc65cc3586fcc414e02))
* **airplay:** replace C shairplay FFI with pure Rust shairplay crate ([ff7f192](https://github.com/metaneutrons/snapdog/commit/ff7f19260f8ec459cb94e385cf960851c8963a4b))
* **airplay:** switch to shairplay 0.1.0 from crates.io ([7fdc2dd](https://github.com/metaneutrons/snapdog/commit/7fdc2dd8da3f48f6a1817f5d47010ee301ec807e))
* **airplay:** upgrade to shairplay 0.2 — metadata off audio path ([6e51e59](https://github.com/metaneutrons/snapdog/commit/6e51e5982b0b22161e07321a56a6d9c0754b5aee))
* **airplay:** wire all callbacks — metadata, cover art, progress, disconnect ([f00ffe1](https://github.com/metaneutrons/snapdog/commit/f00ffe1dcab359f4e168697a83e5ef00b9220c70))
* **airplay:** wire DACP remote control for AirPlay transport commands ([e078e18](https://github.com/metaneutrons/snapdog/commit/e078e1846f28cfa4f1d916742655c6e575126e77))
* **api,state:** wire API endpoints to live state store ([c95b020](https://github.com/metaneutrons/snapdog/commit/c95b020c0c4b1fa3cf6d392fa33d930e13a801b4))
* **api:** add seekable flag to TrackInfo ([074218b](https://github.com/metaneutrons/snapdog/commit/074218b2ed630df65bf8b95b50bc84ab98830791))
* **api:** API key auth, playlist caching, volume coalescing ([7b0b94f](https://github.com/metaneutrons/snapdog/commit/7b0b94fc3562b15744aca6ca3966bc5395f231e4))
* **api:** cover art cache and endpoint (ADR-012) ([15c3dd6](https://github.com/metaneutrons/snapdog/commit/15c3dd62fea85f1f3c61d2c7a7eb921119b5099b))
* **api:** expose all metadata in track/metadata endpoint and zone source ([4f7bc24](https://github.com/metaneutrons/snapdog/commit/4f7bc2411aa27452381e400d41f1a167a326c683))
* **api:** REST API with full endpoint surface ([dc6490b](https://github.com/metaneutrons/snapdog/commit/dc6490bf634294d23a8ef08a1681482c25b2d1e7))
* **api:** route client volume/mute through Snapcast command channel ([6e3a34e](https://github.com/metaneutrons/snapdog/commit/6e3a34ee6e73487406856b1a9be9abce09f93a1a))
* **api:** support multiple API keys ([e088858](https://github.com/metaneutrons/snapdog/commit/e0888586f21e36290188292c21b1f73c3a1fa82d))
* **api:** support relative volume values (+5, -3) on PUT endpoints ([be93476](https://github.com/metaneutrons/snapdog/commit/be9347689f6c18b8e56394b3cc5f2b6f0cf020fc))
* **api:** WebSocket endpoint for real-time state notifications ([faa03d6](https://github.com/metaneutrons/snapdog/commit/faa03d6f8b558704ed9290b4e2c00ce451d46882))
* **api:** wire zone endpoints to ZonePlayer via command channel ([3ca083b](https://github.com/metaneutrons/snapdog/commit/3ca083bba36477630dc23b2d457500bcd2a377ef))
* **audio:** configurable output bit depth (16/24/32-bit) ([b075825](https://github.com/metaneutrons/snapdog/commit/b0758257d8e8a3d0a71d7929aa26023426f9f031))
* **audio:** HLS metadata, encrypted HLS detection, timeout/retry resilience ([1304871](https://github.com/metaneutrons/snapdog/commit/13048716811f5836c6b348fdc525b9233c7f2d9d))
* **audio:** HLS segment streaming support ([055848c](https://github.com/metaneutrons/snapdog/commit/055848c882616f020668788979c8821a6a9bc976))
* **audio:** HTTP stream decoding and MVP playback pipeline ([fb25f5c](https://github.com/metaneutrons/snapdog/commit/fb25f5c915570813f04e4d8b0856a3bf63e21737))
* **audio:** ICY metadata parsing for live radio title updates ([7750eca](https://github.com/metaneutrons/snapdog/commit/7750ecafa29180a85df90871d982f2acb2fc5fd0))
* **audio:** persistent streaming resampler via rubato ([804910a](https://github.com/metaneutrons/snapdog/commit/804910a9b1dffb273067fe69872fe772d940740e))
* **config:** add encryption_psk to audio config ([b6e0600](https://github.com/metaneutrons/snapdog/commit/b6e0600bb4065791dab14685f362922e5040dae6))
* **config:** add TOML parsing with convention-over-config ([b342e2d](https://github.com/metaneutrons/snapdog/commit/b342e2d476f71a326180cb49a49eaf1a9819f3d3))
* **config:** optional snapserver console output via verbose flag ([0205d33](https://github.com/metaneutrons/snapdog/commit/0205d33fdc3d5ca32ea4711f03da129ce838ef9f))
* **cover:** unified cover art with hash-based cache busting ([3a12108](https://github.com/metaneutrons/snapdog/commit/3a12108501582b22b6e191a68776e66b5e96896b))
* **docker:** add knx-monitor to dev rig ([128ddda](https://github.com/metaneutrons/snapdog/commit/128dddaa78dd9ab3ed8e052aba9eb885dc65804e))
* **docker:** enable MQTT verbose logging in dev rig ([69c46ee](https://github.com/metaneutrons/snapdog/commit/69c46ee7784471f87fcdf53db1554da9126df5bf))
* **dsp:** per-zone parametric EQ with WebUI overlay ([4651583](https://github.com/metaneutrons/snapdog/commit/46515838d6f3b625a27384efc6ccd900e25fadc0))
* **knx:** complete bidirectional KNX bridge, wired into main.rs ([ceca6c0](https://github.com/metaneutrons/snapdog/commit/ceca6c046e0605a798521d50f8bc4fba2c9fdd02))
* **knx:** KNX bridge with tunnel connection and status publishing ([24f653b](https://github.com/metaneutrons/snapdog/commit/24f653b5ce6368413f9a8ec05538800fb2ea0485))
* **knx:** KNX/IP address parsing, DPT encoding, and connection setup ([cc70d96](https://github.com/metaneutrons/snapdog/commit/cc70d96304908db8a4897ac5719ea003bbe2f429))
* **knx:** support tunnel and router via single URL config ([509e16f](https://github.com/metaneutrons/snapdog/commit/509e16f7ecc2e0a876ac6f40cc9ddb4967f06015))
* **knx:** wire remaining KNX group addresses ([bac564e](https://github.com/metaneutrons/snapdog/commit/bac564e2b920702d289d854182cb5d75a74be824))
* **mqtt:** bidirectional MQTT bridge with command/status topics ([347bb47](https://github.com/metaneutrons/snapdog/commit/347bb47e1b3aa1e9099e7404944ca3fe60ff290c))
* **player:** full ZonePlayer implementation with all commands and sources ([591a2a5](https://github.com/metaneutrons/snapdog/commit/591a2a53e78648bbfd2de78bc2493b1bbb2cc6e6))
* **player:** implement Seek and Playlist Navigation ([995cb41](https://github.com/metaneutrons/snapdog/commit/995cb414b658145a0859b08fee5c07860e05cb9a))
* **player:** resume Subsonic playback from last position after pause ([9447f91](https://github.com/metaneutrons/snapdog/commit/9447f9196ba60b39464070ac22d575ae927059c2))
* **player:** wire AirPlay into ZonePlayer — receiver, events, cover cache ([fcbf053](https://github.com/metaneutrons/snapdog/commit/fcbf053d510741e8bd946da043b04234246f4178))
* **process:** snapserver lifecycle management ([5c04c49](https://github.com/metaneutrons/snapdog/commit/5c04c498d4938330ae095aaa8c7ee275aebb5785))
* **receiver:** add Spotify Connect receiver skeleton (feature-gated) ([d42a928](https://github.com/metaneutrons/snapdog/commit/d42a9285510f4c0cf08f38178612bbc1b45bfb6a))
* **snapcast:** add event handler for embedded backend ([441404c](https://github.com/metaneutrons/snapdog/commit/441404c1651a20bf127f7c7184cbe0868a8c3760))
* **snapcast:** add snapcast-server dep with feature gates ([cc9376d](https://github.com/metaneutrons/snapdog/commit/cc9376d3171778db9f339ad12d36e880015d3148))
* **snapcast:** add SnapcastBackend trait + SnapcastEvent enum ([7fbc667](https://github.com/metaneutrons/snapdog/commit/7fbc66778fc3210ce6d6af1b26996698102aa175))
* **snapcast:** client zone reassignment via Group.SetClients + logging ([1680287](https://github.com/metaneutrons/snapdog/commit/1680287b24827550754dc1528eb063fea2816c46))
* **snapcast:** dynamic zone group setup on client connect ([4a9e284](https://github.com/metaneutrons/snapdog/commit/4a9e28476da50bdfdddc2eab77e7d6f87ed6ee66))
* **snapcast:** enable mDNS for embedded server ([77e20dc](https://github.com/metaneutrons/snapdog/commit/77e20dc50926656c40d11bd05693473ad33a2795))
* **snapcast:** enable state persistence for embedded server ([2f472fd](https://github.com/metaneutrons/snapdog/commit/2f472fdcbebcfee749248bec66a1390058bbac61))
* **snapcast:** event-driven client state + startup fixes ([5225818](https://github.com/metaneutrons/snapdog/commit/522581887a357e40c0c1fba1efcde87685844ab9))
* **snapcast:** fix embedded audio routing and pacing ([d8913b4](https://github.com/metaneutrons/snapdog/commit/d8913b4efd321b8c60e810a3b0672c28b8648431))
* **snapcast:** handle server-pushed notifications in main loop ([bdf338b](https://github.com/metaneutrons/snapdog/commit/bdf338b01b8ec93fadd4736e72d7e988f6fa3bc7))
* **snapcast:** implement EmbeddedBackend using snapcast-server crate ([f3172bf](https://github.com/metaneutrons/snapdog/commit/f3172bf9ea615975b70ba35f5d957d764ece0ecf))
* **snapcast:** implement ProcessBackend wrapping JSON-RPC + TCP sinks ([136cfa5](https://github.com/metaneutrons/snapdog/commit/136cfa580fe1198349c8c786ac833d1e4bdbf6e2))
* **snapcast:** JSON-RPC connection and control API ([f0f98bb](https://github.com/metaneutrons/snapdog/commit/f0f98bba0892907fe5d2ecc9f7b235f1010b2538))
* **snapcast:** per-zone audio streams via add_stream() ([b7264e7](https://github.com/metaneutrons/snapdog/commit/b7264e74fe11843a8ffcdfa71414ccbcb73b9e29))
* **snapcast:** use MAC address from embedded server for client matching ([b0f9740](https://github.com/metaneutrons/snapdog/commit/b0f9740213d1d31d5f65a8105eb3378a558516cf))
* **snapcast:** wire GroupNameChanged, GroupClientsChanged, ServerUpdated events ([657478d](https://github.com/metaneutrons/snapdog/commit/657478dc8496f91398ed3d81f2fdd106dd26fab6))
* **spotify:** migrate librespot from git dev to crates.io 0.8 ([e242b89](https://github.com/metaneutrons/snapdog/commit/e242b898233b1a6e122fe2a8ab42875cdd81e7cc))
* **state:** in-memory state store with JSON persistence ([e7e5c2b](https://github.com/metaneutrons/snapdog/commit/e7e5c2b2500a47ca61d49d1d709150032ab088c0))
* **subsonic:** add tls_skip_verify for self-signed certs ([ef9c72f](https://github.com/metaneutrons/snapdog/commit/ef9c72ff55c74d536e13e52c927cc9b566adb9cb))
* **subsonic:** configurable stream format, default flac ([cbc3b0b](https://github.com/metaneutrons/snapdog/commit/cbc3b0b480f1a623967cb867f239d4b1b1642226))
* **subsonic:** Subsonic API client for music library access ([6f54f79](https://github.com/metaneutrons/snapdog/commit/6f54f79d733e6bf1e488aa63c2cec1fc395af165))
* **test:** add 3 MQTT integration tests (conditional on .env.test) ([c0cbfe1](https://github.com/metaneutrons/snapdog/commit/c0cbfe1ccafe04276f17a2330704bde272925877))
* **test:** add 8 API integration tests with real HTTP + real snapserver ([0f769e8](https://github.com/metaneutrons/snapdog/commit/0f769e8cba663d33e94c52134bb9d0c795c5722a))
* **test:** comprehensive integration test suite ([69b88ae](https://github.com/metaneutrons/snapdog/commit/69b88aee823f24c5395bb5b04337eae5d961d41d))
* **test:** integration tests with real snapserver, fix config generation ([dc38ff3](https://github.com/metaneutrons/snapdog/commit/dc38ff30779267ac8fa1f30f4d7ecf5586a15590))
* **webui:** 4-tier responsive layout, remove connection dot ([1046f97](https://github.com/metaneutrons/snapdog/commit/1046f9752c8d9ad9f780c12d45ededeaf658d1bd))
* **webui:** add i18n infrastructure with next-intl ([b8ac408](https://github.com/metaneutrons/snapdog/commit/b8ac408133fa532947f2d5f040d2cf000cea92f4))
* **webui:** API key prompt with localStorage persistence ([60243a3](https://github.com/metaneutrons/snapdog/commit/60243a3b425676afa6f02d7873fb3c968d3a8002))
* **webui:** Apple-style locale picker ([74bdfa3](https://github.com/metaneutrons/snapdog/commit/74bdfa3257a58944eabdc9b896eb1bceb668a03d))
* **webui:** complete all UI components and wire to API ([26c4228](https://github.com/metaneutrons/snapdog/commit/26c4228b9abe2d28ee388056501b0d66cd493e04))
* **webui:** cover art matches controls height on desktop ([c3c9ef6](https://github.com/metaneutrons/snapdog/commit/c3c9ef65375f42a89adb648f911b1e92a2aa037f))
* **webui:** drag-and-drop client chips to mobile zone tabs ([1d0490e](https://github.com/metaneutrons/snapdog/commit/1d0490e2fd01f173059762d1e1b1aaa480fb3f03))
* **webui:** elapsed time counter for endless streams (radio) ([8650c9d](https://github.com/metaneutrons/snapdog/commit/8650c9d9d8b260adfa5a757e71c96237ca78f82f))
* **webui:** flexible zone card width with min/max constraints ([29e68eb](https://github.com/metaneutrons/snapdog/commit/29e68eb5db3636105a63a0d6882af482c4ea2dec))
* **webui:** hide offline clients by default with toggle ([81b2132](https://github.com/metaneutrons/snapdog/commit/81b2132a9ac720b08ccd572d9e98695dd6048936))
* **webui:** improve zone card layout + drag-and-drop clients ([1ffe57e](https://github.com/metaneutrons/snapdog/commit/1ffe57e8598c721e82b37107ec6278ee763ee9a5))
* **webui:** integrate i18n in all components ([51fa72d](https://github.com/metaneutrons/snapdog/commit/51fa72d124e7db04ad0b5c3474f1e7e365cee5cd))
* **webui:** marquee effect for overflowing track metadata ([03b72e7](https://github.com/metaneutrons/snapdog/commit/03b72e786eb3a2d8ae2eaad95f8d13cc32df2a86))
* **webui:** move track metadata above controls, always show controls ([cde305c](https://github.com/metaneutrons/snapdog/commit/cde305c4542052a581db564d4c22c671ee26d1e8))
* **webui:** multi-zone grid layout on desktop (xl+) ([e169a4e](https://github.com/metaneutrons/snapdog/commit/e169a4ee511f4c0d537df6d1962bb430f00175d3))
* **webui:** polish, rust-embed integration, and build tooling ([dc3f9ff](https://github.com/metaneutrons/snapdog/commit/dc3f9ff25bc08b6b3ff441c489a6738af6526910))
* **webui:** polished cover art fallback with gradient and zone name ([dcdf054](https://github.com/metaneutrons/snapdog/commit/dcdf054d649f64615a22b56add1be08a6956ed97))
* **webui:** scaffold Next.js 16 SPA with full data layer and responsive shell ([d4ca2c6](https://github.com/metaneutrons/snapdog/commit/d4ca2c663d03173b9785e75be729184e3c339d37))
* **webui:** show track cover art in playlist browser ([9e9472a](https://github.com/metaneutrons/snapdog/commit/9e9472a739efc1814434429ef6ec578aa8d10d6c))
* **webui:** three-dots menu on client chips for zone reassignment ([e1abff4](https://github.com/metaneutrons/snapdog/commit/e1abff4a56ae87f6ff670d461236a5cb2faeb3f3))
* **webui:** use desktop-style zone card on tablet, pin max width ([03bd856](https://github.com/metaneutrons/snapdog/commit/03bd85623b760562031f0c311f9c3e2d96999563))
* wire MQTT bridge into main loop, fix parallel test flakiness ([d4b7670](https://github.com/metaneutrons/snapdog/commit/d4b767074e01dd1a5df61d26f101edafdeda7112))


### Bug Fixes

* add wildcard arm for non_exhaustive ServerEvent ([64417cb](https://github.com/metaneutrons/snapdog/commit/64417cbfd37fd7ae787ea65767616ac0490adaac))
* **airplay:** AirPlay working on macOS ([4257b5a](https://github.com/metaneutrons/snapdog/commit/4257b5a445a7c694e40dd136f6f2eda250f90944))
* **airplay:** sequential port search from 5000, IPv4+IPv6 safe ([d1bf714](https://github.com/metaneutrons/snapdog/commit/d1bf7140111dcfe4c5c087762c601ecfad4a7acc))
* align cfg gate on snap_notifications with process_backend ([75e7cd7](https://github.com/metaneutrons/snapdog/commit/75e7cd768dacdd1e1f6536eb28918269cfa84f6a))
* **api:** eliminate all dead stubs — every endpoint wired to real logic ([b0dbb4c](https://github.com/metaneutrons/snapdog/commit/b0dbb4cf4db23fc72beced48fa86c0dc5fbe15b6))
* **api:** handle 'radio' playlist ID in media endpoints ([b9a68fe](https://github.com/metaneutrons/snapdog/commit/b9a68fe4137b5266cc54cd807aee42e127e96158))
* **api:** health endpoint returns real zone/client count from config ([4b9aa5c](https://github.com/metaneutrons/snapdog/commit/4b9aa5c251abe6123a43a6ae3c320aebba37787b))
* **api:** MIME dedup, compression, set_zone fire-and-forget ([6b27ab3](https://github.com/metaneutrons/snapdog/commit/6b27ab3486b0b94775a05d5fad6b56a9423e5869))
* **audio:** 4s buffer for smooth playback + instant EOF on disconnect ([8e19d2a](https://github.com/metaneutrons/snapdog/commit/8e19d2af14c74b91153625c927d3774462531d11))
* **audio:** CRITICAL — passthrough resampler was dropping all audio ([d466210](https://github.com/metaneutrons/snapdog/commit/d4662101a110c3b5173c877a8bdaa6d1035ed47c))
* **audio:** don't pass through unresampled AirPlay audio during buffering ([713f32b](https://github.com/metaneutrons/snapdog/commit/713f32b883142c7a1d052773e4047c6591a8502f))
* **audio:** faster radio stream restart, downgrade stream error to WARN ([f661fc5](https://github.com/metaneutrons/snapdog/commit/f661fc5e6e4106b8fd090e4d8052e6ae93317707))
* **audio:** output PCM at configured bit depth, not hardcoded S16LE ([cd296d1](https://github.com/metaneutrons/snapdog/commit/cd296d1cfaedbac5b7d318e91727d318d61505c8))
* **audio:** proper resampling for active sources (radio, Subsonic, URL) ([dc63041](https://github.com/metaneutrons/snapdog/commit/dc63041f7f6e8863f5c6700fcd3927e7a75bd883))
* **audio:** robust playlist URL resolution with all edge cases handled ([a3ed35f](https://github.com/metaneutrons/snapdog/commit/a3ed35f08394139313b8f4e6317a2912114e9974))
* **audio:** suppress unused variable warning in resolve_playlist_url ([9532b9b](https://github.com/metaneutrons/snapdog/commit/9532b9b5284d7cb21eae02d90f43083d0de8786c))
* **audio:** use decoded sample rate, not container metadata ([7eac256](https://github.com/metaneutrons/snapdog/commit/7eac256de48ecade267553583f3f04f36d304bea))
* **audio:** use final URL after redirects for playlist extension detection ([ef9c72f](https://github.com/metaneutrons/snapdog/commit/ef9c72ff55c74d536e13e52c927cc9b566adb9cb))
* **audio:** use HLS-specific tags for media playlist detection ([d3ca760](https://github.com/metaneutrons/snapdog/commit/d3ca760068eae2e44d0c2657d945ff8e46626ae0))
* clippy — use contains_key instead of get().is_none() ([ea76c1b](https://github.com/metaneutrons/snapdog/commit/ea76c1bc987a1526c73e61ab75327030da12e390))
* commit Cargo.lock, fix release-please config ([e302791](https://github.com/metaneutrons/snapdog/commit/e302791e7f3eaa1848259dcdc11391b7def33247))
* comprehensive WebUI and backend fixes from live testing ([1d130a0](https://github.com/metaneutrons/snapdog/commit/1d130a0349e2a18605df6fdb0f673a394c96299d))
* **cover:** add User-Agent header to cover art fetcher ([1f6f6f9](https://github.com/metaneutrons/snapdog/commit/1f6f6f9cec96fab446d0e6bf69b0b268e5a06940))
* **cover:** clear cover cache when switching to station without cover URL ([81a809b](https://github.com/metaneutrons/snapdog/commit/81a809b6545161744cea208556746dcbc7cc7efd))
* **cover:** fetch radio covers synchronously to prevent race condition ([1adbfc6](https://github.com/metaneutrons/snapdog/commit/1adbfc6161847dfd2e49ae072ab9aea8516f8671))
* **cover:** fetch radio station cover on next/previous, return 404 for missing covers ([dec9740](https://github.com/metaneutrons/snapdog/commit/dec9740244810d402cc4dbcc7bcfdd14bc76c1e0))
* **cover:** prevent browser caching of stale cover art ([dfa946f](https://github.com/metaneutrons/snapdog/commit/dfa946f186a5a0ebac5951519c940802c541dd66))
* **cover:** skip SetPlaylist when already in radio mode ([1d46330](https://github.com/metaneutrons/snapdog/commit/1d463305866cea0c45459c55709a949df5e5c784))
* **dev:** expose KNX port on UDP (KNX/IP tunneling uses UDP, not TCP) ([7a316b2](https://github.com/metaneutrons/snapdog/commit/7a316b248acd0a1346f494b6e6057ee352be177d))
* **docker:** add libstdc++ to snapserver, null player for clients, default config ([687a7f2](https://github.com/metaneutrons/snapdog/commit/687a7f2e5f42a48957ca85e0e8291273b34ce4b1))
* **docker:** add WebUI build stage, remove obsolete dependencies ([2770c78](https://github.com/metaneutrons/snapdog/commit/2770c782c43c797f0fa21ddeb4e0f214d56ccdae))
* **docker:** build Snapcast from source, fix non-existent images ([2c73752](https://github.com/metaneutrons/snapdog/commit/2c737523527d4a167169d56f82209adcb08ad9eb))
* **docker:** expose TCP source ports 4953/4954 for dev mode ([4c56737](https://github.com/metaneutrons/snapdog/commit/4c56737ba245e7e8ad2aa7450202bfd7bc94a4f3))
* **docker:** use latest stable Rust in production Dockerfile ([814dd04](https://github.com/metaneutrons/snapdog/commit/814dd0439256408e18b6e710f0ee35946a5e18dc))
* eliminate group churn on client reconnect ([05ab4df](https://github.com/metaneutrons/snapdog/commit/05ab4df853f3c5af3d25a72f86aa8a4f2facc871))
* **eq:** merge EQ routes into zones nest, always show frequency curve ([dec5fbb](https://github.com/metaneutrons/snapdog/commit/dec5fbbb6304f600183dde9dfb7af2c2a81cf5dd))
* **http:** use Chrome user-agent for external HTTP requests ([808edb7](https://github.com/metaneutrons/snapdog/commit/808edb7826a4c48e2336ea21a3f4f60eb8ca4f5a))
* make snapcast types available for both backends ([c6a8b76](https://github.com/metaneutrons/snapdog/commit/c6a8b760f5d345260a17bee0ac3ad756f37126c9))
* make snapcast-embedded and snapcast-process mutually exclusive ([f1927d6](https://github.com/metaneutrons/snapdog/commit/f1927d6c716c1b2379c6a04b4a4c65a46696babb))
* **mqtt:** route all zone commands through ZonePlayer channels ([ab5b186](https://github.com/metaneutrons/snapdog/commit/ab5b186d7d8478de0531af47654c458b4ae15a90))
* **player:** eliminate two-step SetPlaylist+SetTrack race condition ([f1455e0](https://github.com/metaneutrons/snapdog/commit/f1455e0df38632a8ddba338b04414c3d88ea6274))
* **player:** preserve seek position — add offset for server-side seek ([bcf8f49](https://github.com/metaneutrons/snapdog/commit/bcf8f492319b525ad315ee52b32a5acd4a88f507))
* **player:** reset position offset on every stop_decode ([d77a3d0](https://github.com/metaneutrons/snapdog/commit/d77a3d0e6f20365223c12610e99ff71a4efeed9b))
* **player:** resume radio after pause ([6a52653](https://github.com/metaneutrons/snapdog/commit/6a5265361a440d903679bee7356bc31bead6fd18))
* **player:** seek while paused auto-resumes playback ([ff8ba2f](https://github.com/metaneutrons/snapdog/commit/ff8ba2f7b75126391b93cb68f7901bc5aeb394d4))
* **player:** set position offset on Subsonic resume after pause ([c0a7ab0](https://github.com/metaneutrons/snapdog/commit/c0a7ab0747f03ac6a568df71abee962f8d05fa89))
* **player:** SetTrack now works for radio sources ([f45dcba](https://github.com/metaneutrons/snapdog/commit/f45dcbaa0fe9700b0200ad2785144d09083b3d35))
* **player:** track playback position from symphonia packet timestamps ([ed43064](https://github.com/metaneutrons/snapdog/commit/ed4306461f5fe196d6badea5a3e0e883fb1a5a9f))
* **process:** fix snapserver config — port collision, dual-stack, retry ([12269e1](https://github.com/metaneutrons/snapdog/commit/12269e177eb7376c13b4d0c9c6376f6058ad879a))
* **radio:** preserve station name, show ICY data as artist ([9c29981](https://github.com/metaneutrons/snapdog/commit/9c29981eafd22eae3f1c6ca1e824dcecc0b6dd84))
* realtime pacing with now_usec() timestamp after sleep ([f0e0fc5](https://github.com/metaneutrons/snapdog/commit/f0e0fc571f808ef4b7ab5ac86716a6055e664c7c))
* remove hardcoded auto-start of first radio station ([77ec768](https://github.com/metaneutrons/snapdog/commit/77ec76837c764fbe1e73b4feb31a8bdf1803c5e0))
* resolve 10 TODOs — media endpoints, play/url bug, WS commands, MQTT dispatch ([7f86034](https://github.com/metaneutrons/snapdog/commit/7f860343301b01faebb11037c932df660e3c1e43))
* **snapcast:** add client_set_mute for mute-only commands ([bc07f89](https://github.com/metaneutrons/snapdog/commit/bc07f89e37579b8436e0dc87f63ab65ae5c3b248))
* **snapcast:** add group management to embedded event handler ([f54e2fb](https://github.com/metaneutrons/snapdog/commit/f54e2fb1fbad3d4885679ca67430b924cadde6b4))
* **snapcast:** handle Result from add_f32_stream ([898cc91](https://github.com/metaneutrons/snapdog/commit/898cc914d325800546a61edb91a307ff8ea4395c))
* **snapcast:** match groups to zones by stream ID, not group UUID ([332e74a](https://github.com/metaneutrons/snapdog/commit/332e74ad40f665789f0c67c7700af4210887b894))
* **snapcast:** per-zone mutex to avoid cross-zone contention on send_audio ([211d7b4](https://github.com/metaneutrons/snapdog/commit/211d7b475a5028812e9c309b8d637819acd356ae))
* **snapcast:** prefer connected client when same MAC has multiple entries ([11c147b](https://github.com/metaneutrons/snapdog/commit/11c147b505e392eea992cf4e86049c50afdc54d1))
* **snapcast:** preserve volume on mute and mute on volume change ([221959f](https://github.com/metaneutrons/snapdog/commit/221959f8eb4d2d00b45c65fd6b54fcbe41456179))
* **snapcast:** remove GroupClientsChanged, match C++ snapserver events ([5cf7906](https://github.com/metaneutrons/snapdog/commit/5cf79064fea7ea0c701aba33219bd6a4c11fbb5a))
* **snapcast:** resolve hostname for JSON-RPC connection, add startup delay ([dee1c15](https://github.com/metaneutrons/snapdog/commit/dee1c15d89768ba77762a80d1a5a5f2573face44))
* **snapcast:** retry initial JSON-RPC connect (10 attempts, 500ms backoff) ([0e98e7c](https://github.com/metaneutrons/snapdog/commit/0e98e7c3102f5401994f09e62456e8806cac827f))
* **snapcast:** serialize typed ServerStatus to JSON for backend trait ([0fef067](https://github.com/metaneutrons/snapdog/commit/0fef067f8b93c22b52b71094e9df952eb113bad0))
* **snapcast:** ServerOnUpdate only syncs group IDs, not zone_index ([19d248d](https://github.com/metaneutrons/snapdog/commit/19d248db8562872787dfef6f27fd1a90c903f77e))
* **snapcast:** sync client state after command execution ([b3e048f](https://github.com/metaneutrons/snapdog/commit/b3e048f6ad589c429b611c25fd9711701857b126))
* **snapcast:** sync client state at startup + handle all notifications ([1cf434e](https://github.com/metaneutrons/snapdog/commit/1cf434eaba2a35eea3af79afb02f5e61018fdb44))
* **snapcast:** sync client zone assignments from group membership at startup ([1459afa](https://github.com/metaneutrons/snapdog/commit/1459afa1f06e0f2272c8baf93e54cd2b71ace1a8))
* **snapcast:** sync client zone assignments on ServerOnUpdate ([ca5e7a0](https://github.com/metaneutrons/snapdog/commit/ca5e7a0c353f9c57774f06a96e3cd05d5b5ad711))
* **snapcast:** sync client zones after Group.SetClients via Server.GetStatus ([e2d0f7c](https://github.com/metaneutrons/snapdog/commit/e2d0f7ce2a98dcf988d8fbf96b23f4abe1f229dd))
* **snapcast:** sync group IDs on ServerUpdated in embedded event handler ([fe13b5d](https://github.com/metaneutrons/snapdog/commit/fe13b5d2a76d73ec8660694ae5fb9bdc85bd68bf))
* **snapcast:** use jsonrpc_port from config instead of streaming_port+1 ([0c12e12](https://github.com/metaneutrons/snapdog/commit/0c12e12b53874d871d04d7c2d42742cd8a643292))
* **subsonic:** request raw format to preserve original quality ([9df8029](https://github.com/metaneutrons/snapdog/commit/9df802928b818cc05d5cdcc4f740e43aa80803d4))
* use simple release type with toml jsonpath for workspace version ([cbd00f1](https://github.com/metaneutrons/snapdog/commit/cbd00f19733a127ac8b39c03e162d529e449caf3))
* **webui:** add 16px margin on each side for mobile zone card ([4936f5e](https://github.com/metaneutrons/snapdog/commit/4936f5e00addeb18f3ceda7fdae0ff0afc0bcd53))
* **webui:** always show controls in idle mode, hide sidebar on desktop ([07224ac](https://github.com/metaneutrons/snapdog/commit/07224acae8db7094c39325c71ab0dce886279caa))
* **webui:** auto-reload state on WebSocket reconnect + steering rules ([6472172](https://github.com/metaneutrons/snapdog/commit/647217211fd2d932cbfeec517e5defe4f22d8f25))
* **webui:** auto-retry cover art load after 1.5s for late-arriving AirPlay covers ([7963d0f](https://github.com/metaneutrons/snapdog/commit/7963d0f24cf2383d5ef872937070ac7322154e48))
* **webui:** cap client chip width at 256px (max-w-64) ([4ce0ad5](https://github.com/metaneutrons/snapdog/commit/4ce0ad59a469b7d0f3f91952b0a1331ff1b8f712))
* **webui:** client cards side by side with auto line-break ([5c288e1](https://github.com/metaneutrons/snapdog/commit/5c288e136587810bc61ecb2b23da8a4b4b6d789a))
* **webui:** client chip UX improvements ([10b41c7](https://github.com/metaneutrons/snapdog/commit/10b41c71b6629b2b7c2b526d10160ea890fc89e2))
* **webui:** clients collapsible but open by default on all viewports ([ba4da9b](https://github.com/metaneutrons/snapdog/commit/ba4da9b8617f2328f3fd3327e5773118f64b07f9))
* **webui:** collapse client list by default on mobile ([a7ac3a1](https://github.com/metaneutrons/snapdog/commit/a7ac3a12daea8afab4b809f2d98a08323b5ce153))
* **webui:** consistent controls column height across zone cards ([ec814aa](https://github.com/metaneutrons/snapdog/commit/ec814aa6491a464b5965fa3123aa19a22cf4e8e3))
* **webui:** consistent TrackInfo height across all zone cards ([b282bfb](https://github.com/metaneutrons/snapdog/commit/b282bfb8e9a1307421c8b139f8965943e4366c6b))
* **webui:** controls max-width drives zone card width limit ([45d478e](https://github.com/metaneutrons/snapdog/commit/45d478e4a9c18e4a3199b8d140118abcdfedc378))
* **webui:** handle API key change/removal — clear stale key on 401 ([1ed4881](https://github.com/metaneutrons/snapdog/commit/1ed4881f3755af350b26d050c69c19ae63372399))
* **webui:** hide playlist duration when 0:00 (e.g. radio playlist) ([2b3ef6d](https://github.com/metaneutrons/snapdog/commit/2b3ef6d5d933c71af27a87d01e0cc24a6fdc0deb))
* **webui:** hide ZoneHeader on mobile — tab bar already shows zone name ([0c79a24](https://github.com/metaneutrons/snapdog/commit/0c79a248e2966b5e11e6ac8da76910e6b8d659df))
* **webui:** increase cover art size from w-56 to w-64 for better proportion ([c61a264](https://github.com/metaneutrons/snapdog/commit/c61a264bf1c494dac7cdc6eca1b14dd1ec104be9))
* **webui:** make slider track visible against muted backgrounds ([6cfd211](https://github.com/metaneutrons/snapdog/commit/6cfd2119b2ca5dc2d03179b05372824b8d0bee0a))
* **webui:** mobile zone card takes full display width ([c20c69f](https://github.com/metaneutrons/snapdog/commit/c20c69fdfeaf0d2e5277df14cc62634842766b9e))
* **webui:** move clients and playlists to full-width below cover+controls ([3e9f81f](https://github.com/metaneutrons/snapdog/commit/3e9f81fa43bb7bd0934d5b66ba5daece996463ce))
* **webui:** move PlaylistBrowser below horizontal layout for full card width ([57b479b](https://github.com/metaneutrons/snapdog/commit/57b479b0cf309a626e2928dbf8a2b38829867952))
* **webui:** persist selected zone across refresh via sessionStorage ([e48ea52](https://github.com/metaneutrons/snapdog/commit/e48ea5298e8f284edb8cb80572a49669103d3356))
* **webui:** pin zone card max width to exactly 660px ([b176f5d](https://github.com/metaneutrons/snapdog/commit/b176f5d60f11e2138d199dd2778662f2efbaa0ee))
* **webui:** player controls use full width on mobile ([f3d8801](https://github.com/metaneutrons/snapdog/commit/f3d8801dce7e004c9aa4412a3414d4696958c7a1))
* **webui:** prevent chip drag when interacting with volume slider ([1d99201](https://github.com/metaneutrons/snapdog/commit/1d99201ca0d41c9133dfb6a6dd9cc52037eb5d61))
* **webui:** push cover_url update when cover fetch completes ([3e79ac8](https://github.com/metaneutrons/snapdog/commit/3e79ac80c3228cb50a7b16771a8029ca5b9885d9))
* **webui:** push cover_url via WebSocket on track change ([2bfaf22](https://github.com/metaneutrons/snapdog/commit/2bfaf220814be1390ba52997dd408ff57fbae936))
* **webui:** reliable cover art updates, always-square cover, plain img ([38bae54](https://github.com/metaneutrons/snapdog/commit/38bae54f3ed7ba723d3802f8f822acd7523af760))
* **webui:** responsive desktop grid with auto-fill columns ([ee2ec6b](https://github.com/metaneutrons/snapdog/commit/ee2ec6bd10b3130196bd16b8d375fd458fb2e6e9))
* **webui:** stepped reconnect backoff (1/2.5/5/10/15s) and wire onReconnect ([323bf2a](https://github.com/metaneutrons/snapdog/commit/323bf2a384e23dae572772716e1cd734f29be8b5))
* **webui:** stop cover art flickering on radio ICY metadata updates ([cdf93c8](https://github.com/metaneutrons/snapdog/commit/cdf93c8d9eada42fe067598fd69225067c1d4e03))
* **webui:** stronger sunken effect for client cards ([be73cdf](https://github.com/metaneutrons/snapdog/commit/be73cdff9ecbca4341b58c36344686ba38a6c492))
* **webui:** sunken client cards with drag handle for visual affordance ([5c9456f](https://github.com/metaneutrons/snapdog/commit/5c9456f85433e36ea95ed5445c8eef573d81ec9e))
* **webui:** sync seek bar on playback state change ([855a61b](https://github.com/metaneutrons/snapdog/commit/855a61b5100637289df90ea13f3268c487267ebc))
* **webui:** truly responsive zone cards with flex-wrap layout ([6fc1730](https://github.com/metaneutrons/snapdog/commit/6fc1730be7fd653b9ca1c5c0522c463cf011a339))
* **webui:** use CSS grid for equal-width client chips ([0f2cec1](https://github.com/metaneutrons/snapdog/commit/0f2cec16c96e89f091d4b7c690dc80c5ca96bed7))
* **webui:** use object-contain for cover art + update NDR 1 cover URL ([3405a8e](https://github.com/metaneutrons/snapdog/commit/3405a8e9464b9605b90a46fe0409fb0eb2c5801d))
* **webui:** zone card layout audit improvements ([5dc964f](https://github.com/metaneutrons/snapdog/commit/5dc964f217acf58f0d6fb3de42be2e9d55f2f0ea))
* **webui:** zone card max width to 600px ([e40c9eb](https://github.com/metaneutrons/snapdog/commit/e40c9ebd143b7331cbe188e3554b8ba315197581))
* **ws:** trigger ZoneTrackChanged on artist change too ([2496d3c](https://github.com/metaneutrons/snapdog/commit/2496d3c309dd75a3afac9cc5c36b7579a0a35a83))


### Performance Improvements

* **audio:** eliminate unnecessary allocations in PCM pipeline ([f450e0b](https://github.com/metaneutrons/snapdog/commit/f450e0b5b0b2ee197f4100fbbed5554b0acd6af5))
* **player:** reduce config cloning in ZonePlayer select loop ([de76b46](https://github.com/metaneutrons/snapdog/commit/de76b461534b2fa3ec2d8277c8a0202ad3805b69))
