# Deterministic Domain Rewriting and Proxy Error Pages Design

**Date:** 2026-07-16
**Target release:** v0.3.0
**Status:** Approved for implementation

## 1. Goals

1. Make response-domain rewriting deterministic when multiple incoming routes share one upstream.
2. Preserve the root-domain fallback that rewrites unlisted upstream subdomains, while preventing replacements inside unrelated DNS labels.
3. Ensure Mirrox-generated upstream 5xx pages can pass through a fronting CDN such as Cloudflare without losing the original error information.
4. Improve configuration validation, DNS behavior, error handling, observability, shutdown behavior, documentation, and test coverage without redesigning the proxy protocol.

## 2. Confirmed rewrite behavior

Given these routes:

```toml
[[routes]]
incoming = "api.bangumi.lol"
upstream = "api.bgm.tv"

[[routes]]
incoming = "bangumi.lol"
upstream = "bgm.tv"
```

Mirrox must produce the following results:

| Input | Output |
| --- | --- |
| `https://api.bgm.tv/v0` | `https://api.bangumi.lol/v0` |
| `https://foo.bgm.tv/path` | `https://foo.bangumi.lol/path` |
| `fxbgm.tv` | unchanged |
| `evil-bgm.tv` | unchanged |
| `bgm.tv.cn` | unchanged |
| `BGM.TV:443` | `bangumi.lol:443` |

The exact `bgm.tv -> bangumi.lol` route is the intentional fallback for unlisted subdomains such as `foo.bgm.tv`.

### 2.1 Shared-upstream precedence

Multiple incoming routes may map to the same upstream. They are not rejected. Rewrite target selection follows these rules:

1. The currently matched route wins for its own upstream host.
2. For another upstream with multiple incoming candidates, normalize hosts to lowercase and select the lexicographically smallest incoming host.
3. Sort selected rules by upstream-host length descending so an exact host such as `api.bgm.tv` precedes the fallback `bgm.tv`.
4. Break equal-length ties by normalized upstream and then normalized incoming host.

The selected rules must not depend on `HashMap` iteration order, source process, platform, or configuration ordering.

### 2.2 Boundary-aware replacement

The rewrite engine performs a single logical pass over the original input. Replacement output is not scanned again, preventing cascading rewrites.

A match is valid when:

- its left side is the beginning of input, a DNS-label separator `.`, or a non-host character;
- it is not immediately preceded by an ASCII letter, digit, `_`, or `-`;
- its right side is the end of input or a non-host-suffix character;
- it is not immediately followed by an ASCII letter, digit, `_`, `-`, or `.`.

Matching is ASCII case-insensitive because DNS hosts are case-insensitive. Replacements use normalized configured hosts.

The same host-boundary semantics apply to response bodies and `Location`. Cookie rewriting remains attribute-aware and rewrites only a Cookie `Domain` value, not arbitrary cookie text.

## 3. Error-page behavior

Mirrox distinguishes the original error status from the HTTP status sent through a fronting proxy.

```toml
[server]
proxy_error_status = 200
```

Behavior:

1. Without `proxy_error_status`, Mirrox returns the original status.
2. The override applies only when the original custom error response is a 5xx response.
3. Upstream 4xx responses retain their original status.
4. Error-page content and title always display the original status, even when the outward status is overridden.
5. Custom 5xx responses include:

```http
X-Mirrox-Error-Status: 502
Cache-Control: no-store
X-Content-Type-Options: nosniff
Content-Type: text/html; charset=utf-8
```

6. Configuration rejects response statuses that cannot carry the HTML page, including informational statuses, 204, 205, and 304.
7. A Cloudflare/Tunnel failure that occurs before the request reaches Mirrox cannot be rendered by Mirrox and must be handled at the Cloudflare layer.

## 4. Error-page frontend design

The error page is a reliable infrastructure surface, not a marketing page. It uses a restrained industrial-editorial visual language and remains fully self-contained.

### 4.1 Layout and hierarchy

- A compact top line identifies `mirrox / gateway status`.
- Desktop uses an asymmetric two-column composition: an oversized monospace original status code on the left and the localized explanation on the right.
- The details column presents the error title, concise explanation, affected domain, and a link that retries the current request.
- A lightweight diagnostic rail shows `CLIENT -> MIRROX -> UPSTREAM` and marks the failed upstream boundary.
- Mobile collapses to one column without clipping or horizontal scrolling.

### 4.2 Visual system

- Warm neutral background, near-black foreground, and one desaturated failure-red accent.
- System sans-serif for explanatory copy and the system monospace stack for status and diagnostic data, with no font downloads.
- Flat editorial surfaces with fine rules and deliberate whitespace; no generic centered card, glassmorphism, gradient mesh, illustration, or decorative icon dependency.
- Native dark-mode colors through `prefers-color-scheme: dark`.
- Any nonessential transition is disabled by `prefers-reduced-motion`.

### 4.3 Reliability and accessibility

- Inline semantic HTML and CSS only; no JavaScript, external image, external font, SVG fetch, or other network dependency.
- Localized Chinese and English documents remain separate and set the correct `lang` attribute.
- Visible keyboard focus, AA text contrast, responsive type through `clamp()`, and semantic `<main>`/`<section>` structure.
- The visible status always uses the original upstream/gateway error status, never the optional outward proxy status.
- Dynamic domain, status, title, and description values are HTML-escaped before interpolation.

## 5. Internal structure

### 5.1 Routing and rewrite planning

`RouteTable` keeps:

- an exact-host lookup map for request routing;
- an ordered, normalized collection for deterministic rewrite-plan construction;
- wildcard rules in deterministic configuration order.

A rewrite-plan builder deduplicates each upstream according to the precedence rules and produces immutable rewrite rules. The body and header rewrite paths consume this plan instead of repeatedly applying unrelated `String::replace` calls.

### 5.2 Error model

Gateway errors use named fields and a shared `ErrorContext` containing:

- normalized incoming host;
- selected response language;
- validated optional proxy error status.

The renderer computes `original_status` and `response_status` separately. Production response construction must not panic.

### 5.3 Proxy module boundaries

The HTTP forwarding path remains in `proxy.rs`, while reusable host rewriting stays in `rewrite.rs`. Helpers will be extracted only where they establish a clear responsibility or remove duplicated error construction. Public behavior unrelated to this design remains compatible.

## 6. Engineering audit scope

The repository-wide review includes:

- make configured DoH, DoT, UDP, TCP, timeout, and cache settings effective;
- reject duplicate incoming exact routes and duplicate wildcard suffixes;
- allow shared upstreams and resolve them deterministically;
- keep request IDs and structured tracing without logging complete request headers;
- begin graceful shutdown immediately when SIGINT or SIGTERM arrives rather than sleeping before shutdown starts;
- replace repeated positional gateway-error construction with named constructors/context;
- remove avoidable production `unwrap()` calls on response paths;
- preserve WebSocket, compression, script injection, upstream proxy, direct mode, and health endpoint behavior.

## 7. Testing strategy

Implementation follows red-green-refactor. Regression coverage includes:

- current-route precedence for shared upstreams;
- lexicographic selection for ambiguous sibling mappings;
- deterministic results across different route declaration orders;
- exact-before-root fallback ordering;
- positive fallback rewriting for arbitrary subdomains;
- negative boundary cases such as `fxbgm.tv`, `evil-bgm.tv`, and `bgm.tv.cn`;
- case-insensitive domain matching;
- non-cascading replacement;
- Cookie Domain boundary and casing behavior;
- original versus outward error status;
- error response headers and body status text;
- HTML escaping, bilingual document language, responsive CSS hooks, and self-contained error-page assets;
- 4xx non-override behavior;
- invalid `proxy_error_status` configuration;
- duplicate incoming route validation;
- DNS configuration parsing/building;
- existing HTTP, WebSocket, compression, script, and upstream-proxy regression suites.

## 8. Documentation and release

Update:

- `README.md`;
- `docs/README_zhcn.md`;
- `docs/configuration.md`;
- `docs/configuration_zhcn.md`;
- `examples/config.example.toml`.

After formatting, linting, tests, and release build succeed, commit the implementation, update both Cargo manifests to `0.3.0` in a release commit, and create annotated tag `v0.3.0`. The existing `v0.2.9` tag remains unchanged. No remote push is performed automatically.

## 9. Non-goals

- Automatically infer `.one` and `.lol` as coherent mirror families.
- Add a new `rewrite_group` configuration field.
- Parse and rewrite HTML through a DOM.
- Replace Cloudflare-generated errors that occur before Mirrox receives a request.
- Redesign the HTTP or WebSocket proxy architecture beyond the correctness and maintainability work described above.
