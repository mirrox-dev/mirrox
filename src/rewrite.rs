pub fn rewrite_text_body(input: &str, from_host: &str, to_host: &str) -> String {
    input
        .replace(
            &format!("https://{from_host}"),
            &format!("https://{to_host}"),
        )
        .replace(&format!("http://{from_host}"), &format!("http://{to_host}"))
        .replace(&format!("//{from_host}"), &format!("//{to_host}"))
        .replace(from_host, to_host)
}

pub fn rewrite_header_value(input: &str, from_host: &str, to_host: &str) -> String {
    rewrite_text_body(input, from_host, to_host)
}

pub fn rewrite_cookie_domain(input: &str, from_domain: &str, to_domain: &str) -> String {
    let dotted_from = format!("Domain=.{from_domain}");
    let dotted_to = format!("Domain=.{to_domain}");
    let plain_from = format!("Domain={from_domain}");
    let plain_to = format!("Domain={to_domain}");

    input
        .replace(&dotted_from, &dotted_to)
        .replace(&plain_from, &plain_to)
        .replace(&dotted_from.to_ascii_lowercase(), &dotted_to)
        .replace(&plain_from.to_ascii_lowercase(), &plain_to)
}

pub fn is_rewritable_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let content_type = content_type.to_ascii_lowercase();
    if content_type.starts_with("text/event-stream") {
        return false;
    }
    content_type.starts_with("text/html")
        || content_type.starts_with("text/css")
        || content_type.starts_with("text/javascript")
        || content_type.starts_with("application/javascript")
        || content_type.starts_with("application/x-javascript")
        || content_type.starts_with("application/json")
}

pub fn registrable_suffix(host: &str) -> &str {
    let mut parts = host.rsplitn(3, '.');
    let tld = parts.next();
    let domain = parts.next();
    match (domain, tld) {
        (Some(domain), Some(tld)) => {
            let start = host.len() - domain.len() - tld.len() - 1;
            &host[start..]
        }
        _ => host,
    }
}
