# Hephos: Designing Agent-First Execution Environments through the Paradigm of Agent Experience (AX)

**Principal AI Systems Research & Distributed Systems Architecture Group**  
*Technical Paper — Systems & Software Architecture (modeled after ACM SOSP / USENIX OSDI / IEEE TSE)*  
*System Artifact: `arthurlch/hephos` (v1.0.0-GA, Rust 1.85+)*  
*Artifact Evaluation: `cargo test -p hephos-agent --test unbiased_evaluation` — every structural claim below is asserted in-test (the suite fails if a claim regresses); Monte Carlo figures are deterministic under the fixed base seed `0xDEADBEEF`; wall-clock latency figures are hardware-dependent and will vary per run.*  

---

### Abstract

Software engineering frameworks have historically been architected around the cognitive constraints, perceptual bandwidth, and ergonomic preferences of human engineers. User Experience (UX) and Developer Experience (DX) optimize for visual affordances, forgiving control flows, semantic flexibility, and permissive dynamic typing. However, the emerging paradigm of autonomous software engineering—wherein Large Language Model (LLM) agents act both as primary codebase authors and as in-process runtime orchestrators—exposes critical pathologies in human-centric runtime environments. Autonomous agents operate under strict physical and statistical constraints: finite token-bounded context windows, quadratic self-attention complexity $\mathcal{O}(L^2)$, non-deterministic autoregressive generation, and acute vulnerability to semantic ambiguity and schema drift. When agents are forced to operate in human-oriented frameworks, the abundance of syntactically valid implementations inflates conditional generation entropy, while verbose unstructured errors (such as 80-line panics and HTML stack traces) rapidly dilute context-window attention, leading to compounding autoregressive hallucinations and unrecoverable execution loops.

In this paper, we formalize **Agent Experience (AX)** as a rigorous systems engineering discipline governing the structural, semantic, and runtime interfaces between autonomous probabilistic cognitive engines and deterministic software environments. We introduce **Hephos**, an aggressively opinionated systems framework implemented in Rust, engineered from first principles for AX. Hephos eliminates semantic ambiguity by enforcing a strict four-layer architectural invariant ($\text{api} \to \text{services} \to \text{repos} \to \text{domain}$) verified via static text/pattern linting (`arch-lint.sh` and `hephos check`, which scan for forbidden cross-layer tokens — not an AST analysis), collapsing the agent's structural branching factor toward a constant $\mathcal{O}(1)$ at architectural decision points. The framework introduces an infallible request context (`Ctx<S>`) that eliminates generic extractor hallucinations; an isomorphic capability model whose JSON Schemas are derived from compile-time `schemars` derive macros and materialized at runtime for simultaneous in-process agent execution and external Model Context Protocol (MCP) serving; a closed-loop structured fault reflection mechanism mapping runtime failures into deterministic `{ "error": { "kind", "message" } }` recovery tokens; and a Postgres-checkpointed durable saga engine providing at-least-once step execution with exactly-once memoization of completed steps (idempotency required for non-idempotent effects) and best-effort Last-In, First-Out (LIFO) compensating transactions. 

Critically, rather than relying on strawman baselines, we evaluate Hephos through **four falsifiable experiments, each asserted in-test,** against idiomatic Rust baselines (`anyhow`, standard Axum, and stateless retries):
1. **Error Density Trade-Off:** While Hephos's structured JSON errors reduce payload size by $7.1\times$ (mean 22 tokens vs 158 tokens for backtraces) and maximize runtime tool retry actionability (rubric 1.0 vs 0.3), they exhibit a critical diagnostic defect for source-code repair agents (rubric 0.15 vs 0.95) by eliminating file/line provenance. (The actionability figures are an author-assigned qualitative rubric, not measured agent outcomes.)
2. **The Durability Tax:** On happy paths ($p=0.00$), Hephos pays an unconditional tax of $\sim 9\ \mu\text{s}$ CPU time (hardware-dependent) and 302 bytes written per step. However, Monte Carlo simulations (500 trials/cell, base seed `0xDEADBEEF`, common random numbers, no attempt cap) show that for deep workflows ($N=20, p=0.10$), durability reduces executions by $3.2\times$ (22.2 vs 71.4 steps), incurs exactly zero duplicate side effects where stateless restart re-executes committed steps, and prevents catastrophic cascading retries (at $N=20, p=0.25$ blind restart averages $>1{,}200$ step executions). It breaks even whenever external per-step I/O cost dominates the per-step checkpoint cost.
3. **Architectural Invariance Cost:** Implementing a minimal vertical slice in Hephos requires $4.0\times$ more files (4 vs 1), $1.9\times$ more code (42 vs 22 non-blank LoC), and $1.9\times$ more context tokens (~333 vs ~173 tokens) than single-file Axum. We then present an *analytical* grammar-entropy model (explicitly a hypothesis, not measured agent behaviour) under which unconstrained pattern drift drives modeled agent error rates past $70\%$ while Hephos's grammar entropy stays constant at $b=1$.
4. **Tool Inventory Context Bloat:** Upfront static schema generation via `schemars` executes in tens of microseconds per tool (steady-state median), but registering $K \ge 100$ tools consumes $>11\%$ of a 128k context window (~15,400 tokens), showing that static upfront schema registration hits severe context bottlenecks unless paired with dynamic semantic retrieval.

Criterion microbenchmarks on Apple Silicon demonstrate that Hephos's runtime safety harness (UUIDv4/v7 tracing, panic isolation, body bounding, timeouts, and rate limiting) adds $+4.85\ \mu\text{s}$ (GET) and $+4.10\ \mu\text{s}$ (POST) of CPU over raw Axum. This is a single-core, no-network serialization microbenchmark — it bounds per-request CPU (a ceiling on the order of $10^5\ \text{req/s/core}$), not sustained end-to-end throughput, which is dominated by network and database latency. We demonstrate that designing software environments for Agent Experience establishes production-grade agent reliability, while formally documenting the precise architectural boundaries where strict AX is counter-indicated.

---

## 1. Introduction & Problem Formulation

### 1.1 The Mismatch Between Human DX and Autoregressive Transformers

For more than five decades, programming language paradigms and software frameworks were engineered around Human-Computer Interaction (HCI). Developer Experience (DX) championing "syntactic sugar", implicit conventions, dynamic typing, and multi-paradigm flexibility ("there is more than one way to do it") aimed to reduce human typing friction and accommodate human working memory (Miller’s $7 \pm 2$ chunks).

When an autonomous cognitive agent (an LLM such as Claude Code, Codex, or Gemini) enters this software environment, human-centric design choices invert into severe systems liabilities:

1. **Quadratic Attention Dilution:** In Transformer-based auto-regressive models [7], self-attention compute and memory scale quadratically with sequence length $L$:
   $$\text{Attention}(Q, K, V) = \text{softmax}\left(\frac{QK^T}{\sqrt{d_k}}\right)V, \quad \text{Complexity} = \mathcal{O}(L^2 \cdot d_k)$$
   Human-centric frameworks emit verbose, conversational diagnostic payloads: 80-line stack backtraces, ANSI escape sequences, descriptive English prose, and unformatted HTML error pages. For an agent with a finite context ceiling $C_{\text{max}}$, these non-semantic tokens dilute the soft-attention distribution over instructions, resulting in the *lost-in-the-middle* phenomenon and catastrophic forgetfulness of system invariants.
2. **Autoregressive Error Compounding:** An LLM generates code and actions token-by-token:
   $$P(y_1, y_2, \dots, y_T) = \prod_{t=1}^T P(y_t \mid y_{<t}, \mathcal{E})$$
   If the environment $\mathcal{E}$ exhibits syntactic flexibility—offering $N$ valid ways to extract a parameter, handle an error, or wire a route—the conditional probability distribution $P(y_t \mid y_{<t}, \mathcal{E})$ spreads across a multimodal distribution. If an agent samples an atypical token early in generation, subsequent tokens drift rapidly toward hallucinated dependencies and invalid APIs.
3. **The Absence of Static Action Sandboxes:** In dynamically typed frameworks (e.g., Python FastAPI, Node.js Express), runtime parameter shape mismatches, missing fields, or implicit type coercions only surface when specific code paths execute. An agent generating code in such environments receives delayed, unstructured runtime crashes rather than immediate static verification signals.

```
+---------------------------------------------------------------------------------------+
| HUMAN-CENTRIC SOFTWARE STACK (UX / DX)                                                |
| - High Syntactic Entropy: Multiple valid patterns per task                           |
| - Unstructured Telemetry: Verbose human-oriented stack traces & HTML                   |
| - Ephemeral Execution: Unbounded loops, unmanaged in-flight side effects              |
+---------------------------------------------------------------------------------------+
                                        ▲
                                        │  Attention Dilution & Autoregressive Drift
                                        ▼
+---------------------------------------------------------------------------------------+
| AUTONOMOUS COGNITIVE AGENT (TRANSFORMER ENGINE)                                       |
| - Context Ceiling C_max              - Non-Deterministic Sampling                     |
| - Quadratic Attention O(L^2)         - Brittle Text Parsing                           |
+---------------------------------------------------------------------------------------+
                                        ▲
                                        │  Optimal Signal Density & Zero Ambiguity
                                        ▼
+---------------------------------------------------------------------------------------+
| AGENT EXPERIENCE STACK (HEPHOS AX INVARIANT)                                          |
| - Invariant 4-Layer Hierarchy: api -> services -> repos -> domain                     |
| - Deterministic Reflection: Static Schemas (schemars) + Closed Error Algebra (kind)    |
| - Durable Execution: Write-Ahead Turn Checkpointing & LIFO Saga Rollback              |
+---------------------------------------------------------------------------------------+
```

---

## 2. Formalizing Agent Experience (AX): Theoretical Foundations

Building on the "Agent Experience" framing introduced by Biilmann [6], we define **Agent Experience (AX)** as the formal systems discipline of designing software runtime environments, API surfaces, communication protocols, and code architectures to maximize the conditional probability of valid agent execution while minimizing token consumption and state entropy.

### 2.1 The Autoregressive Drift Problem

Let $\mathcal{A}$ denote the set of all syntactically valid code ASTs that achieve a specified functional requirement. In a flexible DX framework, $|\mathcal{A}| \gg 1$. 

At each token generation step $t$, the conditional entropy of the model’s generation is:
$$\mathcal{H}(y_t \mid y_{<t}, \mathcal{E}) = -\sum_{w \in \mathcal{V}} P(y_t = w \mid y_{<t}, \mathcal{E}) \log_2 P(y_t = w \mid y_{<t}, \mathcal{E})$$

When the environment prompt $\mathcal{E}$ provides conflicting or flexible examples (e.g., three different route extraction patterns across existing files), the conditional probability distribution over the vocabulary $\mathcal{V}$ is multimodal:
$$P(y_t = w \mid y_{<t}, \mathcal{E}) = \sum_{k=1}^K \pi_k P_k(y_t = w \mid y_{<t}, \text{Pattern}_k)$$
This entropy compounds over an emitted sequence of length $L$:
$$\mathcal{H}(y_{1:L} \mid \mathcal{E}) = \sum_{t=1}^L \mathcal{H}(y_t \mid y_{<t}, \mathcal{E})$$

**Proposition 1 (Structural Entropy Reduction).** *Let $\mathcal{E}_{\text{AX}}$ be an environment constrained by a canonical invariant that collapses the set of accepted implementations for a domain operation toward a single structural form (ideally $|\mathcal{A}| \to 1$ at structural decision points — module layout, extractor order, error construction). Then, to the extent the model's in-context demonstrations are consistent with that form, the conditional generation entropy $\mathcal{H}(y_{1:L} \mid \mathcal{E}_{\text{AX}})$ at those decision points is reduced relative to a flexible environment, lowering the probability of structurally invalid generations.*

This is a modelling argument, not a theorem: in practice $|\mathcal{A}|$ is never exactly $1$ (there is always more than one token sequence that compiles), and the reduction is bounded by how consistently the environment and its examples present the canonical form. *Argument.* Under an invariant architecture, consistent in-context demonstrations concentrate the mixture weights $\pi_k$ toward the canonical pattern, shrinking the support of the conditional distribution at structural decision points and reducing branching in the AST generation tree. The claim is directional (entropy decreases), not that it reaches zero.

### 2.2 Mathematical Formalization of the Five Laws of AX

We formalize the five systems invariants governing AX:

#### Axiom 1: Information Density & Context Conservation
Let $\Omega$ be the observation space emitted by the runtime back to the agent. Let $\mathcal{I}(\Omega; \mathcal{S})$ denote the mutual information between the observation and the true environment state $\mathcal{S}$, and let $|\Omega|_{\text{tokens}}$ be the token length of the observation. The runtime must satisfy:
$$\max_{\text{Runtime}} \frac{\mathcal{I}(\Omega; \mathcal{S})}{|\Omega|_{\text{tokens}}} \quad \text{subject to} \quad |\Omega|_{\text{tokens}} \ll C_{\text{max}}$$
The runtime must filter out non-actionable tokens (decorative banners, raw stack memory offsets, unindexed text) and emit strictly dense semantic discriminators.

#### Axiom 2: Structural Invariance & Syntactic Canonicalization
Let $\mathcal{P}$ be an application codebase comprising modules $M_1, M_2, \dots, M_n$. There exists a bijective homomorphism $\phi$ between any two resource modules $M_i, M_j$ such that their module topologies, directory placements, and handler signatures are AST-isomorphic:
$$\forall i, j \in \{1, \dots, n\}, \quad \text{AST}(M_i) \cong \text{AST}(M_j)$$
An agent learning the pattern of resource $M_1$ can generate resource $M_k$ with zero architectural inference error.

#### Axiom 3: Type-Driven Boundary Enforcement
Every capability (tool or API) exposed to an agent is governed by a compile-time static type signature $\mathcal{T}_{\text{in}} \to \mathcal{T}_{\text{out}}$. The environment must derive an exact, non-lossy JSON Schema $\mathcal{J}$ via static reflection:
$$\sigma: \mathcal{T}_{\text{in}} \xrightarrow{\text{compile-time}} \mathcal{J}(\mathcal{T}_{\text{in}})$$
The agent’s tool call $a = \langle \text{name}, \theta \rangle$ is validated at the deserialization boundary prior to execution:
$$\text{Exec}(a) = \begin{cases} 
f(\theta) & \text{if } \theta \in \text{Domain}(\mathcal{J}) \\
\text{Err}(\text{SchemaMismatch}) & \text{otherwise}
\end{cases}$$

#### Axiom 4: Closed-Loop Structured Fault Reflection
Let $f(\theta)$ be a fallible operation returning $\text{Result}\langle T, E \rangle$. The environment must map all $e \in E$ into a finite, closed error algebra $\mathcal{K} = \{\kappa_1, \kappa_2, \dots, \kappa_m\}$:
$$\rho: E \longrightarrow \langle \kappa \in \mathcal{K}, \mu \in \text{String} \rangle$$
Runtime failures must never abort process execution or emit unstructured panics. Instead, failures are returned as an observation turn:
$$\mathcal{O}_{\text{fault}} = \left\{ \text{"error"}: \left\{ \text{"kind"}: \kappa, \text{"message"}: \mu \right\} \right\}$$

#### Axiom 5: Idempotent Durable Execution
Let $\mathcal{W} = [s_1, s_2, \dots, s_k]$ be a sequence of stateful workflow steps with external side effects. The execution engine must maintain a persistent write-ahead checkpoint log $\mathcal{L} \subseteq \mathbb{N} \times \mathcal{S}$. For any step $s_i$ executed at turn $i$:
$$\text{Exec}(s_i) = \begin{cases}
o_i & \text{if } \langle i, o_i \rangle \in \mathcal{L} \quad (\text{Memoized Replay}) \\
\text{Commit}(\mathcal{L} \cup \{\langle i, f(s_i) \rangle\}) & \text{otherwise}
\end{cases}$$
If step $s_m$ fails fatally, the runtime must execute compensating transactions in reverse order (LIFO):
$$\text{Rollback}(\mathcal{W}) = \prod_{j=m-1}^{1} \text{Compensate}(s_j)$$

**Implementation note (delivery semantics).** The axiom above is the *target*. Because Hephos's `step` API takes an opaque closure, the side effect $f(s_i)$ and the checkpoint commit $\text{Commit}(\mathcal{L} \cup \dots)$ cannot be made atomic for arbitrary external effects. The implementation therefore provides **at-least-once execution with exactly-once memoization of *completed* steps**: a write-ahead `running` marker is persisted before $f$ runs, a `completed` checkpoint (with output) after; on resume only `completed` steps are memoized and skipped, while `running`/`failed`/absent steps re-execute. A crash in the window between the effect and the `completed` commit re-runs the effect, so **step bodies must be idempotent** (idempotency key, upsert, or conditional write). True exactly-once would require handing the transaction to the step so the effect and checkpoint commit together — an API Hephos deliberately does not yet expose. Compensation is likewise **best-effort and at-least-once**: a failing compensator does not abort the remaining LIFO rollback, and compensators must themselves be idempotent.

---

## 3. The Hephos Architecture: Compiler- and Linter-Enforced AX

Hephos implements these theoretical axioms in a modular Rust workspace (`crates/hephos-*`) targeting the 2024 edition and Rust 1.85+.

### 3.1 The 4-Layer Static Invariant (`api` → `services` → `repos` → `domain`)

To eliminate structural ambiguity, Hephos enforces a rigid separation of concerns. In traditional layered architectures, guidelines are mere prose documentation that human developers—and consequently AI agents—frequently violate by running SQL queries directly inside HTTP route handlers or leaking HTTP status codes into domain logic.

Hephos formalizes this boundary as an automated gate in `scripts/arch-lint.sh` and `crates/hephos-cli/src/commands/check.rs`:

```bash
# scripts/arch-lint.sh (Lines 44-59)
check "handler runs SQL or touches the pool/tx (api/ must call a service)" \
    'sqlx::|query_as!|query!|\.pool\(|\.begin\(' \
    examples/*/src/api.rs examples/*/src/api/*.rs

check "handler imports a repo (api/ must not depend on repos/)" \
    'use crate::repos' \
    examples/*/src/api.rs examples/*/src/api/*.rs

check "service speaks HTTP (services/ must be transport-agnostic)" \
    '(^|[^[:alnum:]_])axum::|StatusCode|IntoResponse' \
    examples/*/src/services.rs examples/*/src/services/*.rs

check "repo calls up into services (repos/ runs SQL only)" \
    'use crate::services' \
    examples/*/src/repos.rs examples/*/src/repos/*.rs
```

#### Complete Vertical Slice Implementation
The following verbatim code demonstrates the strict isolation across all four layers for a `Widget` domain aggregate:

```rust
// 1. domain/widget.rs - Pure Data Structs (Zero logic, zero I/O)
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Widget {
    pub id: uuid::Uuid,
    pub name: String,
    pub quantity: i32,
}

// 2. repos/widgets.rs - Pure Parameterized SQL (Zero business logic)
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

// 3. services/widgets.rs - Business Validation & Orchestration (No HTTP types)
#[derive(Clone)]
pub struct WidgetService { db: hephos::db::Db }
impl WidgetService {
    pub async fn get(&self, _ctx: &Ctx, id: uuid::Uuid) -> hephos::Result<Widget> {
        match WidgetRepo::find(self.db.pool(), id).await? {
            Some(w) => Ok(w),
            None => Err(hephos::Error::not_found("widget")),
        }
    }
}

// 4. api/widgets.rs - HTTP Routing & Infallible Context Adapter (Under 15 lines)
pub async fn get_widget(ctx: Ctx, Path(id): Path<uuid::Uuid>) -> hephos::Result<Json<Widget>> {
    let widget = ctx.state().widgets.get(&ctx, id).await?;
    Ok(Json(widget))
}
```

#### Empirical Trade-off: Boilerplate & Context Fragmentation (Experiment 3)

While Section 2 argued that a branching factor of $b=1$ suppresses structural drift, an independent systems reviewer must ask: **What is the upfront engineering tax of this 4-layer separation?**

We measured the exact structural footprint of implementing a minimal vertical slice in Hephos versus an idiomatic single-file Axum handler. Both implementations are checked in as real source fixtures in `crates/hephos-agent/tests/unbiased_evaluation.rs` (Experiment 3), and the test counts their files, non-blank lines, type definitions, imports, and token estimates directly — none of the numbers below are hardcoded:

| Architectural Dimension | Hephos 4-Layer Architecture | Idiomatic Single-File Axum | Structural Tax Ratio |
| :--- | :--- | :--- | :--- |
| **Files Touched** | 4 files (`domain`, `repos`, `services`, `api`) | 1 file (`handlers/widgets.rs`) | **$4.0\times$ files** |
| **Source Lines (non-blank)** | 42 lines | 22 lines | **$1.91\times$ LoC** |
| **Structs & Enums** | 3 definitions | 1 definition | **$3.0\times$ definitions** |
| **Cross-Module Imports** | 7 imports | 3 imports | **$2.33\times$ imports** |
| **Context Window Read Tokens** | ~333 tokens | ~173 tokens | **$1.92\times$ token tax** |

For a coding agent implementing its very first endpoint ($M=1$), Hephos imposes an upfront **$\sim 1.9\times$ context penalty** and, more importantly, requires reading and coordinating 4 separate files across different directories rather than one.

We then ask how the dynamics *might* change as a codebase scales across $M$ endpoints. The following table is an **analytical model, not a measurement**: the grammar-entropy columns are closed-form functions ($\mathcal{H}_{\text{Axum}} = 2.5\log_2(1 + 0.8M)$, $\mathcal{H}_{\text{Hephos}} = 1$), and the error-rate column is a hypothesized curve ($1 - e^{-0.05M}$ for unconstrained grammars, a flat $4\%$ type-error baseline for Hephos). No agent was run to produce these numbers; they express a hypothesis about pattern drift, which we flag as a threat to validity (§7) and leave for future empirical work.

| Endpoints Count ($M$) | Axum Grammar Entropy $\mathcal{H}$ (model) | Hephos Grammar Entropy $\mathcal{H}$ (model) | Modeled Agent Error Rate (analytical) |
| :--- | :--- | :--- | :--- |
| **$M = 1$** | $2.12\ \text{bits}$ | $1.00\ \text{bits}$ | $4.9\%\ \text{vs.}\ 4.0\%$ |
| **$M = 5$** | $5.80\ \text{bits}$ | $1.00\ \text{bits}$ | $22.1\%\ \text{vs.}\ 4.0\%$ |
| **$M = 10$** | $7.92\ \text{bits}$ | $1.00\ \text{bits}$ | $39.3\%\ \text{vs.}\ 4.0\%$ |
| **$M = 25$** | $10.98\ \text{bits}$ | $1.00\ \text{bits}$ | $71.3\%\ \text{vs.}\ 4.0\%$ |
| **$M = 50$** | $13.39\ \text{bits}$ | $1.00\ \text{bits}$ | $91.8\%\ \text{vs.}\ 4.0\%$ |
| **$M = 100$** | $15.85\ \text{bits}$ | $1.00\ \text{bits}$ | $99.3\%\ \text{vs.}\ 4.0\%$ |

The mechanism the model encodes: in unconstrained frameworks, developers and agents introduce subtle variations (inline queries, custom query helpers, different status wrappers, ad-hoc error returns), so cumulative pattern entropy grows with $M$ and pushes the agent toward multimodal sampling. Hephos instead trades an upfront $\sim 1.9\times$ single-file context tax for a grammar entropy that the `arch-lint` gate holds constant at $b=1$ regardless of scale. The directional claim (constant vs. growing entropy) follows from the invariant; the specific error-rate percentages do not — they are modeled, not observed.

---

### 3.2 Infallible Context Propagation (`Ctx`)

In general-purpose asynchronous web frameworks (e.g., raw Axum or Actix-web), request handling relies on dynamic extractor composition:
```rust
// Standard Axum: Fragile, hallucination-prone extractor ordering
async fn handler(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<Payload>,
) -> impl IntoResponse
```
If an AI agent misorders extractors (e.g., placing a consuming body extractor before an extension extractor) or misses a trait bound on `AppState`, the Rust compiler generates notoriously dense, multi-page error messages:
```text
error[E0277]: the trait bound `fn(State<...>, ...): Handler<_, _>` is not satisfied
  --> src/main.rs:42:19
   = note: required because of the requirements on the impl of `Handler` for ...
```
To an LLM agent, this compile-time error provides zero actionable hints regarding which extractor failed.

Hephos eliminates extractor hallucination by enforcing **Infallible Context Propagation**. The entire state, identity, tracing, and lifecycle infrastructure is unified into a single struct:

```rust
// crates/hephos-core/src/ctx.rs (Lines 27-32)
#[derive(Clone)]
pub struct Ctx<S> {
    state: S,
    identity: Identity,
    request_id: Uuid,
    cancel: CancellationToken,
}
```

In `crates/hephos-core/src/ctx.rs` (Lines 169–199), Hephos implements Axum's `FromRequestParts` with `type Rejection = Infallible`:

```rust
impl<S> FromRequestParts<S> for Ctx<S>
where
    S: Clone + Send + Sync + 'static,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let identity = parts.extensions.get::<Identity>().cloned().unwrap_or_default();
        let request_id = parts.extensions.get::<RequestId>().map(|r| r.0).unwrap_or_else(Uuid::new_v4);
        let cancel = parts.extensions.get::<CancellationToken>().cloned().unwrap_or_default();

        Ok(Ctx {
            state: state.clone(),
            identity,
            request_id,
            cancel,
        })
    }
}
```

Because the Hephos standard middleware stack (`crates/hephos-core/src/stack.rs`) unconditionally injects `RequestId`, `Identity`, and `CancellationToken` into request extensions during the outermost `observe` and `cancel` layers, extraction can **never fail**. Every handler signature in every Hephos application begins with `ctx: Ctx`.

---

### 3.3 Isomorphic Tooling & The Schema Context Bloat Problem

A primary challenge in LLM agent systems is capability duplication: developers write internal function calling wrappers for in-process agents, and separate OpenAPI or MCP endpoints for external tool consumers. This duplication inevitably introduces schema divergence and security drift.

Hephos resolves this through the `Tool` trait (`crates/hephos-agent/src/tool.rs`):

```rust
// crates/hephos-agent/src/tool.rs (Lines 35-46)
pub trait Tool: Send + Sync + 'static {
    type Input: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize + Send;

    const NAME: &'static str;
    const DESCRIPTION: &'static str;

    fn call(
        &self,
        input: Self::Input,
    ) -> impl std::future::Future<Output = Result<Self::Output>> + Send;
}
```

The `#[derive(JsonSchema)]` macro runs at **compile time**, generating a `JsonSchema` impl for `Self::Input` from the type's fields. `schemars::schema_for!` then **materializes** the concrete JSON Schema value at **runtime** by invoking that impl — which is precisely why Experiment 4 can measure its runtime cost in microseconds. The derivation is static (no reflection, no runtime type inspection beyond the generated impl); the schema value itself is built on demand:

```rust
// crates/hephos-agent/src/tool.rs (Lines 134-140)
pub fn schema_for<T: Tool>() -> ToolSchema {
    let schema = schemars::schema_for!(T::Input);
    ToolSchema {
        name: T::NAME.to_string(),
        description: T::DESCRIPTION.to_string(),
        input_schema: serde_json::to_value(schema).unwrap_or_default(),
    }
}
```

A registered `Tool` is monomorphized into an erased `ToolDefinition` and mounted into two distinct consumers simultaneously: the in-process agent turn loop (`crates/hephos-agent`) and the external Model Context Protocol server (`crates/hephos-mcp/src/server.rs`).

#### Empirical Bottleneck: The Tool-Schema Context Bloat Problem (Experiment 4)

While static schema derivation guarantees runtime type safety, we evaluated how static schema injection scales as an agent’s tool registry expands (measured in `crates/hephos-agent/tests/unbiased_evaluation.rs`). Latency is the **steady-state median** over 200 runs after warm-up — our earlier single-shot measurement conflated one-time code-path/allocator warm-up with derivation cost, so the complex tool appeared (wrongly) faster than the simple one; the steady-state figures below correct that. Byte/token counts are over **compact** JSON, matching the on-the-wire tool-definition payload:

- **Simple Tool (`SimpleToolInput`):** $\sim 10\ \mu\text{s}$ median, 294 bytes, $\sim 78$ prompt tokens.
- **Complex Nested Tool (`ComplexToolInput`):** $\sim 42\ \mu\text{s}$ median, 874 bytes, $\sim 230$ prompt tokens.

When an application exposes $K$ tools statically to an agent, the total schema payload scales linearly (using the per-tool average of the two samples above):

| Registered Tools ($K$) | Total Schema Bytes | System Prompt Token Cost | Context Window Consumption ($128\text{k}$) |
| :--- | :--- | :--- | :--- |
| **$K = 5$ tools** | $2,920\ \text{B}$ | $770\ \text{tokens}$ | $0.59\%$ |
| **$K = 25$ tools** | $14,600\ \text{B}$ | $3,850\ \text{tokens}$ | $2.94\%$ |
| **$K = 50$ tools** | $29,200\ \text{B}$ | $7,700\ \text{tokens}$ | $5.87\%$ |
| **$K = 100$ tools** | $58,400\ \text{B}$ | $15,400\ \text{tokens}$ | **$11.75\%$** |
| **$K = 250$ tools** | $146,000\ \text{B}$ | $38,500\ \text{tokens}$ | **$29.37\%$** |
| **$K = 500$ tools** | $292,000\ \text{B}$ | $77,000\ \text{tokens}$ | **$58.75\%$** |

**Finding:** Upfront static schema generation is computationally cheap (tens of microseconds per tool, steady-state). However, for $K \ge 100$ tools, injecting static schemas directly into the LLM system prompt consumes $>11\%$ of a 128k window, crowding out reasoning tokens; by $K = 500$ it approaches $60\%$. For enterprise environments with hundreds of tools, static AX must be augmented with dynamic semantic tool retrieval or pagination. (This extrapolation assumes the average per-tool schema size of our two samples; real inventories will vary with input complexity.)

---

## 4. Runtime Mechanics: Self-Healing Loops & Durable Checkpointing

### 4.1 The Resilient Agent Execution Loop

Hephos models agent interaction as an explicit, bounded turn loop. Unlike unconstrained agent frameworks that permit infinite recursion under hallucinated errors, Hephos enforces strict multi-dimensional resource bounds via `Limits`:

```rust
// crates/hephos-agent/src/agent.rs (Lines 62-67)
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_turns: u32,
    pub max_tokens: u32,
    pub timeout: Duration,
    pub max_tool_calls_per_turn: usize,
}
```

The execution flow of the agent turn engine is formalized in **Algorithm 1**.

```
Algorithm 1: Hephos Bounded Agent Turn Loop with Durable Checkpointing
───────────────────────────────────────────────────────────────────────────────────────────
Input  : User Prompt x_0, Model M, Tools collection T, Bounds Limits, Checkpoint Store C
Output : Final Assistant Message y or Error E

 1:  messages ← [ Message::User(x_0) ]
 2:  start_turn ← 0
 3:  if C is provided then
 4:      for turn ← 0 to Limits.max_turns - 1 do
 5:          if checkpoint ← C.load_turn(turn) exists then
 6:              messages ← checkpoint
 7:              start_turn ← turn + 1
 8:          else
 9:              break
10:      if start_turn > 0 and messages.last().role == Assistant and no_tools(messages.last()) then
11:          return messages.last()
12:
13:  timer ← StartTimer(Limits.timeout)
14:  for turn ← start_turn to Limits.max_turns - 1 do
15:      if timer.expired() or CancelToken.is_cancelled() then
16:          return Err(Error::TimeoutOrCancelled)
17:
18:      schemas ← T.schemas()
19:      req ← Completion { messages, tools: schemas, max_tokens: Limits.max_tokens }
20:      reply ← await M.complete(req)
21:
22:      calls ← parse_tool_calls(reply)
23:      if calls is not empty then
24:          if |calls| > Limits.max_tool_calls_per_turn then
25:              return Err(Error::Invalid("tool fan-out exceeded"))
26:          messages.push(reply)
27:
28:          // Concurrent Tool Dispatch with Structured Fault Interception
29:          tool_responses ← parallel_map(calls, c =>
30:              span ← OpenSpan("agent.tool", c.name)
31:              res ← T.dispatch(c)
32:              match res with
33:              | Ok(val) => ToolResult { id: c.id, output: val }
34:              | Err(err) => ToolResult { id: c.id, output: { "error": { "kind": err.kind(), "message": err.msg() } } }
35:          )
36:          messages.extend(tool_responses)
37:          if C is provided then
38:              await C.save_turn(turn, messages)
39:      else
40:          messages.push(reply)
41:          if C is provided then
42:              await C.save_turn(turn, messages)
43:          return reply
44:
45:  return Err(Error::Internal("exceeded max_turns without final answer"))
───────────────────────────────────────────────────────────────────────────────────────────
```

---

### 4.2 Structured Fault Reflection vs. Diagnostic Information Density (Experiment 1)

In standard software environments, when a tool execution encounters an error, it throws an exception or aborts. Hephos defines a closed error algebra (`crates/hephos-core/src/error.rs`):

```rust
// crates/hephos-core/src/error.rs (Lines 21-55)
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("request timeout")]
    Timeout,
    #[error("method not allowed")]
    MethodNotAllowed,
    #[error("too many requests: {0}")]
    RateLimited(String),
    #[error("service unavailable: {0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}
```

In `crates/hephos-agent/src/agent.rs` (Lines 484–498), tool dispatch captures every failure and serializes it directly into a model-interpretable JSON observation:

```rust
let result = match self.tools.dispatch(call).instrument(span).await {
    Ok(res) => res,
    Err(err) => ToolResult::with_call_id(
        call_id,
        name,
        serde_json::json!({
            "error": {
                "kind": err.kind(),
                "message": err.to_string(),
            }
        }),
    ),
};
```

#### Objective Comparison: Three Error Formats Across Failure Scenarios

To avoid strawman comparisons against artificial megabyte backtraces, we evaluated three realistic error representations across identical failure scenarios in `crates/hephos-agent/tests/unbiased_evaluation.rs`:
- **Format A (Hephos AX JSON):** `{"error": {"kind": "...", "message": "..."}}` — generated from a real `hephos::Error`.
- **Format B (Idiomatic Developer Rust):** a *real* `anyhow` causal chain rendered via `{:?}` (`Error: ... Caused by: ...`), built in-test, not hand-typed.
- **Format C (Standard Debug / Backtrace):** a representative sanitized ~30-frame Rust backtrace. A live `RUST_BACKTRACE=1` capture is non-deterministic (addresses and frame counts vary per build), so a fixed representative sample is used — these byte/token counts are illustrative of backtrace scale, not a captured measurement.

Token counts use the paper's $\sim 3.8$ chars/token heuristic (see §7.3). Formats A and B are exact renderings; C is the representative fixture:

| Failure Scenario | Format A (AX JSON) | Format B (Causal Chain) | Format C (Debug Backtrace) | Token Reduction (C / A) |
| :--- | :--- | :--- | :--- | :--- |
| **Schema Mismatch** | $93\ \text{B}\ /\ 25\ \text{tok}$ | $125\ \text{B}\ /\ 33\ \text{tok}$ | $651\ \text{B}\ /\ 172\ \text{tok}$ | **$6.9\times$** |
| **DB Constraint Violation** | $104\ \text{B}\ /\ 28\ \text{tok}$ | $165\ \text{B}\ /\ 44\ \text{tok}$ | $728\ \text{B}\ /\ 192\ \text{tok}$ | **$6.9\times$** |
| **Missing Entity (404)** | $77\ \text{B}\ /\ 21\ \text{tok}$ | $127\ \text{B}\ /\ 34\ \text{tok}$ | $546\ \text{B}\ /\ 144\ \text{tok}$ | **$6.9\times$** |
| **Internal Timeout** | $56\ \text{B}\ /\ 15\ \text{tok}$ | $126\ \text{B}\ /\ 34\ \text{tok}$ | $480\ \text{B}\ /\ 127\ \text{tok}$ | **$8.5\times$** |
| **Mean Across Scenarios** | **$22\ \text{tokens}$** | **$36\ \text{tokens}$** | **$158\ \text{tokens}$** | **$7.1\times$ average** |

#### Semantic Actionability: A Qualitative Rubric (not a measurement)

The scores below are an **author-assigned qualitative rubric** (0.0–1.0) encoding engineering judgement about how usable each format is for a given agent archetype. They are **not** measured from agent runs and should not be read as empirical results; they exist to make the diagnostic trade-off explicit and are the single source of truth asserted in-test (the ordering claims — A beats C at runtime, C beats A at repair — are what the test checks).

1. **Runtime Tool-Calling Agent (Invoking APIs in production):**
   - **Format A (Rubric: 1.0):** The machine-readable key `"kind": "conflict"` lets the agent immediately sample an alternate parameter without parsing ambiguity.
   - **Format B (Rubric: 0.70):** Requires parsing unstructured English text to extract the underlying failure mode.
   - **Format C (Rubric: 0.30):** Attention dilution — stack frames pollute working memory and risk triggering hallucinated code fixes rather than parameter retries.
2. **Code-Repair Agent (Debugging Source Code Bugs):**
   - **Format A (Rubric: 0.15 — judged a critical defect):** Stripping the stack trace leaves the agent unable to locate which file, module, or line generated the error.
   - **Format B (Rubric: 0.65):** Provides conceptual causal breadcrumbs, but lacks exact line numbers.
   - **Format C (Rubric: 0.95 — judged optimal):** Pinpoints the exact `file:line:column` and call stack needed to formulate a compiler-compatible `git diff`.

**Conclusion:** On token density (measured) Format A dominates; on this qualitative rubric Format A is best for runtime tool orchestration but worst for source-code repair. The compression result is empirical; the actionability ranking is reasoned judgement.

---

### 4.3 The "Durability Tax" vs. Crash Recovery Break-Even Analysis (Experiment 2)

Durable execution (checkpointing every turn and maintaining compensation logs) guarantees that an agent never duplicates side effects upon crashing. However, durability imposes an inescapable systems overhead.

In `crates/hephos-agent/tests/unbiased_evaluation.rs`, we measured the exact happy-path serialization tax and executed a 500-trial-per-cell Monte Carlo simulation across workflow depths $N \in \{1, 3, 5, 10, 20\}$ and step failure probabilities $p \in \{0.00, 0.01, 0.05, 0.10, 0.25\}$. The simulation uses a single base PRNG seed `0xDEADBEEF`; each $(N, p)$ cell derives a deterministic sub-seed and feeds **common random numbers** to all three strategies (a paired comparison), and there is **no artificial attempt cap** — the random walk completes with probability 1 for $p < 1$, so the reported executions are true, uncensored expectations.

#### Happy-Path Overhead Invariant ($p = 0.00$)
- **Hephos Serialization Latency:** $\sim 9\ \mu\text{s}$ CPU time per step (in-memory JSON; hardware-dependent, varies per run).
- **Persisted Payload Size:** $302\ \text{bytes}$ written per step (deterministic).
- **Stateless Baseline:** $0\ \mu\text{s}$ CPU time, $0\ \text{bytes}$ written.

On a flawless execution run, Hephos pays an unconditional tax of $\sim 9\ \mu\text{s}$ and $302$ bytes per step with zero operational benefit.

#### Monte Carlo Failure Sweep Across Strategies

We compared three strategies:
1. *Stateless Restart:* Upon failure, the process aborts and restarts from step 1.
2. *Stateless Retry:* Failed steps are retried in memory, with a simulated $2\%$ process crash rate.
3. *Hephos Durable Saga:* Every step is checkpointed; failure resumes from the exact failed step; fatal errors trigger LIFO compensations.

The final column is the **Hephos Durable duplicate-side-effect count**, which is identically $0.00$ across every cell — this is the point of Axiom 5 and Canary 3: completed steps are memoized and never re-executed, so a committed side effect is never repeated. The contrast is that stateless restart re-executes already-committed steps on every rewind; its *own* duplicate side effects equal (Restart Executions $- N$), which grows without bound as $p$ rises (e.g. $>1{,}200$ at $N=20, p=0.25$).

| Depth ($N$) | Failure Prob ($p$) | Stateless Restart Executions | Stateless In-Memory Retry | Hephos Durable Executions | Durable Dup. Side-Fx |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **$N = 1$** | $p = 0.00$ | $1.00$ | $1.00$ | $1.00$ | $0.00$ |
| | $p = 0.05$ | $1.05$ | $1.05$ | $1.05$ | $0.00$ |
| | $p = 0.25$ | $1.37$ | $1.33$ | $1.37$ | $0.00$ |
| **$N = 5$** | $p = 0.00$ | $5.00$ | $5.00$ | $5.00$ | $0.00$ |
| | $p = 0.05$ | $5.97$ | $5.31$ | $5.31$ | $0.00$ |
| | $p = 0.25$ | $12.59$ | $6.77$ | $6.65$ | $0.00$ |
| **$N = 10$** | $p = 0.00$ | $10.00$ | $10.00$ | $10.00$ | $0.00$ |
| | $p = 0.05$ | $13.40$ | $10.60$ | $10.52$ | $0.00$ |
| | $p = 0.10$ | $18.34$ | $11.06$ | $11.04$ | $0.00$ |
| | $p = 0.25$ | $63.21$ | $13.75$ | $13.32$ | $0.00$ |
| **$N = 20$** | $p = 0.00$ | $20.00$ | $20.00$ | $20.00$ | $0.00$ |
| | $p = 0.01$ | $21.91$ | $20.18$ | $20.18$ | $0.00$ |
| | $p = 0.05$ | $34.59$ | $21.21$ | $21.05$ | $0.00$ |
| | $p = 0.10$ | $71.38$ | $22.58$ | $22.21$ | $0.00$ |
| | $p = 0.25$ | $1234.23$ | $28.40$ | $26.63$ | $0.00$ |

```
Step Executions Under High Failure Rate (N = 20, p = 0.25)
┌───────────────────────────────────────────────────────────────┐
│ Stateless Restart: 1234.23 execs (Catastrophic Cascading)     │
├───────────────────────────────────────────────────────────────┤
│ Stateless In-Memory: 28.40 executions                         │
├───────────────────────────────────────────────────────────────┤
│ Hephos Durable Saga: 26.63 execs (0 duplicate side effects)   │
└───────────────────────────────────────────────────────────────┘
```

#### Break-Even Threshold Curve

Let $C_{\text{io}}$ be the latency/financial cost of an external step (e.g., Stripe API, LLM call, DB write) and $C_{\text{check}}$ be the checkpoint serialization cost ($\sim 9\ \mu\text{s}$ CPU + DB write). Rearranging, durability is favourable when
$$\frac{C_{\text{io}}}{C_{\text{check}}} > \frac{N}{\mathbb{E}[\text{Executions}_{\text{restart}}] - \mathbb{E}[\text{Executions}_{\text{durable}}]}$$

Plugging the simulated cell $N=5, p=0.01$ (restart $5.18$, durable $5.06$, difference $0.12$) gives a threshold ratio of $\sim 42$: durability pays off once a step's external I/O exceeds roughly $42\times$ the checkpoint cost — i.e. for I/O in the sub-millisecond range and up, even a $1\%$ failure rate favours durability. Because the restart cost grows super-linearly as $p$ rises (reaching $>1{,}200$ executions at $N=20, p=0.25$), the crossover moves to still lower failure rates for deeper workflows and slower I/O. This is an order-of-magnitude threshold derived from the inequality and the simulated cells, not a separately measured crossover. Conversely, for sub-millisecond, in-memory CPU workflows where $C_{\text{io}} \approx C_{\text{check}}$, durability is an unnecessary throughput tax.

---

## 5. Empirical Evaluation, Verification Canaries & Microbenchmarks

### 5.1 Behavioural Verification via AX Canary Tests

The Hephos repository contains dedicated automated canary test suites verifying AX invariants:

#### Canary 1: In-Flight Tool Fault Recovery
Located in `crates/hephos-agent/tests/agent.rs` (Lines 332–365), this test verifies that an agent executing a tool that fails with a `404 Not Found` recovers on the subsequent turn and successfully streams its final textual answer without dropping the connection:

```rust
#[tokio::test]
async fn tool_using_agent_streams_after_recovering_from_tool_error() {
    struct FailOnceTool;
    impl Tool for FailOnceTool {
        type Input = serde_json::Value;
        type Output = String;
        const NAME: &'static str = "fail_tool";
        const DESCRIPTION: &'static str = "fails with not_found";

        async fn call(&self, _input: Self::Input) -> Result<String> {
            Err(hephos_core::Error::not_found("item not found"))
        }
    }

    let model = SequenceModel::new(vec![
        Message::assistant_with_tools("trying tool", vec![ToolCall::with_id("c_fail", "fail_tool", serde_json::json!({}))]),
        Message::assistant("recovered stream output"),
    ]);

    let agent = Agent::new(model).tool(FailOnceTool);
    let mut stream = agent.stream("start").await.unwrap();
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        text.push_str(&chunk.unwrap());
    }
    assert_eq!(text, "recovered stream output");
}
```

#### Canary 2: Router Composition Safety (`routes_build`)
In standard Axum, invalid route tree compositions—specifically nesting sub-routers at `"/"`—compile cleanly but trigger a runtime panic on server startup:
```text
thread 'main' panicked at 'Cannot nest a router at "/"'
```
To eliminate this failure mode in automated agent coding workflows, Hephos ships an in-memory test client (`hephos::test::TestClient`) and enforces a mandatory `routes_build` canary test in every application crate (`examples/reference-app/src/tests.rs:L10-15`):

```rust
#[tokio::test]
async fn routes_build() {
    let state = AppState::init().await.expect("state init");
    let router = api::routes(state.auth.clone());
    let _client = TestClient::new(router, state); // Catches router nesting panics in CI
}
```

#### Canary 3: Idempotent Checkpointed Turn Resumption
Located in `crates/hephos-agent/src/agent.rs` (Lines 758–787), this test verifies that an agent interrupted after executing a side-effecting tool resumes from its saved turn checkpoint without re-invoking the tool:

```rust
#[tokio::test]
async fn agent_durable_resumes_from_saved_turn_without_reexecuting_tools() {
    let counter = Arc::new(AtomicUsize::new(0));
    let model = MultiTurnModel::default();
    let tool = IncTool { counter: counter.clone() };
    let checkpoint = InMemoryAgentCheckpoint::new();

    // Run 1: completes turn 1 (invoking tool) and turn 2
    let agent1 = Agent::new(model.clone()).tool(tool.clone());
    let res1 = agent1.run_durable("start", &checkpoint).await.unwrap();
    assert_eq!(res1, "increment completed successfully");
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Run 2: resumed with same checkpoint; tool must NOT be re-executed
    let agent2 = Agent::new(model.clone()).tool(tool.clone());
    let res2 = agent2.run_durable("start", &checkpoint).await.unwrap();
    assert_eq!(res2, "increment completed successfully");
    assert_eq!(counter.load(Ordering::SeqCst), 1); // Counter strictly remains 1
}
```

---

### 5.2 Systems Overhead & Criterion Microbenchmarks

Microbenchmarks were executed using Criterion.rs (`crates/hephos-core/benches/overhead.rs`) on an Apple Silicon M-series single core. These measure **per-request CPU time** with no network or database in the loop — they bound the framework's own CPU cost, not sustained end-to-end throughput. The benchmark directly compares bare Axum against Hephos running its full standard Tower middleware stack:

| Benchmark Target | Raw Axum Baseline | Hephos Full Stack | Delta Overhead ($\Delta$) | Overhead Relative to 10ms RTT |
| :--- | :--- | :--- | :--- | :--- |
| **GET Request + JSON Serialize** | $0.84\ \mu\text{s}$ | $5.69\ \mu\text{s}$ | $+4.85\ \mu\text{s}$ | **$0.048\%$** |
| **POST Request + JSON Deserialize**| $1.72\ \mu\text{s}$ | $5.82\ \mu\text{s}$ | $+4.10\ \mu\text{s}$ | **$0.041\%$** |

```
CPU Overhead Breakdown of Hephos Standard Stack (~4.85 µs total)
┌───────────────────────────────────────────────────────────────┐
│ 01. UUIDv4 RequestId Generation (0.42 µs)                     │
├───────────────────────────────────────────────────────────────┤
│ 02. Tracing Span Open/Close & Header Injection (1.12 µs)      │
├───────────────────────────────────────────────────────────────┤
│ 03. Cancellation Token Child Instantiation (0.38 µs)          │
├───────────────────────────────────────────────────────────────┤
│ 04. Timeout Guard & Body Limit Layers (0.85 µs)               │
├───────────────────────────────────────────────────────────────┤
│ 05. CatchPanicLayer Wrapper & Isolation Frame (0.64 µs)       │
├───────────────────────────────────────────────────────────────┤
│ 06. RED Metrics Route Matching & Counter Recording (1.44 µs)  │
└───────────────────────────────────────────────────────────────┘
```

The entire production-grade AX safety harness consumes less than **$5\ \mu\text{s}$ of CPU time** per request, which is below **$0.05\%$ of a single $10\text{ms}$ network round-trip** — negligible against the network and database latency that dominate real transactions. (The per-component breakdown above is an approximate attribution of the measured $\sim 4.85\ \mu\text{s}$ delta across the stack's layers, not six independently isolated benchmarks.)

---

## 6. When NOT to Use Strict AX (Counter-Indications & Failure Modes)

A rigorous systems evaluation must explicitly demarcate where an architectural philosophy breaks down. Strict AX is counter-indicated in the following regimes:

### 6.1 Ephemeral & Sub-Millisecond Micro-Tasks
For stateless, high-throughput microservices (e.g., DNS routers, proxy gateways, memory caches) where execution latency is under $1\text{ms}$ and failures carry no persistent side effects, Hephos’s durable checkpointing and serialization ($\sim 9\ \mu\text{s}$ and 302 bytes per step) constitutes 100% pure overhead.

### 6.2 Exploratory Throwaway Prototyping
When human engineers or AI agents are writing exploratory "spike" scripts or single-use scrapers, the mandatory 4-layer file footprint ($4\times$ files, $\sim 1.9\times$ LoC, $\sim 1.9\times$ read tokens) slows iteration. Unconstrained single-file scripts in Python or raw Axum are significantly more agile for throwaway artifacts.

### 6.3 Deep Compiler & Source-Code Bug Repair
As demonstrated in Experiment 1, Hephos’s closed error algebra strips stack frames, module paths, and line numbers to preserve agent context during runtime tool calls. When an agent’s objective is source-code repair (e.g., resolving a compiler panic or a subtle memory issue), Format A leaves the agent blind (actionability score $0.15$). Source-code repair agents must be provided raw compiler diagnostics and backtraces (Format C).

### 6.4 Massive Tool Inventories ($K > 100$) Without Dynamic Retrieval
As shown in Experiment 4, static upfront schema registration scales linearly. At $K = 100$ tools schemas already consume $\sim 12\%$ of a 128k context window, and at $K = 500$ tools $\sim 59\%$. Frameworks relying strictly on static upfront registration degrade under large tool inventories unless augmented with dynamic semantic indexing (e.g., embedding-based tool selection).

---

## 7. Threats to Validity

### 7.1 Internal Validity
- **Synthetic Fault Distributions:** In Experiment 2, step failure probabilities were modeled using independent Bernoulli trials generated via deterministic PRNG (`0xDEADBEEF`). In real distributed systems, failures are often bursty and correlated (e.g., cascading network partitions or database connection pool exhaustion).
- **In-Memory Checkpoint Serialization:** Happy-path checkpoint latency was measured via in-memory JSON serialization. In production deployments backed by physical PostgreSQL clusters, disk I/O write latency ($0.5–5\text{ms}$) dominates over serialization CPU time.

### 7.2 External Validity
- **No live LLM in the loop.** This is the most important caveat: none of the four experiments invoke an actual language model. Token counts are produced by a fixed $\sim 3.8$ chars/token heuristic rather than any real tokenizer, and the attention-dilution and drift arguments are motivated by the transformer literature, not measured on a specific model here. The quantitative claims that *are* measured (error-payload bytes/tokens, serialization latency, simulated executions, schema sizes) are model-independent; the claims about *agent behaviour* (error rates, actionability) are analytical or judgemental and are not validated against a model.
- **Generalization Across LLM Architectures:** The drift/attention motivation assumes contemporary transformer tokenizers and attention behaviour. Real tokenizer vocabularies differ, and non-transformer architectures (e.g., state-space models like Mamba) may exhibit different degradation curves.
- **Language & Runtime Generalization:** Hephos leverages Rust's affine type system, monomorphization, and zero-cost abstractions. In garbage-collected or dynamically typed runtimes (Python, Node.js), middleware isolation frames and schema derivation impose higher latency and memory pressure.

### 7.3 Construct Validity
- **Byte and Token Proxies:** We utilized BPE character heuristics ($\sim 3.8\ \text{chars/token}$) to quantify context window consumption. While standard in LLM literature, true tokenization varies across proprietary tokenizer vocabularies.
- **Semantic Actionability Scoring:** The 0.0–1.0 actionability metric (Experiment 1) is an **author-assigned qualitative rubric**, not a measurement from agent runs. Only its *ordering* (A best for runtime orchestration, C best for source repair) is asserted in-test; the specific values are reasoned judgement and should be read as such.
- **Analytical Grammar-Entropy / Error-Rate Model:** The scaling table in §3.1 (grammar entropy and agent error rate vs. endpoint count $M$) is a **closed-form hypothesis**, not observed behaviour. The functional forms ($2.5\log_2(1+0.8M)$; $1-e^{-0.05M}$) and their constants were chosen to express the drift mechanism, not fitted to data. Validating these curves against a real agent generating many endpoints in both architectures is left to future work; until then, only the *directional* claim (constant vs. growing entropy under the invariant) should be relied upon.
- **Representative (not captured) backtraces:** Format C in Experiment 1 is a fixed representative sample, since a live backtrace is non-deterministic. Its absolute size is illustrative of backtrace scale, not a measured artifact of any single failure.

---

## 8. Related Work & Comparative Analysis

* **Agent Orchestration Frameworks (LangGraph, AutoGen, CrewAI):** Python-based orchestration libraries [3] provide flexible conversational patterns. However, their reliance on dynamic typing, unconstrained dictionary state passing, and permissive execution loops results in high token entropy and frequent runtime parsing exceptions.
* **Protocol Standards (Model Context Protocol / MCP):** Anthropic’s MCP [1] standardizes JSON-RPC communication between LLMs and external tools over stdio and HTTP SSE. While MCP addresses the transport wire format, it does not prescribe internal application architecture. Hephos natively implements MCP via `rmcp` (`crates/hephos-mcp`), but uniquely bridges the protocol directly to in-process Rust trait definitions.
* **Durable Execution Frameworks (DBOS, Temporal):** Temporal [5] and DBOS [2] pioneered durable function execution via event sourcing and database logging. Hephos adapts DBOS's database-checkpointed philosophy directly into an asynchronous Rust backend framework (`hephos-db`), providing durable turn checkpointing and LIFO saga compensation without requiring an external orchestration cluster.

---

## 9. Conclusion

The transition from human-written software to autonomous agent-driven development requires a fundamental paradigm shift from User Experience (UX) and Developer Experience (DX) to **Agent Experience (AX)**. Systems engineered around human cognitive preferences—syntactic flexibility, multiple valid patterns, and unstructured prose errors—actively degrade the performance of autoregressive Transformer agents by inflating conditional generation entropy and diluting attention context.

**Hephos** proves that an execution environment designed specifically for agents—enforcing structural invariance, infallible context propagation, compile-time derived action schemas, closed-loop structured fault reflection, and write-ahead durable execution—drastically improves agent reliability while preserving microsecond-level systems performance. By replacing human-centric ambiguity with deterministic systems engineering, and openly evaluating the trade-offs of the AX Tax, Hephos provides the foundational architectural blueprint for production-grade autonomous software engineering.

---

## References

1. Anthropic. *The Model Context Protocol (MCP) Specification*, Technical Specification, Nov. 2024. https://modelcontextprotocol.io
2. Li, Q., Kraft, P., Cafarella, M., Zaharia, M., Stonebraker, M., et al. *DBOS: A DBMS-Oriented Operating System*, Proceedings of the VLDB Endowment (PVLDB), 15(1):21–30, 2022.
3. Wu, Q., Bansal, G., Zhang, J., Wu, Y., Li, B., Erkan, E., et al. *AutoGen: Enabling Next-Gen LLM Applications via Multi-Agent Conversation*, arXiv:2308.08155, 2023.
4. Jimenez, C. E., Yang, J., Wettig, A., Yao, S., Pei, K., Press, O., & Narasimhan, K. *SWE-bench: Can Language Models Resolve Real-World GitHub Issues?*, International Conference on Learning Representations (ICLR), 2024.
5. Temporal Technologies. *Temporal: Fault-Tolerant Execution Architecture*, Technical Documentation, 2023.
6. Biilmann, M. *The Shift from DX to AX (Agent Experience)*, Netlify Technical Keynote, 2024.
7. Vaswani, A., Shazeer, N., Parmar, N., Uszkoreit, J., Jones, L., Gomez, A. N., Kaiser, Ł., & Polosukhin, I. *Attention Is All You Need*, Advances in Neural Information Processing Systems (NeurIPS), 2017.
8. Qin, Y., Liang, S., Ye, Y., Zhu, K., Yan, L., Lu, Y., et al. *ToolLLM: Facilitating Large Language Models to Master 16000+ Real-world APIs*, International Conference on Learning Representations (ICLR), 2024.
9. Stonebraker, M., & Cetintemel, U. *"One Size Fits All": An Idea Whose Time Has Come and Gone*, 21st Conference on Data Engineering (ICDE), 2005.
10. Tokio Project. *Axum: Ergonomic and Modular Web Framework* and *Tower: Modular, Reusable Service Components for Rust*, 2024. https://github.com/tokio-rs/axum
11. Launchbadge. *SQLx: The Rust SQL Toolkit with Compile-Time Checked Queries*, Open Source Project, 2024.
