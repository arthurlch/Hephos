# Canonical commands. CI runs the same ones; keep them in sync.
# Offline targets exclude the database-backed examples (they need Postgres).

OFFLINE := --workspace --exclude example-database --exclude example-auth

.PHONY: ci fmt fmt-check lint test build run-rest run-streaming run-websocket clean

## ci: what GitHub Actions runs for the offline baseline
ci: fmt-check lint test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --locked --all-targets --all-features $(OFFLINE) -- -D warnings

test:
	cargo test --locked --all-features $(OFFLINE)

build:
	cargo build --locked $(OFFLINE)

run-rest:
	RIVET_ADDR=127.0.0.1:8080 cargo run -p example-rest

run-streaming:
	RIVET_ADDR=127.0.0.1:8080 cargo run -p example-streaming

run-websocket:
	RIVET_ADDR=127.0.0.1:8080 cargo run -p example-websocket

clean:
	cargo clean
