.PHONY: setup check fmt clippy test webui-check dev build-webui build-all knxprod

# ── KNX Product Database ───────────────────────────────────────

## Generate SnapDog.knxprod (XML + signed archive in one step)
knxprod:
	cargo run -p xtask -- knx/snapdog.xml

## First-time setup: configure git hooks
setup:
	git config core.hooksPath .githooks
	@echo "✅ Git hooks configured"

## Run all checks (same as CI)
check: fmt clippy test webui-check

fmt:
	cargo fmt -- --check

clippy:
	cargo clippy -- -D warnings

test:
	cargo test

webui-check:
	cd webui && npm ci && npm run i18n:check && npm run lint && npm run typecheck && npm run build

## Build WebUI (Next.js static export)
build-webui:
	cd webui && npm ci && npm run build

## Build everything: WebUI, Rust binary, and KNX product database
build-all: build-webui knxprod
	cargo build --release

## Start dev environment
dev:
	docker compose -f docker-compose.dev.yml up -d
	@echo "✅ Dev infrastructure running"
	@echo "   Run: cargo run -- --config snapdog.dev.toml"
