const VALID_REFERRER_POLICIES: &[&str] = &[
    "no-referrer",
    "no-referrer-when-downgrade",
    "origin",
    "origin-when-cross-origin",
    "same-origin",
    "strict-origin",
    "strict-origin-when-cross-origin",
    "unsafe-url",
];

pub(crate) fn normalize_referrer_policy(raw: &str) -> Option<String> {
    raw.split(',')
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            VALID_REFERRER_POLICIES
                .contains(&token.as_str())
                .then_some(token)
        })
        .next_back()
}

pub(crate) fn response_referrer_policy_from_headers(
    headers: &[(String, Vec<u8>)],
) -> Option<String> {
    let mut combined = String::new();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
    {
        if !combined.is_empty() {
            combined.push_str(", ");
        }
        combined.push_str(&moli_fetch::decode_header_value(value));
    }
    (!combined.is_empty())
        .then(|| normalize_referrer_policy(&combined))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_valid_referrer_policy_token_wins() {
        assert_eq!(
            normalize_referrer_policy("not-yet-standardized, no-referrer"),
            Some("no-referrer".to_owned())
        );
        assert_eq!(
            normalize_referrer_policy("origin, not-yet-standardized, strict-origin"),
            Some("strict-origin".to_owned())
        );
    }

    #[test]
    fn invalid_trailing_tokens_do_not_clear_previous_valid_policy() {
        assert_eq!(
            normalize_referrer_policy("same-origin, not-yet-standardized"),
            Some("same-origin".to_owned())
        );
    }

    #[test]
    fn response_referrer_policy_combines_header_instances_before_normalizing() {
        let headers: Vec<(String, Vec<u8>)> = vec![
            ("Referrer-Policy".to_owned(), b"no-referrer".to_vec()),
            ("referrer-policy".to_owned(), b"future-policy".to_vec()),
        ];

        assert_eq!(
            response_referrer_policy_from_headers(&headers),
            Some("no-referrer".to_owned())
        );
    }
}
