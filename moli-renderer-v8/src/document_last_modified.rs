use std::time::UNIX_EPOCH;

pub(crate) fn document_last_modified_from_headers(headers: &[(String, Vec<u8>)]) -> Option<f64> {
    let headers = moli_fetch::headers_to_byte_strings(headers);
    let value = headers
        .iter()
        .rev()
        .find(|(name, _)| name.eq_ignore_ascii_case("last-modified"))?
        .1
        .trim();
    let timestamp = httpdate::parse_http_date(value).ok()?;
    Some(match timestamp.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64() * 1000.0,
        Err(error) => -(error.duration().as_secs_f64() * 1000.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_modified_header_is_case_insensitive_and_requires_an_http_date() {
        assert_eq!(
            document_last_modified_from_headers(&[(
                "LAST-MODIFIED".to_owned(),
                b"Thu, 01 Jan 1970 01:23:45 GMT".to_vec(),
            )]),
            Some(5_025_000.0)
        );
        assert_eq!(
            document_last_modified_from_headers(&[(
                "Last-Modified".to_owned(),
                b"not a date".to_vec(),
            )]),
            None
        );
        assert_eq!(document_last_modified_from_headers(&[]), None);
    }
}
