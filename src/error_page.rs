use axum::http::StatusCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    English,
    Chinese,
}

impl Language {
    pub(crate) fn from_code(code: &str) -> Self {
        if code == "zh" {
            Self::Chinese
        } else {
            Self::English
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    Dns,
    Connection,
    Timeout,
    Upstream,
}

pub(crate) struct ErrorPage<'a> {
    pub language: Language,
    pub status: StatusCode,
    pub domain: &'a str,
    pub failure_kind: FailureKind,
}

struct LocalizedCopy {
    html_lang: &'static str,
    gateway_status: &'static str,
    title: &'static str,
    description: &'static str,
    target_label: &'static str,
    state_label: &'static str,
    retry_label: &'static str,
    diagnostic_label: &'static str,
    client_label: &'static str,
    mirrox_label: &'static str,
    upstream_label: &'static str,
    reachable_label: &'static str,
    operating_label: &'static str,
    failed_label: &'static str,
    footer_note: &'static str,
}

pub(crate) fn render(page: ErrorPage<'_>) -> String {
    let copy = localized_copy(page.language, page.failure_kind);
    let status = page.status.as_str();
    let reason = page.status.canonical_reason().unwrap_or("Proxy Error");
    let kind = failure_kind_code(page.failure_kind);
    let domain = escape_html(page.domain);
    let title = escape_html(copy.title);
    let description = escape_html(copy.description);

    format!(
        r#"<!doctype html>
<html lang="{html_lang}">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="robots" content="noindex, nofollow">
    <title>{status} {reason} · mirrox</title>
    <style>{styles}</style>
</head>
<body>
    <main class="shell" aria-labelledby="error-title">
        <header class="masthead">
            <span class="wordmark">mirrox</span>
            <span class="system-label">{gateway_status} / {kind}</span>
        </header>

        <section class="error-layout" aria-describedby="error-description">
            <div class="status-block">
                <div class="status-code">{status}</div>
                <div class="status-reason">{reason}</div>
            </div>

            <div class="error-copy">
                <div class="signal-line"><span aria-hidden="true"></span>{kind}</div>
                <h1 id="error-title">{title}</h1>
                <p class="description" id="error-description">{description}</p>

                <dl class="facts">
                    <div>
                        <dt>{target_label}</dt>
                        <dd>{domain}</dd>
                    </div>
                    <div>
                        <dt>{state_label}</dt>
                        <dd class="failure-state">{failed_label}</dd>
                    </div>
                </dl>

                <a class="retry" href="">{retry_label}<span aria-hidden="true">↗</span></a>
            </div>
        </section>

        <section class="diagnostic" aria-label="{diagnostic_label}">
            <div class="node">
                <span class="node-index">01</span>
                <strong>{client_label}</strong>
                <small>{reachable_label}</small>
            </div>
            <div class="connector" aria-hidden="true"><span></span></div>
            <div class="node">
                <span class="node-index">02</span>
                <strong>{mirrox_label}</strong>
                <small>{operating_label}</small>
            </div>
            <div class="connector broken" aria-hidden="true"><span></span></div>
            <div class="node failed">
                <span class="node-index">03</span>
                <strong>{upstream_label}</strong>
                <small>{failed_label}</small>
            </div>
        </section>

        <footer>
            <span>{footer_note}</span>
            <span class="footer-code">HTTP/{status}</span>
        </footer>
    </main>
</body>
</html>"#,
        html_lang = copy.html_lang,
        status = status,
        reason = reason,
        styles = STYLES,
        gateway_status = copy.gateway_status,
        kind = kind,
        title = title,
        description = description,
        target_label = copy.target_label,
        domain = domain,
        state_label = copy.state_label,
        failed_label = copy.failed_label,
        retry_label = copy.retry_label,
        diagnostic_label = copy.diagnostic_label,
        client_label = copy.client_label,
        reachable_label = copy.reachable_label,
        mirrox_label = copy.mirrox_label,
        operating_label = copy.operating_label,
        upstream_label = copy.upstream_label,
        footer_note = copy.footer_note,
    )
}

fn localized_copy(language: Language, kind: FailureKind) -> LocalizedCopy {
    match language {
        Language::English => LocalizedCopy {
            html_lang: "en",
            gateway_status: "gateway status",
            title: match kind {
                FailureKind::Dns => "DNS resolution failed",
                FailureKind::Connection => "Upstream connection failed",
                FailureKind::Timeout => "Connection timed out",
                FailureKind::Upstream => "Upstream service returned an error",
            },
            description: match kind {
                FailureKind::Dns => {
                    "Mirrox could not resolve the configured upstream domain. The request was not forwarded."
                }
                FailureKind::Connection => {
                    "Mirrox received your request, but could not establish a connection to the upstream service."
                }
                FailureKind::Timeout => {
                    "The upstream service did not respond within the configured time limit."
                }
                FailureKind::Upstream => {
                    "The request reached the upstream service, which returned an unsuccessful response."
                }
            },
            target_label: "Target",
            state_label: "State",
            retry_label: "Retry request",
            diagnostic_label: "Request path diagnostic",
            client_label: "CLIENT",
            mirrox_label: "MIRROX",
            upstream_label: "UPSTREAM",
            reachable_label: "reachable",
            operating_label: "operating",
            failed_label: "interrupted",
            footer_note: "Self-contained diagnostic response",
        },
        Language::Chinese => LocalizedCopy {
            html_lang: "zh-CN",
            gateway_status: "网关状态",
            title: match kind {
                FailureKind::Dns => "DNS 解析失败",
                FailureKind::Connection => "上游连接失败",
                FailureKind::Timeout => "连接超时",
                FailureKind::Upstream => "上游服务返回错误",
            },
            description: match kind {
                FailureKind::Dns => "Mirrox 无法解析配置的上游域名，请求尚未被转发。",
                FailureKind::Connection => {
                    "Mirrox 已收到你的请求，但无法与上游服务建立连接。"
                }
                FailureKind::Timeout => "上游服务未在配置的时间限制内响应。",
                FailureKind::Upstream => "请求已到达上游服务，但上游返回了非成功响应。",
            },
            target_label: "目标",
            state_label: "状态",
            retry_label: "重试请求",
            diagnostic_label: "请求链路诊断",
            client_label: "客户端",
            mirrox_label: "MIRROX",
            upstream_label: "上游服务",
            reachable_label: "可访问",
            operating_label: "运行中",
            failed_label: "链路中断",
            footer_note: "自包含诊断响应",
        },
    }
}

fn failure_kind_code(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Dns => "DNS / RESOLUTION",
        FailureKind::Connection => "PROXY / CONNECT",
        FailureKind::Timeout => "PROXY / TIMEOUT",
        FailureKind::Upstream => "UPSTREAM / RESPONSE",
    }
}

fn escape_html(value: &str) -> String {
    value.chars().fold(String::new(), |mut output, ch| {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(ch),
        }
        output
    })
}

const STYLES: &str = r#"
:root {
    color-scheme: light dark;
    --paper: #efede6;
    --ink: #1b1b18;
    --muted: #6a685f;
    --line: #c9c5ba;
    --accent: #a24036;
    --accent-soft: #e5d1ca;
    --focus: #246da3;
    --sans: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    --mono: "SFMono-Regular", "Cascadia Code", "Roboto Mono", Consolas, monospace;
}

* { box-sizing: border-box; }
html { min-width: 18rem; background: var(--paper); }
body {
    margin: 0;
    min-height: 100dvh;
    background: var(--paper);
    color: var(--ink);
    font-family: var(--sans);
    -webkit-font-smoothing: antialiased;
}

.shell {
    width: min(100%, 90rem);
    min-height: 100dvh;
    margin: 0 auto;
    padding: clamp(1.25rem, 3vw, 3rem) clamp(1.25rem, 5vw, 5rem) 1.5rem;
    display: grid;
    grid-template-rows: auto 1fr auto auto;
}

.masthead,
footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    font-family: var(--mono);
    font-size: .72rem;
    letter-spacing: .08em;
    text-transform: uppercase;
}

.masthead {
    padding-bottom: 1.15rem;
    border-bottom: 1px solid var(--line);
}
.wordmark { font-weight: 750; letter-spacing: -.02em; text-transform: lowercase; }
.system-label, footer { color: var(--muted); }

.error-layout {
    display: grid;
    grid-template-columns: minmax(0, .95fr) minmax(20rem, 1.05fr);
    gap: clamp(3rem, 8vw, 9rem);
    align-items: center;
    padding: clamp(4rem, 10vh, 8rem) 0;
}
.status-block { align-self: center; min-width: 0; }
.status-code {
    font-family: var(--mono);
    font-size: clamp(7.5rem, 17vw, 15rem);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    letter-spacing: -.1em;
    line-height: .76;
    white-space: nowrap;
}
.status-reason {
    margin-top: 1.4rem;
    color: var(--muted);
    font-family: var(--mono);
    font-size: clamp(.72rem, 1vw, .86rem);
    letter-spacing: .08em;
    text-transform: uppercase;
}

.error-copy { max-width: 38rem; }
.signal-line {
    display: flex;
    align-items: center;
    gap: .65rem;
    color: var(--accent);
    font-family: var(--mono);
    font-size: .72rem;
    font-weight: 700;
    letter-spacing: .09em;
}
.signal-line span {
    width: .55rem;
    height: .55rem;
    background: var(--accent);
    border-radius: 50%;
    box-shadow: 0 0 0 .3rem var(--accent-soft);
}
h1 {
    max-width: 15ch;
    margin: 1.2rem 0 0;
    font-size: clamp(2.2rem, 4.6vw, 4.9rem);
    font-weight: 600;
    letter-spacing: -.055em;
    line-height: .98;
    text-wrap: balance;
}
.description {
    max-width: 57ch;
    margin: 1.7rem 0 0;
    color: var(--muted);
    font-size: clamp(1rem, 1.3vw, 1.14rem);
    line-height: 1.65;
    text-wrap: pretty;
}

.facts {
    margin: 2rem 0 0;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
}
.facts div {
    display: grid;
    grid-template-columns: 6rem minmax(0, 1fr);
    gap: 1rem;
    padding: .95rem 0;
}
.facts div + div { border-top: 1px solid var(--line); }
dt {
    color: var(--muted);
    font-family: var(--mono);
    font-size: .7rem;
    letter-spacing: .08em;
    text-transform: uppercase;
}
dd {
    min-width: 0;
    margin: 0;
    font-family: var(--mono);
    font-size: .82rem;
    overflow-wrap: anywhere;
}
.failure-state { color: var(--accent); }

.retry {
    display: inline-flex;
    align-items: center;
    gap: 1rem;
    margin-top: 1.8rem;
    padding: .78rem 1rem;
    border: 1px solid var(--ink);
    background: var(--ink);
    color: var(--paper);
    font-size: .82rem;
    font-weight: 650;
    text-decoration: none;
    transition: background-color 140ms ease, color 140ms ease;
}
.retry span { font-family: var(--mono); }
.retry:hover { background: transparent; color: var(--ink); }
.retry:focus-visible { outline: 3px solid var(--focus); outline-offset: 4px; }

.diagnostic {
    display: grid;
    grid-template-columns: minmax(8rem, 1fr) 3rem minmax(8rem, 1fr) 3rem minmax(8rem, 1fr);
    align-items: center;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
}
.node { position: relative; padding: 1rem 0; }
.node-index {
    display: block;
    margin-bottom: .3rem;
    color: var(--muted);
    font-family: var(--mono);
    font-size: .63rem;
}
.node strong {
    display: block;
    font-family: var(--mono);
    font-size: .76rem;
    letter-spacing: .07em;
}
.node small { color: var(--muted); font-size: .7rem; }
.node.failed strong, .node.failed small { color: var(--accent); }
.connector { position: relative; height: 1px; background: var(--line); }
.connector::after {
    content: "";
    position: absolute;
    right: -.05rem;
    top: -.18rem;
    width: .35rem;
    height: .35rem;
    border-top: 1px solid var(--line);
    border-right: 1px solid var(--line);
    transform: rotate(45deg);
}
.connector.broken { background: transparent; border-top: 1px dashed var(--accent); }
.connector.broken::before {
    content: "×";
    position: absolute;
    inset: 50% auto auto 50%;
    padding: 0 .25rem;
    background: var(--paper);
    color: var(--accent);
    font-family: var(--mono);
    transform: translate(-50%, -52%);
}
.connector.broken::after { border-color: var(--accent); }

footer { padding-top: 1.25rem; }
.footer-code { font-variant-numeric: tabular-nums; }

@media (max-width: 720px) {
    .shell { display: block; }
    .error-layout {
        grid-template-columns: 1fr;
        gap: 3.25rem;
        padding: 4.5rem 0 4rem;
    }
    .status-code { font-size: clamp(7rem, 38vw, 10rem); }
    h1 { max-width: 12ch; }
    .diagnostic {
        grid-template-columns: 1fr;
        padding: .35rem 0;
    }
    .node { padding: .85rem 0 .85rem 2rem; }
    .node-index { position: absolute; left: 0; top: 1rem; }
    .connector {
        width: 1px;
        height: 1.25rem;
        margin-left: .16rem;
    }
    .connector::after {
        right: -.17rem;
        top: auto;
        bottom: 0;
        transform: rotate(135deg);
    }
    .connector.broken { border-top: 0; border-left: 1px dashed var(--accent); }
    .connector.broken::before { inset: 50% auto auto 0; }
    footer { margin-top: 1.25rem; align-items: flex-end; }
    footer span:first-child { max-width: 18ch; }
}

@media (prefers-color-scheme: dark) {
    :root {
        --paper: #171714;
        --ink: #edebe4;
        --muted: #aaa69b;
        --line: #3e3c36;
        --accent: #dc7468;
        --accent-soft: #422823;
        --focus: #78b9e6;
    }
}

@media (prefers-reduced-motion: reduce) {
    *, *::before, *::after {
        scroll-behavior: auto !important;
        transition-duration: .001ms !important;
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::{render, ErrorPage, FailureKind, Language};
    use axum::http::StatusCode;

    #[test]
    fn renders_semantic_self_contained_english_page() {
        let html = render(ErrorPage {
            language: Language::English,
            status: StatusCode::BAD_GATEWAY,
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
        assert!(html.contains("prefers-reduced-motion: reduce"));
        assert!(html.contains("href=\"\""));
        assert!(!html.contains("<script"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn renders_localized_chinese_page() {
        let html = render(ErrorPage {
            language: Language::Chinese,
            status: StatusCode::GATEWAY_TIMEOUT,
            domain: "api.example.com",
            failure_kind: FailureKind::Timeout,
        });

        assert!(html.contains("<html lang=\"zh-CN\">"));
        assert!(html.contains("连接超时"));
        assert!(html.contains("重试请求"));
    }

    #[test]
    fn escapes_dynamic_error_page_values() {
        let html = render(ErrorPage {
            language: Language::English,
            status: StatusCode::BAD_GATEWAY,
            domain: "<script>alert(1)</script>&\"",
            failure_kind: FailureKind::Upstream,
        });

        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;&amp;&quot;"));
        assert!(!html.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn visible_status_is_always_the_original_error_status() {
        let html = render(ErrorPage {
            language: Language::English,
            status: StatusCode::BAD_GATEWAY,
            domain: "api.example.com",
            failure_kind: FailureKind::Connection,
        });

        assert!(html.contains(">502<"));
        assert!(html.contains("Bad Gateway"));
        assert!(!html.contains(">200<"));
    }
}
