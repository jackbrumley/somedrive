/// Microsoft's early warning that most of a quota is used: the
/// `RateLimit-Limit`, `RateLimit-Remaining` and `RateLimit-Reset` headers
/// (any of them present means the warning applies).
#[derive(Debug, PartialEq)]
pub(crate) struct RateLimitWarning {
    pub(crate) limit: Option<u64>,
    pub(crate) remaining: Option<u64>,
    pub(crate) reset_seconds: Option<u64>,
}

pub(crate) fn parse_rate_limit_warning(
    headers: &reqwest::header::HeaderMap,
) -> Option<RateLimitWarning> {
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.trim().parse::<u64>().ok())
    };
    let present = ["RateLimit-Limit", "RateLimit-Remaining", "RateLimit-Reset"]
        .iter()
        .any(|name| headers.contains_key(*name));
    present.then(|| RateLimitWarning {
        limit: number("RateLimit-Limit"),
        remaining: number("RateLimit-Remaining"),
        reset_seconds: number("RateLimit-Reset"),
    })
}

#[cfg(test)]
mod rate_limit_warning_tests {
    use super::*;

    #[test]
    fn the_warning_is_read_from_any_rate_limit_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(parse_rate_limit_warning(&headers), None);
        headers.insert("ratelimit-remaining", "120".parse().expect("value"));
        headers.insert("RateLimit-Reset", " 35 ".parse().expect("value"));
        assert_eq!(
            parse_rate_limit_warning(&headers),
            Some(RateLimitWarning {
                limit: None,
                remaining: Some(120),
                reset_seconds: Some(35),
            })
        );
    }
}
