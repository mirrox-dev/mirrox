use mirrox::rewrite::{
    is_rewritable_content_type, rewrite_cookie_domain, rewrite_header_value, rewrite_text_body,
    RewritePlan, RewriteRule,
};

#[test]
fn rewrites_absolute_urls_in_text_body() {
    let body = r#"<a href="https://www.bgm.tv/subject/1">link</a><script src="//api.bgm.tv/v0.js"></script>"#;
    let rewritten = rewrite_text_body(body, "www.bgm.tv", "www.example.com");
    let rewritten = rewrite_text_body(&rewritten, "api.bgm.tv", "api.example.com");

    assert!(rewritten.contains("https://www.example.com/subject/1"));
    assert!(rewritten.contains("//api.example.com/v0.js"));
}

#[test]
fn rewrites_location_header() {
    let value = rewrite_header_value(
        "https://api.bgm.tv/v0/subjects/1",
        "api.bgm.tv",
        "api.example.com",
    );

    assert_eq!(value, "https://api.example.com/v0/subjects/1");
}

#[test]
fn rewrites_cookie_domain() {
    let value = rewrite_cookie_domain(
        "session=abc; Domain=.bgm.tv; Path=/",
        "bgm.tv",
        "example.com",
    );
    assert_eq!(value, "session=abc; Domain=.example.com; Path=/");
}

#[test]
fn recognizes_rewritable_content_types() {
    assert!(is_rewritable_content_type(Some("text/html; charset=utf-8")));
    assert!(is_rewritable_content_type(Some("application/javascript")));
    assert!(is_rewritable_content_type(Some("application/json")));
    assert!(!is_rewritable_content_type(Some("text/event-stream")));
    assert!(!is_rewritable_content_type(Some("image/png")));
    assert!(!is_rewritable_content_type(None));
}

fn bangumi_plan() -> RewritePlan {
    RewritePlan::new(vec![
        RewriteRule::new("api.bgm.tv", "api.bangumi.lol"),
        RewriteRule::new("bgm.tv", "bangumi.lol"),
    ])
}

#[test]
fn rewrite_plan_rewrites_exact_and_unlisted_subdomains() {
    let rewritten =
        bangumi_plan().rewrite("https://api.bgm.tv/v0 https://foo.bgm.tv/path BGM.TV:443");

    assert_eq!(
        rewritten,
        "https://api.bangumi.lol/v0 https://foo.bangumi.lol/path bangumi.lol:443"
    );
}

#[test]
fn rewrite_plan_respects_dns_label_boundaries() {
    let rewritten =
        bangumi_plan().rewrite("fxbgm.tv evil-bgm.tv bgm.tv.cn x_bgm.tv https://bgm.tv/path");

    assert_eq!(
        rewritten,
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

#[test]
fn whitespace_only_cookie_domain_is_preserved_without_panicking() {
    let cookie = "id=1; Domain=   ; Path=/";

    assert_eq!(
        rewrite_cookie_domain(cookie, "bgm.tv", "bangumi.lol"),
        cookie
    );
}
