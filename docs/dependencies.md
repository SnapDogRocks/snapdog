# Rust dependency maintenance

## Stable update baseline (2026-09-06)

Direct dependencies are checked against the crates.io stable releases, including
incompatible upgrades. The workspace lockfile is then refreshed for all target
platforms. "Latest" for a transitive dependency means the latest release allowed
by its upstream parent's constraints, not an arbitrary forced major version.

This update moves the KNX crates to 0.9.1, shairplay to 0.9.1 and testcontainers to
0.28.0, and refreshes the other compatible locked dependencies. The Mosquitto
integration test uses testcontainers' `GenericImage` with the same image, command
and readiness condition as before: testcontainers-modules 0.15 still requires
testcontainers 0.27. The server and client require Rust 1.94; release builds remain
on the existing pinned Rust 1.97.0 toolchain.

Raising the declared MSRV also enables Clippy suggestions that were previously
inapplicable: let-chains, const functions and duration constructors. The source
changes accompanying the manifest update are mechanical adaptations to those
rules; the audio protocol and service behavior are unchanged.

A follow-up registry check also picked up shairplay 0.9.1 and ipnet 2.12.2.

## Stable update (2026-09-12)

The KNX crates move to 0.9.2 and shairplay to 0.10.0; reqwest 0.13.5, toml 1.1.6,
uuid 1.26.1 and the remaining compatible transitive crates are refreshed. Both
holds below still apply: vergen stays pinned at 9.0.6 and alsa at 0.11.0.

knx-rs 0.9.2 derives the group-value wire form from the DPT wire size instead of
inferring it from the encoded bytes. Under 0.9.1 any one-byte payload of 0x3F or
less was packed into the six-bit APCI data field, so a DPT 5.001 volume below
25 % or a DPT 5.010 count below 64 left the bus as a short telegram that a
one-byte group object cannot read. Client mode publishes every status through
`group_write_value` (`snapdog/src/knx/client.rs`), so it inherits the fix with no
source change.

The same release deprecates `GroupOps::group_write` and `GroupOps::group_respond`,
whose successors are `group_write_raw` and `group_respond_raw` with an explicit
`GroupValuePayload`, or the DPT-aware `group_write_value` and
`group_respond_value`. Neither deprecated method has a call site in this
workspace, which the Clippy run with warnings denied confirms.

Device mode is not covered by that fix. It encodes through knx-rs-device, whose
`application_layer::encode::encode_group_value` still applies the old length
inference, so one-byte status objects below 64 remain affected until that crate
adopts the same `DptWireSize` rule.

shairplay 0.10.0 adds an opt-in `pipewire-auth-setup-compat` feature and bounds
RTSP request ingestion before body dispatch. The public API is unchanged and the
new feature stays off. Its plist dependency moves to 1.10.1, which drops the
duplicate quick-xml 0.41 copy from the graph; librespot-core still holds
quick-xml 0.38.4, so the two ignored advisories for it remain.

### Validation (knx-rs 0.9.2, shairplay 0.10.0)

- macOS ARM64, Rust 1.97.0 from `rust-toolchain.toml`: 354 workspace tests passed
  under nextest, including the KNX golden wire contract and the xtask knxprod
  artifact-freshness test.
- Clippy with warnings denied passed for the workspace and for the process-mode
  Snapcast build. Formatting is clean.
- `cargo deny check` passed for advisories, bans, licences and sources with the
  existing exception list. The three rustls-webpki ignores no longer match any
  crate in the graph; that predates this update, because the lockfile already
  resolved rustls-webpki 0.103.15 on main.
- `cargo run -p xtask -- knx/snapdog.xml` regenerated the product database.
  knx-rs-prod 0.9.2 indents the `ChannelIndependentBlock` children consistently,
  so the committed `knx/snapdog.xml` changes by whitespace only (verified by
  comparing both files with all spaces and tabs stripped) and `knx/snapdog.knxprod`
  grows by the same 36 bytes. The ETS application version is unchanged.
- Linux, Windows and the tier-2 service tests were not run locally; CI covers
  them.

## Explicit upstream holds

| Crate | Held version | Reason and removal condition |
| --- | --- | --- |
| `alsa` | 0.11.0 | The latest CPAL 0.18.2 uses ALSA 0.11 / alsa-sys 0.4. ALSA 0.12.1 uses alsa-sys 0.6; Cargo rejects two crates with `links = "alsa"`. Upgrade when CPAL adopts the new ALSA family. |
| `vergen` | 9.0.6 in Cargo.lock | librespot-core 0.8.0 uses vergen-gitcl 1.0.8 / vergen-lib 0.1.6. vergen 9.1.0 switches to vergen-lib 9.1.0, producing incompatible `Add` traits in librespot's build script. Retest after an upstream librespot/vergen-gitcl fix. |

Do not delete these holds or regenerate the lockfile without compiling the
default Spotify-enabled build. Do not use prereleases or local forks merely to
report that every version number is newest.

## Security exceptions

An unfiltered `cargo audit` still reports three vulnerabilities, all through
librespot 0.8.0. This is **not** a vulnerability-free dependency graph:

- `RUSTSEC-2023-0071`: rsa 0.9.10, Marvin timing attack; no patched stable release.
- `RUSTSEC-2026-0194`: quick-xml 0.38.4, quadratic duplicate-attribute checking.
- `RUSTSEC-2026-0195`: quick-xml 0.38.4, unbounded namespace allocation.

The quick-xml issues are fixed in 0.41+, but librespot-core 0.8.0 requires 0.38.
These are runtime dependencies when Spotify is enabled, not merely test tools.
CI and nightly auditing retain only these three existing exceptions. Ten obsolete
exceptions have been removed. Review the unfiltered audit on every update; a
successful audit with exceptions does not establish that these issues are safe.

## Update and verification

```sh
# cargo-upgrade is supplied by cargo-edit. Review before applying manifest changes.
cargo upgrade --dry-run --incompatible allow --pinned allow --ignore-rust-version
cargo update
# Necessary until the vergen compatibility issue above is resolved:
cargo update -p vergen --precise 9.0.6
cargo fmt --all -- --check
SKIP_WEBUI_BUILD=1 cargo check --workspace --all-targets --locked
SKIP_WEBUI_BUILD=1 cargo clippy --workspace --all-targets --locked -- -D warnings
SKIP_WEBUI_BUILD=1 cargo clippy -p snapdog --no-default-features --features snapcast-process --all-targets --locked -- -D warnings
SKIP_WEBUI_BUILD=1 cargo nextest run --workspace --locked
SKIP_WEBUI_BUILD=1 cargo nextest run -p snapdog --no-default-features --features snapcast-process --locked
SKIP_WEBUI_BUILD=1 cargo test -p snapdog --features test-harness --test zone_player --locked
SKIP_WEBUI_BUILD=1 cargo test -p snapdog --features ap2 --lib receiver::airplay --locked
SKIP_WEBUI_BUILD=1 cargo test -p snapdog --test mqtt_tier2 --locked -- --nocapture
cargo deny check
```

The MQTT test must actually exercise Docker/Mosquitto: a `SKIP IT-T32` message is
not successful broker verification. CI also checks Linux, Windows, macOS and KNX
product generation. Local macOS checks cannot substitute for the other targets.

With Colima, explicitly select its Docker socket for the MQTT test if
`/var/run/docker.sock` is absent:

```sh
export DOCKER_HOST="$(docker context inspect --format '{{.Endpoints.docker.Host}}')"
```

### Initial validation (shairplay 0.9.0)

- macOS ARM64: 353 workspace tests, 8 test-harness zone-player tests and 7 AirPlay
  receiver tests with `ap2` enabled passed. The separate MQTT test passed against
  real Mosquitto under Colima without a skip.
- Clippy with warnings denied: workspace/default and external Snapcast modes.
- Rust 1.94.0: workspace/all-targets and external Snapcast checks passed.
- Linux ARM64 (Rust 1.98 container): the refreshed dependency graph compiled for
  workspace/all-targets with `snapdog/ap2`, including the ALSA-backed client.
- Workflow lint passed. The security audit passed with exactly the three
  documented exceptions; an unfiltered audit still reports those advisories.
- KNX XML and an unsigned `.knxprod` were generated successfully with xtask.
- The macOS client check reported zero future-incompatible dependencies;
  the old `block` 0.1.6 dependency has not returned.

Windows, physical audio hardware and live Spotify/AirPlay sessions were not
tested locally. Keep the repository's platform checks before merging.

### Follow-up validation (shairplay 0.9.1, ipnet 2.12.2)

The 353 workspace tests and 7 AirPlay receiver tests passed again, as did
workspace Clippy with warnings denied and a Rust 1.94.0 workspace/all-targets
check with `snapdog/ap2`. Auditing still requires the same three exceptions.
The registry recheck found no further direct stable upgrades except ALSA;
a full lockfile-update dry run offered only the incompatible vergen 9.1.0 /
vergen-lib 9.1.0 combination. Linux and Windows checks were not rerun for these
two patch updates.
