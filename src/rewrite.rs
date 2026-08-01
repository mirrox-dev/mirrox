use std::borrow::Cow;

/// A normalized host-to-host rewrite rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteRule {
    from: String,
    to: String,
}

impl RewriteRule {
    pub fn new(from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        Self {
            from: normalize_rewrite_host(from.as_ref()),
            to: normalize_rewrite_host(to.as_ref()),
        }
    }

    pub fn from(&self) -> &str {
        &self.from
    }

    pub fn to(&self) -> &str {
        &self.to
    }
}

/// An immutable, deterministic collection of host rewrite rules.
///
/// Rules are matched against the original input in one pass, so a replacement
/// cannot trigger another rule. Longer hosts take precedence over suffix
/// fallbacks such as `bgm.tv`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewritePlan {
    rules: Vec<RewriteRule>,
}

impl RewritePlan {
    pub fn new(mut rules: Vec<RewriteRule>) -> Self {
        rules.retain(|rule| !rule.from.is_empty() && rule.from != rule.to);
        rules.sort_by(|a, b| {
            b.from
                .len()
                .cmp(&a.from.len())
                .then_with(|| a.from.cmp(&b.from))
                .then_with(|| a.to.cmp(&b.to))
        });
        rules.dedup_by(|a, b| a.from == b.from);
        Self { rules }
    }

    pub fn rules(&self) -> &[RewriteRule] {
        &self.rules
    }

    pub fn rewrite<'a>(&self, input: &'a str) -> Cow<'a, str> {
        let bytes = input.as_bytes();
        let mut output = String::new();
        let mut copied_until = 0;
        let mut cursor = 0;
        let mut changed = false;

        while cursor < bytes.len() {
            let matched = self.rules.iter().find(|rule| {
                let end = cursor + rule.from.len();
                end <= bytes.len()
                    && bytes[cursor..end].eq_ignore_ascii_case(rule.from.as_bytes())
                    && is_left_host_boundary(bytes, cursor)
                    && is_right_host_boundary(bytes, end)
            });

            if let Some(rule) = matched {
                output.push_str(&input[copied_until..cursor]);
                output.push_str(&rule.to);
                cursor += rule.from.len();
                copied_until = cursor;
                changed = true;
            } else {
                // Host patterns are ASCII, but response bodies need not be. Step
                // over a complete UTF-8 scalar so subsequent string slicing is safe.
                cursor += input[cursor..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(1);
            }
        }

        if changed {
            output.push_str(&input[copied_until..]);
            Cow::Owned(output)
        } else {
            Cow::Borrowed(input)
        }
    }
}

fn normalize_rewrite_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn is_left_host_boundary(input: &[u8], start: usize) -> bool {
    start == 0 || !is_dns_label_byte(input[start - 1])
}

fn is_right_host_boundary(input: &[u8], end: usize) -> bool {
    end == input.len() || (!is_dns_label_byte(input[end]) && input[end] != b'.')
}

fn is_dns_label_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

pub fn rewrite_text_body(input: &str, from_host: &str, to_host: &str) -> String {
    RewritePlan::new(vec![RewriteRule::new(from_host, to_host)])
        .rewrite(input)
        .into_owned()
}

pub fn rewrite_header_value(input: &str, from_host: &str, to_host: &str) -> String {
    rewrite_text_body(input, from_host, to_host)
}

pub fn rewrite_cookie_domain(input: &str, from_domain: &str, to_domain: &str) -> String {
    let from_domain = normalize_rewrite_host(from_domain);
    let to_domain = normalize_rewrite_host(to_domain);
    let mut rewritten = String::with_capacity(input.len());

    for segment in input.split_inclusive(';') {
        rewritten.push_str(&rewrite_cookie_segment(segment, &from_domain, &to_domain));
    }

    rewritten
}

fn rewrite_cookie_segment(segment: &str, from_domain: &str, to_domain: &str) -> String {
    let (attribute, delimiter) = segment
        .strip_suffix(';')
        .map_or((segment, ""), |value| (value, ";"));
    let Some(equals_index) = attribute.find('=') else {
        return segment.to_string();
    };
    if !attribute[..equals_index]
        .trim()
        .eq_ignore_ascii_case("domain")
    {
        return segment.to_string();
    }

    let value_with_spacing = &attribute[equals_index + 1..];
    let trimmed_value = value_with_spacing.trim();
    if trimmed_value.is_empty() {
        return segment.to_string();
    }
    let leading_len = value_with_spacing.len() - value_with_spacing.trim_start().len();
    let value_end = leading_len + trimmed_value.len();
    let value = &value_with_spacing[leading_len..value_end];
    let (dot, host) = value
        .strip_prefix('.')
        .map_or(("", value), |host| (".", host));

    if !host.eq_ignore_ascii_case(from_domain) {
        return segment.to_string();
    }

    format!(
        "{}={}{}{}{}{}",
        &attribute[..equals_index],
        &value_with_spacing[..leading_len],
        dot,
        to_domain,
        &value_with_spacing[value_end..],
        delimiter
    )
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
