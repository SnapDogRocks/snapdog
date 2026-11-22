.PHONY: setup check fmt clippy test webui-check dev build-webui build-all knxprod

# ── KNX Product Database ───────────────────────────────────────

## Generate SnapDog.knxprod (XML + signed archive in one step)
knxprod:
	cargo xtask knx/snapdog.xml

## First-time setup: configure git hooks
setup:
	lefthook install
	@echo "✅ Git hooks configured (lefthook)"

## Run all checks (same as CI): the steps of the rust-core and webui jobs in
## .github/workflows/ci.yml, read from that file by `cargo xtask ci`.
check:
	cargo xtask ci

## Quick checks for the edit loop (a subset of `check`)
fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --locked --all-targets -- -D warnings

test:
	cargo nextest run --workspace --locked

webui-check:
	cd webui && pnpm install --frozen-lockfile && pnpm run i18n:check && pnpm run lint && pnpm run typecheck && pnpm run test && pnpm run build

## Build WebUI (Next.js static export)
build-webui:
	cd webui && pnpm install --frozen-lockfile && pnpm run build

## Build everything: WebUI, Rust binary, and KNX product database
build-all: build-webui knxprod
	cargo build --release

## Start dev environment
dev:
	docker compose -f docker-compose.dev.yml up -d
	@echo "✅ Dev infrastructure running"
	@echo "   Once: cp snapdog.example.toml snapdog.dev.toml (gitignored, then edit)"
	@echo "   Run:  cargo run -p snapdog -- --config snapdog.dev.toml"
