# Deterministic Rewrite and Error Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Mirrox v0.3.0 with deterministic, boundary-safe domain rewriting, CDN-safe 5xx pages, a redesigned self-contained error interface, and targeted infrastructure hardening.

**Architecture:** `RouteTable` retains a lookup map and an ordered route list, producing a deterministic `RewritePlan` whose current route has priority and whose ambiguous fallbacks use lexical ordering. `RewritePlan` scans original text once with DNS-label boundaries and is shared by body and `Location` rewriting. Error rendering separates original and outward status and moves localized HTML into a dedicated `error_page` module.

**Tech Stack:** Rust 2021, Axum 0.7, Tokio, Hickory Resolver, Tower HTTP, vanilla inline HTML/CSS, Cargo integration tests.

---

## File responsibility map

- `src/rewrite.rs`: immutable rewrite rules, one-pass boundary scanner, Cookie Domain rewriting, content-type helpers.
- `src/routing.rs`: host lookup plus deterministic rewrite-plan selection.
- `src/proxy.rs`: request forwarding and integration of rewrite/error context.
- `src/error.rs`: structured application errors, status policy, response headers.
- `src/error_page.rs`: localized copy, HTML escaping, semantic self-contained error-page rendering.
- `src/config.rs`: validation of `proxy_error_status` and duplicate incoming routes.
- `src/dns.rs`: configured Hickory resolver construction.
- `src/server.rs`: safe tracing and immediate graceful-shutdown signal.
- `tests/*`: public behavior and regression coverage.
- `README.md`, `docs/*.md`, `examples/config.example.toml`: operator behavior and Cloudflare guidance.

### Task 1: Boundary-safe, non-cascading rewrite engine

**Files:**
- Modify: `src/rewrite.rs`
- Modify: `tests/rewrite_tests.rs`

- [ ] **Step 1: Add failing rewrite-plan tests**

```rust
use mirrox::rewrite::{
    is_rewritable_content_type, rewrite_cookie_domain, rewrite_header_value,
    rewrite_text_body, RewritePlan, RewriteRule,
};

fn bangumi_plan() -> RewritePlan {
    RewritePlan::new(vec![
        RewriteRule::new("api.bgm.tv", "api.bangumi.lol"),
        RewriteRule::new("bgm.tv", "bangumi.lol"),
    ])
}

#[test]
fn rewrite_plan_rewrites_exact_and_unlisted_subdomains() {
    assert_eq!(
        bangumi_plan().rewrite(
            "https://api.bgm.tv/v0 https://foo.bgm.tv/path BGM.TV:443"
        ),
        "https://api.bangumi.lol/v0 https://foo.bangumi.lol/path bangumi.lol:443"
    );
}

#[test]
fn rewrite_plan_respects_dns_label_boundaries() {
    assert_eq!(
        bangumi_plan().rewrite(
            "fxbgm.tv evil-bgm.tv bgm.tv.cn x_bgm.tv https://bgm.tv/path"
        ),
        "fxbgm.tv evil-bgm.tv bgm.tv.cn x_bgm.tv https://bangumi.lol/path"
    );
}

#[test]
fn rewrite_plan_does_not_cascade_replacements() {
    let plan = RewritePlan::new(vec![
        RewriteRule::new("a.example", "b.example"),
        RewriteRule::new("b.example", "c.example"),
    ]);
    assert_eq!(plan.rewrite("https://a.example"), "https://b.example");
}

#[test]
fn rewrites_cookie_domain_case_insensitively_without_substring_matches() {
    assert_eq!(
        rewrite_cookie_domain("id=1; DOMAIN=.BGM.TV; Path=/", "bgm.tv", "bangumi.lol"),
        "id=1; DOMAIN=.bangumi.lol; Path=/"
    );
    assert_eq!(
        rewrite_cookie_domain("id=1; Domain=.fxbgm.tv; Path=/", "bgm.tv", "bangumi.lol"),
        "id=1; Domain=.fxbgm.tv; Path=/"
    );
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test --test rewrite_tests -- --nocapture`

Expected: compilation fails because `RewritePlan` and `RewriteRule` do not exist.

- [ ] **Step 3: Implement normalized rules and a single-pass scanner**

```rust
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteRule {
    from: String,
    to: String,
}

impl RewriteRule {
    pub fn new(from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        Self {
            from: from.as_ref().trim().trim_end_matches('.').to_ascii_lowercase(),
            to: to.as_ref().trim().trim_end_matches('.').to_ascii_lowercase(),
        }
    }
    pub fn from(&self) -> &str { &self.from }
    pub fn to(&self) -> &str { &self.to }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewritePlan {
    rules: Vec<RewriteRule>,
}

impl RewritePlan {
    pub fn new(mut rules: Vec<RewriteRule>) -> Self {
        rules.retain(|rule| !rule.from.is_empty() && rule.from != rule.to);
        rules.sort_by(|a, b| {
            b.from.len().cmp(&a.from.len())
                .then_with(|| a.from.cmp(&b.from))
                .then_with(|| a.to.cmp(&b.to))
        });
        rules.dedup_by(|a, b| a.from == b.from);
        Self { rules }
    }

    pub fn rules(&self) -> &[RewriteRule] { &self.rules }

    pub fn rewrite<'a>(&self, input: &'a str) -> Cow<'a, str> {
        let bytes = input.as_bytes();
        let mut output = String::new();
        let mut copied_until = 0;
        let mut cursor = 0;
        while cursor < bytes.len() {
            let matched = self.rules.iter().find(|rule| {
                let end = cursor + rule.from.len();
                end <= bytes.len()
                    && bytes[cursor..end].eq_ignore_ascii_case(rule.from.as_bytes())
                    && left_boundary(bytes, cursor)
                    && right_boundary(bytes, end)
            });
            if let Some(rule) = matched {
                output.push_str(&input[copied_until..cursor]);
                output.push_str(&rule.to);
                cursor += rule.from.len();
                copied_until = cursor;
            } else {
                cursor += input[cursor..].chars().next().map(char::len_utf8).unwrap_or(1);
            }
        }
        if copied_until == 0 {
            Cow::Borrowed(input)
        } else {
            output.push_str(&input[copied_until..]);
            Cow::Owned(output)
        }
    }
}

fn left_boundary(input: &[u8], start: usize) -> bool {
    start == 0 || !matches!(input[start - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-')
}

fn right_boundary(input: &[u8], end: usize) -> bool {
    end == input.len()
        || !matches!(input[end], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.')
}
```

Make `rewrite_text_body` and `rewrite_header_value` compatibility wrappers around a one-rule plan. Rewrite Cookie Domain by parsing semicolon-delimited attributes, matching `Domain=` case-insensitively, and replacing only a leading-dot-tolerant exact domain value.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run: `cargo test --test rewrite_tests -- --nocapture`

Expected: every rewrite test passes.

- [ ] **Step 5: Commit the rewrite engine**

```bash
git add src/rewrite.rs tests/rewrite_tests.rs
git commit -m "fix: enforce domain boundaries during rewriting"
```

### Task 2: Deterministic rewrite-plan selection

**Files:**
- Modify: `src/routing.rs`
- Modify: `tests/routing_tests.rs`

- [ ] **Step 1: Add failing shared-upstream tests**

```rust
#[test]
fn current_route_wins_for_its_shared_upstream() {
    let table = shared_upstream_table(false);
    let route = table.match_host("api.bangumi.one").unwrap();
    assert_eq!(
        table.rewrite_plan_for(&route).rewrite("https://api.bgm.tv/v0"),
        "https://api.bangumi.one/v0"
    );
}

#[test]
fn ambiguous_sibling_uses_lexicographically_first_incoming() {
    let table = shared_upstream_table(false);
    let route = table.match_host("www.bangumi.one").unwrap();
    assert_eq!(
        table.rewrite_plan_for(&route).rewrite("https://api.bgm.tv/v0"),
        "https://api.bangumi.lol/v0"
    );
}

#[test]
fn rewrite_plan_is_independent_of_declaration_order() {
    let first = shared_upstream_table(false);
    let reversed = shared_upstream_table(true);
    let a = first.match_host("www.bangumi.one").unwrap();
    let b = reversed.match_host("www.bangumi.one").unwrap();
    let input = "https://api.bgm.tv https://foo.bgm.tv";
    assert_eq!(
        first.rewrite_plan_for(&a).rewrite(input),
        reversed.rewrite_plan_for(&b).rewrite(input)
    );
}
```

The helper builds the same `.one` and `.lol` aliases in opposite TOML declaration orders.

- [ ] **Step 2: Run routing tests and verify RED**

Run: `cargo test --test routing_tests -- --nocapture`

Expected: compilation fails because `rewrite_plan_for` does not exist.

- [ ] **Step 3: Retain ordered routes and build deterministic plans**

Add `ordered_exact: Vec<MatchedRoute>` to `RouteTable`, derive the lookup map from it, and implement:

```rust
pub fn rewrite_plan_for(&self, current: &MatchedRoute) -> RewritePlan {
    let mut selected: BTreeMap<String, String> = BTreeMap::new();
    for route in &self.ordered_exact {
        selected
            .entry(route.upstream_host.clone())
            .and_modify(|incoming| {
                if route.incoming_host < *incoming {
                    incoming.clone_from(&route.incoming_host);
                }
            })
            .or_insert_with(|| route.incoming_host.clone());
    }
    selected.insert(current.upstream_host.clone(), current.incoming_host.clone());
    RewritePlan::new(
        selected.into_iter()
            .map(|(from, to)| RewriteRule::new(from, to))
            .collect(),
    )
}
```

Keep `rewrite_pairs_for` as a compatibility adapter over `plan.rules()`. Make `all_rewrite_pairs` choose lexical targets from `ordered_exact` and sort deterministically.

- [ ] **Step 4: Run routing and rewrite tests and verify GREEN**

Run: `cargo test --test routing_tests --test rewrite_tests`

Expected: all tests pass.

- [ ] **Step 5: Commit route planning**

```bash
git add src/routing.rs tests/routing_tests.rs
git commit -m "fix: make shared-upstream rewrites deterministic"
```

### Task 3: Integrate one plan into HTTP responses

**Files:**
- Modify: `src/proxy.rs`
- Modify: `tests/proxy_tests.rs`

- [ ] **Step 1: Add a failing proxy regression test**

Configure `.one` and `.lol`, request through `api.bangumi.lol`, and serve a body plus `Location` containing `api.bgm.tv`, `foo.bgm.tv`, and `fxbgm.tv`:

```rust
response.assert_status_ok();
assert_eq!(response.headers()["location"], "https://api.bangumi.lol/v0");
response.assert_text_contains("https://api.bangumi.lol/v0");
response.assert_text_contains("https://foo.bangumi.lol/path");
response.assert_text_contains("fxbgm.tv");
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test --test proxy_tests shared_upstream -- --nocapture`

Expected: current code selects the wrong alias or rewrites `fxbgm.tv`.

- [ ] **Step 3: Share one plan between response headers and body**

Build `let rewrite_plan = state.routes.rewrite_plan_for(&route);` once. Pass `&RewritePlan` into response-header and buffered-body helpers. Replace pair loops with `rewrite_plan.rewrite(value)` and `rewrite_plan.rewrite(&text)`. Keep current-route reverse request rewriting and attribute-aware Cookie Domain rewriting.

- [ ] **Step 4: Run proxy, route, and rewrite tests and verify GREEN**

Run: `cargo test --test proxy_tests --test routing_tests --test rewrite_tests`

Expected: all tests pass.

- [ ] **Step 5: Commit proxy integration**

```bash
git add src/proxy.rs tests/proxy_tests.rs
git commit -m "fix: apply response rewrite plans in one pass"
```

### Task 4: Separate original and outward error status

**Files:**
- Modify: `src/config.rs`
- Modify: `src/error.rs`
- Modify: `tests/config_tests.rs`
- Modify: `tests/proxy_tests.rs`

- [ ] **Step 1: Add failing policy tests**

Add accepted `200`, rejected `204`, a 502-to-200 response, and a 404 that remains 404:

```rust
assert_eq!(config.server.proxy_error_status, Some(200));
assert!(error.to_string().contains("proxy_error_status"));
response.assert_status_ok();
assert_eq!(response.headers()["x-mirrox-error-status"], "502");
assert_eq!(response.headers()["cache-control"], "no-store");
response.assert_text_contains("502");
response.assert_text_not_contains(">200<");
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --test config_tests proxy_error_status -- --nocapture; cargo test --test proxy_tests proxy_error_status -- --nocapture`

Expected: validation and original/outward status assertions fail.

- [ ] **Step 3: Implement named context and status policy**

Use `ErrorContext { incoming_host, language, proxy_error_status }` and:

```rust
fn response_status(original: StatusCode, override_status: Option<StatusCode>) -> StatusCode {
    if original.is_server_error() {
        override_status.unwrap_or(original)
    } else {
        original
    }
}
```

Validate the configured code through `StatusCode::from_u16`, rejecting informational, 204, 205, and 304. Render with the original status, send the outward status, and add `X-Mirrox-Error-Status`, `Cache-Control: no-store`, and `X-Content-Type-Options: nosniff` to custom 5xx pages. Replace response-path `unwrap()` with a safe fallback response.

- [ ] **Step 4: Run config and proxy tests and verify GREEN**

Run: `cargo test --test config_tests --test proxy_tests`

Expected: all tests pass.

- [ ] **Step 5: Commit status policy**

```bash
git add src/config.rs src/error.rs tests/config_tests.rs tests/proxy_tests.rs
git commit -m "fix: preserve original status in proxied error pages"
```

### Task 5: Redesign and isolate the bilingual error page

**Files:**
- Create: `src/error_page.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Test: `src/error_page.rs`

- [ ] **Step 1: Add failing renderer tests**

```rust
#[test]
fn renders_semantic_self_contained_english_page() {
    let html = render(ErrorPage {
        language: Language::English,
        status: StatusCode::BAD_GATEWAY,
        title: "Upstream unavailable",
        description: "The upstream did not respond.",
        domain: "api.example.com",
        failure_kind: FailureKind::Upstream,
    });
    assert!(html.contains("<html lang=\"en\">"));
    assert!(html.contains("<main"));
    assert!(html.contains("CLIENT"));
    assert!(html.contains("MIRROX"));
    assert!(html.contains("UPSTREAM"));
    assert!(html.contains("@media (max-width: 720px)"));
    assert!(html.contains("prefers-color-scheme: dark"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("https://"));
}

#[test]
fn escapes_dynamic_error_page_values() {
    let html = render(ErrorPage {
        language: Language::English,
        status: StatusCode::BAD_GATEWAY,
        title: "<unsafe>",
        description: "A & B",
        domain: "<script>alert(1)</script>",
        failure_kind: FailureKind::Upstream,
    });
    assert!(html.contains("&lt;unsafe&gt;"));
    assert!(html.contains("A &amp; B"));
    assert!(!html.contains("<script>alert(1)</script>"));
}
```

- [ ] **Step 2: Run library tests and verify RED**

Run: `cargo test --lib error_page -- --nocapture`

Expected: compilation fails because the module and types do not exist.

- [ ] **Step 3: Implement a typed self-contained renderer**

Create:

```rust
pub enum Language { English, Chinese }
pub enum FailureKind { Dns, Proxy, Timeout, Upstream }

pub struct ErrorPage<'a> {
    pub language: Language,
    pub status: StatusCode,
    pub title: &'a str,
    pub description: &'a str,
    pub domain: &'a str,
    pub failure_kind: FailureKind,
}

pub fn escape_html(value: &str) -> String {
    value.chars().fold(String::new(), |mut out, ch| {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
        out
    })
}
```

Render the approved asymmetric status layout, localized explanation, affected domain, retry link `href=""`, `CLIENT -> MIRROX -> UPSTREAM` rail, warm/light and dark CSS tokens, `clamp()` type, visible focus, 720px mobile collapse, and reduced-motion media query. Escape every dynamic string. Move locale selection and template rendering out of `error.rs`.

- [ ] **Step 4: Run renderer and proxy tests and verify GREEN**

Run: `cargo test --lib error_page -- --nocapture && cargo test --test proxy_tests`

Expected: all tests pass.

- [ ] **Step 5: Commit the error-page redesign**

```bash
git add src/error_page.rs src/error.rs src/lib.rs
git commit -m "feat: redesign the self-contained gateway error page"
```

### Task 6: Complete the infrastructure audit

**Files:**
- Modify: `src/config.rs`
- Modify: `src/dns.rs`
- Modify: `src/server.rs`
- Modify: `src/upstream_proxy.rs`
- Modify: `tests/config_tests.rs`
- Modify: `tests/server_tests.rs`

- [ ] **Step 1: Add audit regression tests**

Add duplicate normalized incoming and duplicate wildcard rejection tests. Add DNS parser/build tests proving configured protocol, timeout, and TTL options are consumed without external network resolution.

- [ ] **Step 2: Run focused tests and record RED/baseline results**

Run: `cargo test --test config_tests --test server_tests --lib dns -- --nocapture`

Expected: missing behavior fails; already-correct v0.2.9 cases remain passing baselines.

- [ ] **Step 3: Apply targeted hardening**

Keep Hickory wired to mode, servers, TTLs, and timeout. Reject duplicate incoming hosts and wildcard suffixes. Keep `TraceLayer` without `include_headers(true)`. Complete the shutdown signal immediately after SIGINT/SIGTERM instead of sleeping before shutdown. Replace repeated positional upstream errors with `AppError::upstream`. Preserve request IDs and proxy behavior.

- [ ] **Step 4: Format and run targeted tests**

Run: `cargo fmt && cargo test --test config_tests --test server_tests --test upstream_proxy_tests --lib`

Expected: all tests pass.

- [ ] **Step 5: Commit hardening**

```bash
git add src/config.rs src/dns.rs src/server.rs src/upstream_proxy.rs tests/config_tests.rs tests/server_tests.rs
git commit -m "refactor: harden proxy infrastructure"
```

### Task 7: Document operator behavior

**Files:**
- Modify: `README.md`
- Modify: `docs/README_zhcn.md`
- Modify: `docs/configuration.md`
- Modify: `docs/configuration_zhcn.md`
- Modify: `examples/config.example.toml`

- [ ] **Step 1: Add configuration examples**

```toml
# When a fronting CDN replaces 5xx bodies, expose Mirrox's custom page as 200.
# The original status remains in the page and X-Mirrox-Error-Status header.
# proxy_error_status = 200
```

- [ ] **Step 2: Document deterministic boundary-safe rewriting**

Describe current-route priority, lexical sibling fallback, longest-host precedence, root-route fallback, and why `fxbgm.tv` is not rewritten in both languages.

- [ ] **Step 3: Document Cloudflare limitations**

Explain that the override helps only when Mirrox receives the request; Cloudflare-to-Tunnel/Mirrox failures require Cloudflare-side handling.

- [ ] **Step 4: Verify documentation consistency**

```bash
rg -n "proxy_error_status|X-Mirrox-Error-Status|fxbgm\\.tv" \
  README.md docs/README_zhcn.md docs/configuration.md \
  docs/configuration_zhcn.md examples/config.example.toml
```

Expected: every operator document contains the relevant behavior.

- [ ] **Step 5: Commit documentation**

```bash
git add README.md docs/README_zhcn.md docs/configuration.md docs/configuration_zhcn.md examples/config.example.toml
git commit -m "docs: explain rewrite boundaries and CDN error pages"
```

### Task 8: Full verification and visual QA

**Files:**
- Modify only when verification finds a defect in an already-owned file.
- Keep screenshots and temporary browser scripts outside the repository.

- [ ] **Step 1: Run full Rust verification**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

Expected: every command exits 0 with zero warnings or failures.

- [ ] **Step 2: Render a representative error page outside the repo**

Generate an English 502 HTML fixture under `/tmp`. The Browser plugin is unavailable in this session, so use existing Playwright/Chromium when present without adding a project dependency.

- [ ] **Step 3: Verify desktop and mobile layouts**

Capture `/tmp/mirrox-error-desktop.png` at 1440x900 and `/tmp/mirrox-error-mobile.png` at 390x844. Inspect for clipping, overflow, contrast, hierarchy, focus visibility, diagnostic-rail collapse, and external requests. Delete temporary scripts after inspection.

- [ ] **Step 4: Review the complete diff**

```bash
git diff --check
git status --short
git log --oneline --decorate -12
```

Expected: no whitespace errors and only known user-owned untracked `.agents/` and `skills-lock.json` remain.

### Task 9: Release v0.3.0

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Set the release version**

Set `package.version = "0.3.0"` in `Cargo.toml` and run `cargo check` so `Cargo.lock` records `mirrox 0.3.0`.

- [ ] **Step 2: Verify version and full suite**

```bash
grep -n 'version = "0.3.0"' Cargo.toml Cargo.lock
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

Expected: both manifests contain 0.3.0 and every command exits 0.

- [ ] **Step 3: Commit the release**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: release v0.3.0"
```

- [ ] **Step 4: Create and verify the annotated tag**

```bash
git tag -a v0.3.0 -m "Release v0.3.0"
git show --no-patch --decorate v0.3.0
git status --short --branch
```

Expected: `v0.3.0` points to the release commit, `v0.2.9` is unchanged, and no task-owned files are dirty.
