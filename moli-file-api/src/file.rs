pub fn normalize_file_last_modified(last_modified: f64, fallback: f64) -> f64 {
    if last_modified.is_finite() {
        last_modified.trunc()
    } else {
        fallback.trunc()
    }
}

/// File-backed reads distinguish a disappeared path from unreadable or changed
/// contents, as do FileReader and Blob byte-consuming methods.
pub fn read_error_name(error: &std::io::Error) -> &'static str {
    if error.kind() == std::io::ErrorKind::NotFound {
        "NotFoundError"
    } else {
        "NotReadableError"
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_file_last_modified;

    #[test]
    fn normalizes_file_last_modified_like_file_metadata() {
        assert_eq!(normalize_file_last_modified(1234.9, 99.0), 1234.0);
        assert_eq!(normalize_file_last_modified(-1234.9, 99.0), -1234.0);
        assert_eq!(normalize_file_last_modified(f64::NAN, 99.9), 99.0);
        assert_eq!(normalize_file_last_modified(f64::INFINITY, 99.9), 99.0);
    }
}
