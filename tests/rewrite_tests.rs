use mirrox::rewrite::{
    is_rewritable_content_type, rewrite_cookie_domain, rewrite_header_value, rewrite_text_body,
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
