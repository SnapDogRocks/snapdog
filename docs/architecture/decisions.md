# Architecture Decision Records

This file is the formal log of architecture decisions for SnapDog. Earlier
decisions (ADR-001…018) are referenced inline across the code and RFCs and will be
back-filled here over time; the formal log begins at ADR-019.

Each entry follows a MADR-minimal shape: **Status · Context · Decision ·
Consequences · References**.

---

## ADR-019 — Pin snapcast at 0.16.1 until the seam firewall is complete

- **Status:** Accepted (2026-06-30)
- **Deciders:** maintainer
- **Tracking:** RFC 0003 `IT-T08` (this decision), `IT-NG-05` (the upgrade itself)

### Context

`snapcast-rs` 0.17 carries breaking changes to the `snapcast-server` /
`snapcast-proto` API (e.g. the `init()` / `state()` removal that already bit once
during refactoring). SnapDog currently pins **0.16.1** from crates.io
(`snapcast-server`, `snapcast-client`, `snapcast-proto` in the workspace
`Cargo.toml`), and a local `snapcast-rs` 0.17 checkout exists as path-deps but is
**not** adopted.

The risk that makes this decision necessary: the embedded-server JSON-RPC control
seam shares **no types** between snapdog and the dependency — drift is therefore
**compiler-invisible** and surfaces only at runtime. A breaking version jump made
without a regression net would land silently.

### Decision

1. **Stay pinned at snapcast 0.16.1** for `snapcast-server` / `snapcast-client` /
   `snapcast-proto`; do **not** adopt the local 0.17 path-deps yet.
2. **Keep the snapdog seam firewall version-agnostic** — it stands on its own
   golden JSON-RPC request vectors, the `ServerEvent → SnapcastEvent` map coverage,
   and the mocked `SnapcastBackend` trait, none of which depend on a snapcast 0.17
   test harness.
3. **Sequencing rule:** the 0.16.1 → 0.17 upgrade (`IT-NG-05`) happens **only after**
   the seam firewall is complete and green — `IT-T52` (event map), `IT-T54` (17
   JSON-RPC vectors), `IT-T55` (F32 sender), **and `IT-T73`** (feature build-smoke
   matrix). The upgrade is then performed as separate work, behind that firewall,
   so any wire/signature drift fails a test rather than shipping.

### Consequences

- The breaking jump is deferred but de-risked: the regression net catches
  method/params drift (golden vectors), event-mapping drift (exhaustive coverage),
  and gross signature breaks across every feature combination (build-smoke).
- SnapDog tracks a stable crates.io release rather than a moving local checkout —
  reproducible builds, no surprise breakage from upstream development.
- Cost: SnapDog forgoes 0.17 features/fixes until the upgrade is scheduled. Accepted
  — correctness of the audio control plane outranks early adoption.

### References

- RFC `docs/rfcs/0003-integration-test-suite.md` — §9.1 crate-contract firewall,
  §15 roadmap `IT-NG-05`, tasks `IT-T52` / `IT-T54` / `IT-T55` / `IT-T73`.
- Workspace `Cargo.toml` — the `snapcast-*` 0.16.1 pins.
- `snapdog/tests/snapcast_rpc.rs` — the golden JSON-RPC wire firewall.

---

## ADR-020 — Defer cpal 0.18 / alsa 0.12 and accept the rustls-webpki tracker (upstream-blocked bumps)

- **Status:** Accepted (2026-07-10)
- **Deciders:** maintainer
- **Tracking:** this ADR; Dependabot alert #21; revisit triggers below.

### Context

The 2026-07 breaking-dependency pass upgraded most `0.x` deps cleanly and verified
each with the pre-push hook (`cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`): **reqwest 0.13**
(TLS backend → rustls+aws-lc), **testcontainers 0.27**, **rubato 4.0**,
**symphonia 0.6**, **tower-http 0.7**, **md5 0.8**, **mockall 0.15**, plus the
`fix(api)` rustls crypto-provider install. Three bumps could **not** land — each is
blocked by an upstream release, not by our code:

1. **cpal 0.17 → 0.18.** cpal 0.18.1 declares `mach2 = "^0.6"` (gated to
   `cfg(target_vendor = "apple")`) but does **not compile** against the only
   published mach2 0.6.0 — a type/borrow error inside cpal's own
   `src/timestamp.rs`. No fixed cpal (0.18.2+) or mach2 (0.6.1+) exists. On Linux
   (the receiver + CI) cpal 0.18 is fine (mach2 is apple-only), but it **breaks
   local macOS dev builds** of `snapdog-client`.
2. **alsa 0.11 → 0.12.** Coupled to cpal: cpal 0.18 still depends on `alsa ^0.11`,
   so a direct `alsa 0.12` bump collides on the `alsa-sys` `links = "alsa"`
   invariant. And cpal itself is deferred (above), so there is no path to 0.12.
3. **rustls-webpki (Dependabot high #21).** The vulnerable `rustls-webpki 0.102.8`
   (GHSA-82j2-j2ch-gfr8, DoS panic on a malformed CRL) enters **only** via
   `rumqttc 0.25.1` (the latest rumqttc), which pins `rustls-webpki ^0.102` and
   cannot take the patched 0.103.x. The other lock copy (0.103.x via reqwest) is
   already patched.

### Decision

1. **Keep `cpal` at 0.17** (`snapdog-client/Cargo.toml`) until cpal ships a build
   that compiles with mach2 0.6, or mach2 releases a fix. Do not trade a working
   local macOS dev build for a marginal audio-output bump.
2. **Keep `alsa` at 0.11**; it moves only together with a working cpal 0.18.
3. **Keep Dependabot #21 open as an accepted tracker.** Dismiss the related low/med
   rustls-webpki alerts as `tolerable_risk`; the DoS is reachable only over
   TLS-MQTT against a hostile/MITM broker presenting a crafted CRL — an already
   privileged position — so the residual risk is tolerable until rumqttc updates.

### Consequences

- SnapDog forgoes cpal 0.18 / alsa 0.12 and the rustls-webpki patch until upstream
  moves. Accepted: a broken local macOS build and a hostile-broker-only DoS both
  outweigh the value of the bumps.
- The blockers are **compiler-/CI-safe as-is**: Linux builds are unaffected by the
  cpal/alsa pins, and `cargo …-locked` passes with the current lock.

### Revisit triggers (roadmap)

- **cpal:** a cpal release that compiles with mach2 0.6 (watch RustAudio/cpal
  releases and the mach2 0.6.x line) → then bump cpal 0.18 **and** re-evaluate
  alsa 0.12 in the same change.
- **alsa:** unblocked by the cpal revisit above (or a cpal that moves to alsa 0.12).
- **rustls-webpki #21:** a rumqttc release (> 0.25.1) whose rustls chain uses
  rustls-webpki ≥ 0.103.13 → then `cargo update -p rumqttc`, confirm #21 clears,
  and drop this tracker.

### References

- Workspace `Cargo.toml` / `snapdog-client/Cargo.toml` — the cpal 0.17 / alsa 0.11
  pins; the completed bumps (rubato 4.0, symphonia 0.6, tower-http 0.7, md5 0.8,
  mockall 0.15).
- Migration commits: `0ea85df` (rubato/md5/mockall), `71088ee` (tower-http),
  `b716472` (symphonia), `a0c655c` (reqwest 0.13), `6d0f4a3` (testcontainers),
  `dff2c39` (rustls crypto-provider `fix(api)`).
- Dependabot: `SnapDogRocks/snapdog` alert #21 (rustls-webpki, kept open).
