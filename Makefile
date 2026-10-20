# Canonical commands. CI runs the same ones; keep them in sync.
# Offline targets exclude the database-backed examples (they need Postgres).

export PATH := /opt/homebrew/bin:$(HOME)/.cargo/bin:$(PATH)

OFFLINE := --workspace --exclude example-database --exclude example-auth

.PHONY: ci fmt fmt-check lint arch-lint deny test nextest build run-rest run-streaming run-websocket clean

## ci: what GitHub Actions runs for the offline baseline
ci: fmt-check lint arch-lint test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --locked --all-targets --all-features $(OFFLINE) -- -D warnings

## arch-lint: enforce the AGENTS.md §2 layering boundaries on app code
arch-lint:
	./scripts/arch-lint.sh

## deny: check advisories, licenses, and bans with cargo-deny
deny:
	cargo deny check

## sbom: generate Software Bill of Materials (SPDX 2.3 JSON)
sbom:
	./scripts/generate-sbom.sh

test:
	cargo test --locked --all-features $(OFFLINE)

## nextest: faster offline test run (opt-in; `cargo install cargo-nextest`). Same scope
## as `test`; `cargo test` stays the zero-install default so a clean checkout always works.
nextest:
	cargo nextest run --locked --all-features $(OFFLINE)

build:
	cargo build --locked $(OFFLINE)

run-rest:
	HEPHOS_ADDR=127.0.0.1:8080 cargo run -p example-rest

run-streaming:
	HEPHOS_ADDR=127.0.0.1:8080 cargo run -p example-streaming

run-websocket:
	HEPHOS_ADDR=127.0.0.1:8080 cargo run -p example-websocket

clean:
	cargo clean
