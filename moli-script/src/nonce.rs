/// Return whether a script element's nonce is safe to use for CSP matching.
///
/// CSP rejects nonces on parser elements with duplicate attributes or with an
/// attribute name/value containing `<script`, `<style`, or `<link`, preventing
/// dangling markup from borrowing a trusted nonce.
pub fn script_element_nonce_is_nonceable<'a>(
    nonce: Option<&str>,
    had_duplicate_attributes: bool,
    attributes: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> bool {
    nonce.is_some()
        && !had_duplicate_attributes
        && attributes.into_iter().all(|(name, value)| {
            !contains_nonce_breaking_markup(name) && !contains_nonce_breaking_markup(value)
        })
}

fn contains_nonce_breaking_markup(value: &str) -> bool {
    [
        b"<script".as_slice(),
        b"<style".as_slice(),
        b"<link".as_slice(),
    ]
    .into_iter()
    .any(|needle| {
        value
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
    })
}

#[cfg(test)]
mod tests {
    use super::script_element_nonce_is_nonceable;

    #[test]
    fn script_nonce_rejects_duplicate_or_markup_shaped_attributes() {
        assert!(script_element_nonce_is_nonceable(
            Some("abc"),
            false,
            [("nonce", "abc"), ("data-value", "safe")],
        ));
        assert!(!script_element_nonce_is_nonceable(
            None,
            false,
            [("data-value", "safe")],
        ));
        assert!(!script_element_nonce_is_nonceable(
            Some("abc"),
            true,
            [("nonce", "abc")],
        ));

        for attributes in [
            [("attribute<script", "safe"), ("nonce", "abc")],
            [("attribute<style", "safe"), ("nonce", "abc")],
            [("attribute", "value<ScRiPt"), ("nonce", "abc")],
            [("attribute", "value<StYlE"), ("nonce", "abc")],
            [("attribute", "value<LiNk"), ("nonce", "abc")],
        ] {
            assert!(!script_element_nonce_is_nonceable(
                Some("abc"),
                false,
                attributes,
            ));
        }
    }
}
