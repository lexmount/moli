use url::Url;

/// The client's policy, independent of the request's referrer or target origin.
/// Plain HTTP clients and navigations retain the default unrestricted policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestMixedContentPolicy {
    #[default]
    Allow,
    Block,
}

impl RequestMixedContentPolicy {
    pub fn check(self, target: &Url) -> Result<(), String> {
        if self == Self::Block
            && target.scheme() == "http"
            && !moli_url::is_potentially_trustworthy_url(target)
        {
            Err(format!("Mixed content: insecure request to `{target}`"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_content_target_exemptions_use_the_url_host_not_dns() {
        for text in [
            "https://remote.test/",
            "http://localhost/",
            "http://sub.localhost/",
            "http://127.0.0.2/",
            "http://[::1]/",
            "data:text/plain,hello",
            "blob:https://client.test/id",
        ] {
            assert!(
                RequestMixedContentPolicy::Block
                    .check(&Url::parse(text).unwrap())
                    .is_ok(),
                "{text}"
            );
        }
        let remote = Url::parse("http://web-platform.test/").unwrap();
        assert!(RequestMixedContentPolicy::Block.check(&remote).is_err());
        assert!(RequestMixedContentPolicy::Allow.check(&remote).is_ok());
    }
}
