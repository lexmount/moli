pub(crate) fn address_target(input: &str) -> Option<url::Url> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(input)
        && matches!(url.scheme(), "http" | "https" | "about" | "data" | "file")
    {
        return Some(url);
    }
    if !input.contains(char::is_whitespace) {
        let host = input.split('/').next().unwrap_or(input);
        let local = host == "localhost"
            || host.starts_with("localhost:")
            || host.starts_with("127.")
            || host.starts_with("[::1]");
        if (local || host.contains('.'))
            && let Ok(url) = url::Url::parse(&format!(
                "{}://{input}",
                if local { "http" } else { "https" }
            ))
        {
            return Some(url);
        }
    }
    let mut search = url::Url::parse("https://duckduckgo.com/").expect("search URL");
    search.query_pairs_mut().append_pair("q", input);
    Some(search)
}
