<!--
Hephos is agent-first: this checklist encodes the invariants so every change — human
or agent — is verified the same way. Tick each box or say why it does not apply.
-->

## What & why

<!-- One or two sentences. What does this change and what problem does it solve? -->

## Checklist

Validation (CLAUDE.md §11 — all must pass locally):
- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test` (offline suite green)
- [ ] For DB/ignored/new-transport work: I **ran the app and observed it**, not just compiled it.

Doctrine (see CLAUDE.md / CONVENTIONS.md):
- [ ] New code is in the right directory and follows the nearest example.
- [ ] One error type (`hephos::Error`); no new error enums in app code.
- [ ] Every new/changed endpoint has a `TestClient` success **and** failure test.
- [ ] A `routes_build` test exists for any app whose routes changed.
- [ ] No `unsafe`, no `Arc<Mutex<_>>` app state, no detached `tokio::spawn` in handlers.
- [ ] No new dependency (or: justified in one line below).
- [ ] No `// BOUNDARY:` left in application code (they belong only in `crates/`).
- [ ] If a framework contract changed, `CONVENTIONS.md`/`ARCHITECTURE.md` and the
      relevant example were updated in this PR — no document now contradicts the code.

## Dependency justification (if any added)

<!-- crate = "reason it's needed and the standard stack can't do it" -->
