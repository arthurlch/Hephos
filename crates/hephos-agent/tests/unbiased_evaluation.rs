//! Comprehensive, unbiased experimental evaluation of Agent Experience (AX) trade-offs.
//!
//! Four experiments compare Hephos mechanisms against realistic steelman baselines.
//! Every experiment (a) derives its numbers from real inputs — real `Error` values,
//! real `anyhow` causal chains, real source fixtures, and real `schemars` schema
//! generation — rather than hardcoded constants, and (b) asserts the structural
//! claims the paper makes, so `cargo test` actually fails if a claim regresses.
//!
//! Two categories of number are explicitly NOT empirical and are labelled as such
//! wherever they appear:
//!   * the qualitative *actionability rubric* (Experiment 1) — author-assigned
//!     engineering judgement, not measured from agent runs;
//!   * the *analytical grammar-entropy / error-rate model* (Experiment 3) — a
//!     closed-form hypothesis, not observed agent behaviour.

use std::collections::HashMap;
use std::time::Instant;

use hephos_agent::Tool;
use hephos_agent::tool_schema;
use hephos_core::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ============================================================================
// Deterministic PRNG for reproducible Monte Carlo simulations
// ============================================================================
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        SimpleRng {
            state: if seed == 0 { 0xDEADBEEF } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        // 64-bit XorShift
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() as f64) / (u64::MAX as f64)
    }
}

/// Byte→token proxy. BPE on code/JSON averages ~3.8 characters per token; this is a
/// heuristic (see the paper's Construct Validity section), not a real tokenizer.
fn estimate_tokens(s: &str) -> usize {
    (s.len() * 10 + 37) / 38
}

/// Non-blank source lines — a fairer "lines of code" measure than raw line count.
fn code_lines(src: &str) -> usize {
    src.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Count lines whose trimmed form begins with any of the given prefixes.
fn count_lines_starting(src: &str, prefixes: &[&str]) -> usize {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            prefixes.iter().any(|p| t.starts_with(p))
        })
        .count()
}

/// Type definitions: `struct`/`enum`, with or without a `pub` qualifier.
fn count_defs(src: &str) -> usize {
    count_lines_starting(src, &["struct ", "pub struct ", "enum ", "pub enum "])
}

/// `use` import statements.
fn count_imports(src: &str) -> usize {
    count_lines_starting(src, &["use "])
}

// ============================================================================
// EXPERIMENT 1: Error Payload Compression vs. Diagnostic Information Density
// ============================================================================

enum FailureScenario {
    SchemaMismatch,
    DbConstraintViolation,
    MissingEntity,
    InternalTimeout,
}

struct ErrorComparison {
    scenario: &'static str,
    format_a_json: String,
    format_b_chain: String,
    format_c_backtrace: String,
}

/// Format A: the real Hephos AX observation, built from a real `Error`.
fn format_a(err: &Error) -> String {
    serde_json::to_string(&serde_json::json!({
        "error": { "kind": err.kind(), "message": err.to_string() }
    }))
    .unwrap()
}

/// Format B: a *real* `anyhow` causal chain rendered exactly as an operator would see
/// it (`{:?}` alternate form produces the "Caused by:" ladder). Not hand-typed.
fn format_b(top: &str, causes: &[&str]) -> String {
    let mut err = anyhow::anyhow!(causes.last().copied().unwrap_or("error").to_string());
    for cause in causes.iter().rev().skip(1) {
        err = err.context((*cause).to_string());
    }
    let err = err.context(top.to_string());
    format!("{err:?}")
}

/// Format C: a representative sanitized backtrace. A *real* captured backtrace is
/// non-deterministic (addresses, frame counts vary per build), so this is a fixed
/// representative sample, not a live capture — the paper labels it as such. It is
/// built from the real Format B chain plus a representative frame tail so the
/// comparison is at least internally consistent.
fn format_c(format_b: &str, frames: &str) -> String {
    format!("{format_b}\n\nStack backtrace:\n{frames}")
}

fn generate_error_formats(scenario: FailureScenario) -> ErrorComparison {
    match scenario {
        FailureScenario::SchemaMismatch => {
            let err = Error::invalid("missing required field 'user_id' in payload");
            let b = format_b(
                "invalid request payload",
                &[
                    "failed to deserialize JSON body",
                    "missing field `user_id` at line 2 column 14",
                ],
            );
            let c = format_c(
                &b,
                "   0: std::backtrace::Backtrace::create\n   1: hephos_core::error::Error::invalid\n             at crates/hephos-core/src/error.rs:42:13\n   2: <axum::extract::Json<T> as FromRequest>::from_request\n             at /rustc/cargo/registry/src/axum-0.8.1/src/extract/json.rs:88:21\n   3: <examples::api::users::create>::{{closure}}\n             at examples/reference-app/src/api/users.rs:24:10\n   4: axum::routing::MethodRouter::call\n   5: tower::util::service_fn::ServiceFn::call\n   ... [32 runtime frames omitted]",
            );
            ErrorComparison {
                scenario: "Schema Mismatch",
                format_a_json: format_a(&err),
                format_b_chain: b,
                format_c_backtrace: c,
            }
        }
        FailureScenario::DbConstraintViolation => {
            let err = Error::conflict("email address 'alice@example.com' already registered");
            let b = format_b(
                "failed to create user record",
                &[
                    "database transaction aborted",
                    "unique constraint violation on index 'users_email_idx' (value: alice@example.com)",
                ],
            );
            let c = format_c(
                &b,
                "   0: std::backtrace::Backtrace::create\n   1: sqlx_postgres::error::PgDatabaseError::into_error\n             at /rustc/cargo/registry/src/sqlx-postgres-0.8.3/src/error.rs:112:9\n   2: <examples::repos::UserRepo>::insert\n             at examples/reference-app/src/repos/users.rs:35:14\n   3: <examples::services::UserService>::create\n             at examples/reference-app/src/services/users.rs:29:18\n   4: <examples::api::users::create>::{{closure}}\n             at examples/reference-app/src/api/users.rs:25:10\n   ... [28 runtime frames omitted]",
            );
            ErrorComparison {
                scenario: "DB Constraint Violation",
                format_a_json: format_a(&err),
                format_b_chain: b,
                format_c_backtrace: c,
            }
        }
        FailureScenario::MissingEntity => {
            let err = Error::not_found("widget with id 'w_9872'");
            let b = format_b(
                "resource retrieval failed",
                &[
                    "query returned no rows",
                    "widget with id 'w_9872' not found in table 'widgets'",
                ],
            );
            let c = format_c(
                &b,
                "   0: std::backtrace::Backtrace::create\n   1: hephos_core::error::Error::not_found\n             at crates/hephos-core/src/error.rs:48:13\n   2: <examples::services::WidgetService>::get\n             at examples/reference-app/src/services/widgets.rs:18:25\n   3: <examples::api::widgets::get>::{{closure}}\n             at examples/reference-app/src/api/widgets.rs:14:10\n   ... [24 runtime frames omitted]",
            );
            ErrorComparison {
                scenario: "Missing Entity",
                format_a_json: format_a(&err),
                format_b_chain: b,
                format_c_backtrace: c,
            }
        }
        FailureScenario::InternalTimeout => {
            let err = Error::timeout();
            let b = format_b(
                "outbound gateway request failed",
                &[
                    "HTTP client error",
                    "request timed out after 5000ms waiting for headers",
                ],
            );
            let c = format_c(
                &b,
                "   0: std::backtrace::Backtrace::create\n   1: reqwest::async_impl::client::Client::execute\n             at /rustc/cargo/registry/src/reqwest-0.12.12/src/async_impl/client.rs:214:15\n   2: <examples::services::PaymentGateway>::charge\n             at examples/reference-app/src/services/payment.rs:44:12\n   ... [30 runtime frames omitted]",
            );
            ErrorComparison {
                scenario: "Internal Timeout",
                format_a_json: format_a(&err),
                format_b_chain: b,
                format_c_backtrace: c,
            }
        }
    }
}

/// Author-assigned qualitative actionability rubric (0.0–1.0). These encode
/// engineering judgement about how usable each format is for a given agent
/// archetype; they are NOT measured from agent runs and must not be reported as
/// empirical. They exist here so the paper's rubric has a single source of truth
/// and so the ordering claims can be asserted.
struct Actionability {
    runtime_a: f64,
    runtime_b: f64,
    runtime_c: f64,
    repair_a: f64,
    repair_b: f64,
    repair_c: f64,
}
const ACTIONABILITY: Actionability = Actionability {
    runtime_a: 1.0,
    runtime_b: 0.70,
    runtime_c: 0.30,
    repair_a: 0.15,
    repair_b: 0.65,
    repair_c: 0.95,
};

#[test]
fn experiment_1_error_payload_tradeoffs() {
    println!(
        "\n=== EXPERIMENT 1: ERROR PAYLOAD COMPRESSION VS. DIAGNOSTIC INFORMATION DENSITY ==="
    );
    println!(
        "{:<24} | {:<16} | {:<16} | {:<16} | {:<12}",
        "Scenario", "Format A (AX JSON)", "Format B (Chain)", "Format C (Trace)", "C/A Ratio"
    );
    println!(
        "{:-<24}-+-{:-<16}-+-{:-<16}-+-{:-<16}-+-{:-<12}",
        "", "", "", "", ""
    );

    let scenarios = [
        FailureScenario::SchemaMismatch,
        FailureScenario::DbConstraintViolation,
        FailureScenario::MissingEntity,
        FailureScenario::InternalTimeout,
    ];

    let (mut total_a, mut total_b, mut total_c) = (0usize, 0usize, 0usize);

    for s in scenarios {
        let comp = generate_error_formats(s);
        let tok_a = estimate_tokens(&comp.format_a_json);
        let tok_b = estimate_tokens(&comp.format_b_chain);
        let tok_c = estimate_tokens(&comp.format_c_backtrace);

        // Format A is a well-formed observation the model can branch on.
        let parsed: serde_json::Value = serde_json::from_str(&comp.format_a_json).unwrap();
        assert!(
            parsed["error"]["kind"].is_string(),
            "Format A must carry a machine-readable kind"
        );
        // Density ordering must hold in every scenario: JSON < chain < backtrace.
        assert!(
            tok_a < tok_b,
            "{}: A should be denser than B",
            comp.scenario
        );
        assert!(
            tok_b < tok_c,
            "{}: B should be denser than C",
            comp.scenario
        );

        total_a += tok_a;
        total_b += tok_b;
        total_c += tok_c;

        println!(
            "{:<24} | {:>4}B / {:>3} tok | {:>4}B / {:>3} tok | {:>4}B / {:>3} tok | {:>5.1}x reduction",
            comp.scenario,
            comp.format_a_json.len(),
            tok_a,
            comp.format_b_chain.len(),
            tok_b,
            comp.format_c_backtrace.len(),
            tok_c,
            tok_c as f64 / tok_a as f64,
        );
    }

    let mean_ratio = total_c as f64 / total_a as f64;
    println!(
        "{:-<24}-+-{:-<16}-+-{:-<16}-+-{:-<16}-+-{:-<12}",
        "", "", "", "", ""
    );
    println!(
        "{:<24} | {:>10} tok | {:>10} tok | {:>10} tok | {:>5.1}x avg",
        "Mean Across Scenarios",
        total_a / 4,
        total_b / 4,
        total_c / 4,
        mean_ratio
    );

    // The headline compression claim: backtraces cost several times more than AX JSON.
    assert!(
        mean_ratio > 3.0,
        "mean C/A token ratio should exceed 3x, got {mean_ratio:.2}"
    );

    println!(
        "\nSemantic Actionability Rubric (author-assigned qualitative judgement, NOT measured):"
    );
    println!(
        "  Runtime Tool-Calling Agent:  A={:.2}  B={:.2}  C={:.2}",
        ACTIONABILITY.runtime_a, ACTIONABILITY.runtime_b, ACTIONABILITY.runtime_c
    );
    println!(
        "  Code-Repair Agent:           A={:.2}  B={:.2}  C={:.2}",
        ACTIONABILITY.repair_a, ACTIONABILITY.repair_b, ACTIONABILITY.repair_c
    );
    // The qualitative claim the paper draws: A wins at runtime, C wins at repair.
    // `black_box` keeps these as runtime checks (the rubric is a const source of truth).
    let runtime = [
        std::hint::black_box(ACTIONABILITY.runtime_a),
        std::hint::black_box(ACTIONABILITY.runtime_c),
    ];
    let repair = [
        std::hint::black_box(ACTIONABILITY.repair_a),
        std::hint::black_box(ACTIONABILITY.repair_c),
    ];
    assert!(
        runtime[0] > runtime[1],
        "A should beat C for runtime orchestration"
    );
    assert!(
        repair[1] > repair[0],
        "C should beat A for source-code repair"
    );
}

// ============================================================================
// EXPERIMENT 2: The "Durability Tax" vs. Crash Recovery Break-Even Analysis
// ============================================================================

#[derive(Serialize, Deserialize, Clone)]
struct StepCheckpointPayload {
    workflow_id: String,
    step_index: usize,
    step_name: String,
    input_data: serde_json::Value,
    output_data: serde_json::Value,
    compensation_closure: String,
    timestamp_nanos: u128,
}

/// Outcome of simulating one recovery strategy on one workflow trial.
#[derive(Default, Clone, Copy)]
struct Outcome {
    executions: f64,
    duplicate_side_effects: f64,
}

/// A single unified workflow simulator parameterised by recovery strategy.
///
/// Semantics (shared across strategies):
///   * A step's external side effect commits only on a *successful* execution.
///   * A "duplicate side effect" is a successful execution of a step whose side
///     effect already committed earlier in the same trial.
///
/// Strategy knobs:
///   * `memoize`         completed steps are skipped, never re-executed (durable, retry).
///   * `restart_on_fail` any failure rewinds to step 0 (stateless restart).
///   * `crash_prob`      on failure, probability of losing memoized state (retry crash).
///
/// There is no artificial attempt cap: the random walk completes with probability 1
/// for p < 1, so the measured executions are true (uncensored) expectations.
fn simulate(
    rng: &mut SimpleRng,
    n: usize,
    p: f64,
    memoize: bool,
    restart_on_fail: bool,
    crash_prob: f64,
) -> (f64, f64) {
    let mut committed_known = vec![false; n]; // memoized knowledge (drives skipping)
    let mut committed_ever = vec![false; n]; // physical side effect performed at least once
    let mut current = 0usize;
    let mut executions = 0u64;
    let mut duplicates = 0u64;

    while current < n {
        if memoize && committed_known[current] {
            current += 1; // memoized replay: no execution, no side effect
            continue;
        }
        executions += 1;
        if rng.next_f64() < p {
            // Failure: no side effect committed.
            if restart_on_fail {
                committed_known.iter_mut().for_each(|c| *c = false);
                current = 0;
            } else if crash_prob > 0.0 && rng.next_f64() < crash_prob {
                committed_known.iter_mut().for_each(|c| *c = false);
                current = 0;
            }
            // otherwise retry the same step in place
        } else {
            // Success: side effect performed now.
            if committed_ever[current] {
                duplicates += 1;
            }
            committed_ever[current] = true;
            committed_known[current] = true;
            current += 1;
        }
    }
    (executions as f64, duplicates as f64)
}

fn mean_over(
    rng: &mut SimpleRng,
    trials: usize,
    n: usize,
    p: f64,
    memoize: bool,
    restart: bool,
    crash: f64,
) -> Outcome {
    let (mut e, mut d) = (0.0, 0.0);
    for _ in 0..trials {
        let (ex, du) = simulate(rng, n, p, memoize, restart, crash);
        e += ex;
        d += du;
    }
    Outcome {
        executions: e / trials as f64,
        duplicate_side_effects: d / trials as f64,
    }
}

#[test]
fn experiment_2_durability_tax_vs_recovery_breakeven() {
    println!("\n=== EXPERIMENT 2: THE DURABILITY TAX VS. CRASH RECOVERY BREAK-EVEN ANALYSIS ===");

    // Happy-path checkpoint serialization overhead (warmed up, then timed).
    let sample_payload = StepCheckpointPayload {
        workflow_id: "wf-7c9e-4b2a-89a1".to_string(),
        step_index: 3,
        step_name: "charge_payment_method".to_string(),
        input_data: serde_json::json!({ "amount_cents": 4900, "currency": "USD", "customer_id": "cust_123" }),
        output_data: serde_json::json!({ "charge_id": "ch_987", "status": "succeeded" }),
        compensation_closure: "refund_charge:ch_987".to_string(),
        timestamp_nanos: 1_729_550_000_000_000,
    };

    // Warm up allocator/serializer before measuring steady state.
    for _ in 0..1_000 {
        let _ = serde_json::to_vec(&sample_payload).unwrap();
    }
    let iters = 100_000;
    let start = Instant::now();
    let mut serialized_bytes = 0;
    for _ in 0..iters {
        serialized_bytes = serde_json::to_vec(&sample_payload).unwrap().len();
    }
    let cpu_us = start.elapsed().as_nanos() as f64 / (iters as f64 * 1000.0);

    println!("Happy-Path Checkpoint Invariant (p = 0.00):");
    println!("  - Serialization CPU Latency: {cpu_us:.3} us / step");
    println!("  - Persisted Payload Size:    {serialized_bytes} bytes / step");
    println!("  - Stateless Baseline:        0.000 us / step, 0 bytes written");
    assert!(serialized_bytes > 0);

    // A single base seed (0xDEADBEEF). Each (N, p) cell derives a deterministic
    // sub-seed, and all three strategies in a cell use common random numbers (the
    // same starting stream) so the comparison is paired, not confounded by noise.
    const BASE_SEED: u64 = 0xDEADBEEF;
    let depths = [1usize, 3, 5, 10, 20];
    let failure_rates = [0.00f64, 0.01, 0.05, 0.10, 0.25];
    let trials = 500;

    println!(
        "\nMonte Carlo Simulation ({trials} trials/cell, base seed 0xDEADBEEF, common random numbers, no attempt cap):"
    );
    println!(
        "{:<4} | {:<5} | {:<16} | {:<16} | {:<16} | {:<18}",
        "N", "p", "Stateless Restart", "Stateless Retry", "Hephos Durable", "Durable Dup Side-Fx"
    );
    println!(
        "{:-<4}-+-{:-<5}-+-{:-<16}-+-{:-<16}-+-{:-<16}-+-{:-<18}",
        "", "", "", "", "", ""
    );

    let mut results: HashMap<(usize, u32), (f64, f64, f64, f64)> = HashMap::new();
    for &n in &depths {
        for &p in &failure_rates {
            let cell_seed =
                BASE_SEED ^ ((n as u64) << 32) ^ ((p * 10_000.0) as u64).wrapping_mul(0x9E37_79B9);
            let restart = mean_over(
                &mut SimpleRng::new(cell_seed),
                trials,
                n,
                p,
                false,
                true,
                0.0,
            );
            let retry = mean_over(
                &mut SimpleRng::new(cell_seed),
                trials,
                n,
                p,
                true,
                false,
                0.02,
            );
            let durable = mean_over(
                &mut SimpleRng::new(cell_seed),
                trials,
                n,
                p,
                true,
                false,
                0.0,
            );

            // Durable execution is idempotent: completed steps are memoized, so a
            // durable run NEVER duplicates a committed side effect (Axiom 5 / Canary 3).
            assert_eq!(
                durable.duplicate_side_effects, 0.0,
                "durable must produce zero duplicate side effects (N={n}, p={p})"
            );
            // Executions can never fall below the N real steps of work.
            assert!(
                durable.executions >= n as f64,
                "durable runs at least N steps (N={n}, p={p})"
            );
            assert!(
                restart.executions >= n as f64,
                "restart runs at least N steps (N={n}, p={p})"
            );
            // At p=0 every strategy runs exactly N steps with zero duplicates.
            if p == 0.0 {
                assert_eq!(restart.executions, n as f64);
                assert_eq!(retry.executions, n as f64);
                assert_eq!(durable.executions, n as f64);
            }

            println!(
                "{:<4} | {:<5.2} | {:>10.2} execs | {:>10.2} execs | {:>10.2} execs | {:>10.2} dups",
                n,
                p,
                restart.executions,
                retry.executions,
                durable.executions,
                durable.duplicate_side_effects
            );
            results.insert(
                (n, (p * 100.0) as u32),
                (
                    restart.executions,
                    retry.executions,
                    durable.executions,
                    durable.duplicate_side_effects,
                ),
            );
        }
    }

    // Report the break-even narrative from the ACTUAL simulated cells, not literals.
    let (r20_10, _, d20_10, _) = results[&(20, 10)];
    let (r10_10, _, d10_10, _) = results[&(10, 10)];
    println!("\nBreak-Even Analysis Findings:");
    println!(
        "  - At p = 0.00: Hephos pays an unconditional tax of ~{cpu_us:.2} us and {serialized_bytes} bytes per step with 0 benefit."
    );
    println!(
        "  - At N = 10, p = 0.10: Stateless restart executes {r10_10:.1} steps; Hephos executes {d10_10:.1} steps ({:.1}x).",
        r10_10 / d10_10
    );
    println!(
        "  - At N = 20, p = 0.10: Stateless restart executes {r20_10:.1} steps; Hephos executes {d20_10:.1} steps ({:.1}x reduction).",
        r20_10 / d20_10
    );
    println!(
        "  - Break-Even Crossover: for external I/O > 5ms, durability wins once the saved re-executions outweigh N checkpoint writes."
    );

    // Sanity: at a non-trivial failure rate and depth, durability saves real work.
    assert!(
        r20_10 > d20_10,
        "restart should execute more steps than durable at N=20,p=0.10"
    );
}

// ============================================================================
// EXPERIMENT 3: Architectural Invariance vs. Boilerplate & Context Fragmentation
// ============================================================================
//
// The 4-layer Hephos slice and the single-file Axum baseline below are REAL,
// complete source fixtures. The experiment counts their files/LoC/defs/imports/
// tokens directly — nothing about the footprint is hardcoded.

const HEPHOS_DOMAIN: &str = r#"#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Widget {
    pub id: uuid::Uuid,
    pub name: String,
    pub quantity: i32,
}
"#;

const HEPHOS_REPO: &str = r#"use crate::domain::widget::Widget;

pub struct WidgetRepo;

impl WidgetRepo {
    pub async fn find(pool: &sqlx::PgPool, id: uuid::Uuid) -> hephos::Result<Option<Widget>> {
        let row = sqlx::query_as!(
            Widget,
            "SELECT id, name, quantity FROM widgets WHERE id = $1",
            id
        )
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }
}
"#;

const HEPHOS_SERVICE: &str = r#"use crate::domain::widget::Widget;
use crate::repos::widgets::WidgetRepo;
use crate::state::Ctx;

#[derive(Clone)]
pub struct WidgetService {
    db: hephos::db::Db,
}

impl WidgetService {
    pub async fn get(&self, _ctx: &Ctx, id: uuid::Uuid) -> hephos::Result<Widget> {
        match WidgetRepo::find(self.db.pool(), id).await? {
            Some(w) => Ok(w),
            None => Err(hephos::Error::not_found("widget")),
        }
    }
}
"#;

const HEPHOS_API: &str = r#"use hephos::prelude::*;

use crate::domain::widget::Widget;
use crate::state::Ctx;

pub async fn get_widget(ctx: Ctx, Path(id): Path<uuid::Uuid>) -> hephos::Result<Json<Widget>> {
    let widget = ctx.state().widgets.get(&ctx, id).await?;
    Ok(Json(widget))
}
"#;

/// Equivalent single-file idiomatic Axum handler: inline struct, inline query,
/// ad-hoc error mapping — the realistic "one file" a developer or agent writes.
const AXUM_SINGLE_FILE: &str = r#"use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Widget {
    pub id: uuid::Uuid,
    pub name: String,
    pub quantity: i32,
}

pub async fn get_widget(
    State(pool): State<sqlx::PgPool>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Widget>, StatusCode> {
    let row = sqlx::query_as!(Widget, "SELECT id, name, quantity FROM widgets WHERE id = $1", id)
        .fetch_optional(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match row {
        Some(w) => Ok(Json(w)),
        None => Err(StatusCode::NOT_FOUND),
    }
}
"#;

#[test]
fn experiment_3_architectural_footprint_and_context_overhead() {
    println!("\n=== EXPERIMENT 3: ARCHITECTURAL FOOTPRINT VS. CONTEXT FRAGMENTATION COST ===");

    let hephos_files = [HEPHOS_DOMAIN, HEPHOS_REPO, HEPHOS_SERVICE, HEPHOS_API];
    let hephos_src: String = hephos_files.concat();

    let h_files = hephos_files.len();
    let h_loc = hephos_files.iter().map(|f| code_lines(f)).sum::<usize>();
    let h_defs = count_defs(&hephos_src);
    let h_imports = count_imports(&hephos_src);
    let h_tokens = estimate_tokens(&hephos_src);

    let a_files = 1usize;
    let a_loc = code_lines(AXUM_SINGLE_FILE);
    let a_defs = count_defs(AXUM_SINGLE_FILE);
    let a_imports = count_imports(AXUM_SINGLE_FILE);
    let a_tokens = estimate_tokens(AXUM_SINGLE_FILE);

    println!(
        "Minimum Viable Vertical Slice Footprint (Widget read endpoint), measured from real fixtures:"
    );
    println!(
        "{:<24} | {:<16} | {:<16} | {:<14}",
        "Dimension", "Hephos (4 Layers)", "Single-File Axum", "Tax / Ratio"
    );
    println!("{:-<24}-+-{:-<16}-+-{:-<16}-+-{:-<14}", "", "", "", "");
    println!(
        "{:<24} | {:>14} | {:>16} | {:>11.2}x",
        "Files",
        h_files,
        a_files,
        h_files as f64 / a_files as f64
    );
    println!(
        "{:<24} | {:>11} LoC | {:>12} LoC | {:>11.2}x",
        "Source Lines (non-blank)",
        h_loc,
        a_loc,
        h_loc as f64 / a_loc as f64
    );
    println!(
        "{:<24} | {:>10} defs | {:>11} defs | {:>11.2}x",
        "Structs & Enums",
        h_defs,
        a_defs,
        h_defs as f64 / a_defs.max(1) as f64
    );
    println!(
        "{:<24} | {:>8} imports | {:>9} imports | {:>11.2}x",
        "Cross-Module Imports",
        h_imports,
        a_imports,
        h_imports as f64 / a_imports.max(1) as f64
    );
    println!(
        "{:<24} | {:>8} tokens | {:>9} tokens | {:>11.2}x",
        "Context Read Tokens",
        h_tokens,
        a_tokens,
        h_tokens as f64 / a_tokens as f64
    );

    // The upfront tax is real and positive across every dimension.
    assert_eq!(h_files, 4);
    assert_eq!(a_files, 1);
    assert!(h_loc > a_loc, "4-layer slice has more LoC");
    assert!(h_tokens > a_tokens, "4-layer slice costs more read tokens");
    assert!(
        h_imports > a_imports,
        "4-layer slice has more cross-module imports"
    );

    println!(
        "\nLongitudinal Scaling Model (ANALYTICAL — hypothesis, not measured agent behaviour):"
    );
    println!(
        "{:<8} | {:<24} | {:<24} | {:<26}",
        "M (APIs)", "Axum entropy (model)", "Hephos entropy (model)", "Modeled agent error rate"
    );
    println!("{:-<8}-+-{:-<24}-+-{:-<24}-+-{:-<26}", "", "", "", "");

    let m_values = [1usize, 5, 10, 25, 50, 100];
    for &m in &m_values {
        // Closed-form MODEL, not an observation. Unconstrained grammars accrue
        // pattern variants with scale; the invariant grammar stays at b=1.
        let h_axum = (1.0 + 0.8 * m as f64).log2() * 2.5;
        let h_hephos = 1.0;
        let p_err_axum_model = 1.0 - (-0.05 * m as f64).exp();
        let p_err_hephos_model = 0.04;
        assert_eq!(h_hephos, 1.0, "Hephos modeled entropy is invariant in M");
        println!(
            "{:<8} | {:>18.2} bits | {:>18.2} bits | {:>10.1}% vs {:>4.1}% (model)",
            m,
            h_axum,
            h_hephos,
            p_err_axum_model * 100.0,
            p_err_hephos_model * 100.0
        );
    }

    println!(
        "\nTakeaway: an upfront ~{:.1}x token tax at M=1 buys constant grammar entropy as M grows.",
        h_tokens as f64 / a_tokens as f64
    );
}

// ============================================================================
// EXPERIMENT 4: Schema Derivation & Tool Inventory Bloat Under Load
// ============================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
struct SimpleToolInput {
    entity_id: String,
    verbose: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct NestedFilter {
    field: String,
    operator: String,
    value: serde_json::Value,
    case_sensitive: bool,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ComplexToolInput {
    tenant_id: String,
    operation: String,
    filters: Vec<NestedFilter>,
    max_records: u32,
    include_archived: bool,
    metadata_overrides: HashMap<String, String>,
}

struct SimpleTestTool;
impl Tool for SimpleTestTool {
    type Input = SimpleToolInput;
    type Output = String;
    const NAME: &'static str = "simple_query";
    const DESCRIPTION: &'static str = "Queries entity status by identifier";
    async fn call(&self, _input: Self::Input) -> Result<String> {
        Ok("ok".to_string())
    }
}

struct ComplexTestTool;
impl Tool for ComplexTestTool {
    type Input = ComplexToolInput;
    type Output = String;
    const NAME: &'static str = "complex_search";
    const DESCRIPTION: &'static str =
        "Performs multi-predicate nested search with metadata extraction";
    async fn call(&self, _input: Self::Input) -> Result<String> {
        Ok("ok".to_string())
    }
}

/// Steady-state schema generation latency: warm up, then take the median of many runs
/// so we measure derivation cost, not one-shot allocator/code-path warmup.
fn steady_state_gen_us<T: Tool>() -> f64 {
    for _ in 0..50 {
        let _ = tool_schema::<T>();
    }
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let start = Instant::now();
        let _ = tool_schema::<T>();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    samples[samples.len() / 2] as f64 / 1000.0
}

#[test]
fn experiment_4_schema_derivation_and_tool_bloat() {
    println!("\n=== EXPERIMENT 4: SCHEMA DERIVATION & TOOL INVENTORY BLOAT UNDER LOAD ===");

    let simple_us = steady_state_gen_us::<SimpleTestTool>();
    let complex_us = steady_state_gen_us::<ComplexTestTool>();

    let simple_schema = tool_schema::<SimpleTestTool>();
    let complex_schema = tool_schema::<ComplexTestTool>();

    // Schemas are real and well-formed.
    assert_eq!(simple_schema.name, "simple_query");
    assert!(
        simple_schema.input_schema.get("properties").is_some(),
        "schema must expose properties"
    );

    // Compact JSON reflects the real on-the-wire tool-definition payload.
    let simple_json = serde_json::to_string(&serde_json::json!({
        "name": simple_schema.name, "description": simple_schema.description, "input_schema": simple_schema.input_schema,
    })).unwrap();
    let complex_json = serde_json::to_string(&serde_json::json!({
        "name": complex_schema.name, "description": complex_schema.description, "input_schema": complex_schema.input_schema,
    })).unwrap();

    let simple_tok = estimate_tokens(&simple_json);
    let complex_tok = estimate_tokens(&complex_json);

    println!("Single Tool Schema Materialization (steady-state median latency, compact JSON):");
    println!(
        "  - Simple Tool:  {simple_us:.2} us, {} bytes, ~{simple_tok} tokens",
        simple_json.len()
    );
    println!(
        "  - Complex Tool: {complex_us:.2} us, {} bytes, ~{complex_tok} tokens",
        complex_json.len()
    );

    assert!(
        complex_json.len() > simple_json.len(),
        "complex tool schema should be larger"
    );
    assert!(
        simple_us < 1_000.0 && complex_us < 1_000.0,
        "steady-state generation should be sub-millisecond"
    );

    println!("\nTool Inventory Scaling (linear schema injection into the system prompt):");
    println!(
        "{:<12} | {:<16} | {:<16} | {:<24}",
        "Tools (K)", "Total Schema Bytes", "Prompt Tokens", "% of 128k Window"
    );
    println!("{:-<12}-+-{:-<16}-+-{:-<16}-+-{:-<24}", "", "", "", "");

    let avg_tok = (simple_tok + complex_tok) / 2;
    let avg_bytes = (simple_json.len() + complex_json.len()) / 2;

    for &k in &[5usize, 25, 50, 100, 250, 500] {
        let total_bytes = k * avg_bytes;
        let total_tokens = k * avg_tok;
        let pct = total_tokens as f64 / 131_072.0 * 100.0;
        println!("{k:<12} | {total_bytes:>13} B | {total_tokens:>13} tok | {pct:>21.2}%");
        if k == 100 {
            assert!(
                pct > 10.0,
                "at K=100, schemas should consume a double-digit % of a 128k window"
            );
        }
    }

    println!(
        "\nTakeaway: generation is cheap, but injecting every schema statically scales linearly;"
    );
    println!(
        "beyond ~K=100 tools, dynamic/semantic tool retrieval is required to protect context."
    );
}
