#!/usr/bin/env bash
#
# Architecture-boundary lint — enforce the layering rules from CLAUDE.md §2 in CI, so a
# violation fails the build instead of relying on the doctrine being read. A `CLAUDE.md`
# is a suggestion; this is a gate.
#
# Scope: application code only (`examples/*/src`, the reference apps and the shape every
# real app copies). The framework crates (`crates/`) are exempt — they *are* the HTTP and
# SQL layer, so they use axum and sqlx freely.
#
# The rules are intentionally few and load-bearing — the boundaries that, if crossed,
# collapse the whole layering model:
#   - handlers (api/)      must not run SQL or reach into repos  → go through a service
#   - services (services/) must not speak HTTP (axum / StatusCode) → stay transport-agnostic
#   - repos (repos/)       must not call up into services        → SQL only
#
# Run locally with `make arch-lint`.

set -uo pipefail
cd "$(dirname "$0")/.."
shopt -s nullglob

status=0

# check "<description>" "<grep -E regex>" <file...>
check() {
    local desc="$1" regex="$2"
    shift 2
    local files=("$@")
    [ ${#files[@]} -eq 0 ] && return 0
    local hits
    hits="$(grep -nE "$regex" "${files[@]}" 2>/dev/null || true)"
    if [ -n "$hits" ]; then
        printf '✗ %s\n' "$desc"
        printf '%s\n' "$hits" | sed 's/^/    /'
        printf '\n'
        status=1
    fi
}

echo "== Architecture-boundary lint (CLAUDE.md §2 — application code) =="
echo

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

if [ "$status" -eq 0 ]; then
    echo "✓ no boundary violations"
fi
exit "$status"
