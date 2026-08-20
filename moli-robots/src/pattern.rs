//! Rule-path and user-agent matching primitives.

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Puts a rule pattern and a request target into the same comparison space.
///
/// RFC 9309 §2.2.2 compares percent-encoded octets, so a pattern written with
/// literal UTF-8 must be escaped before it can match a request target that the
/// URL parser already escaped. Existing escapes are normalized to uppercase hex
/// so `%2f` and `%2F` compare equal.
pub(crate) fn normalize_percent_encoding(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut normalized = String::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            normalized.push('%');
            normalized.push(bytes[index + 1].to_ascii_uppercase() as char);
            normalized.push(bytes[index + 2].to_ascii_uppercase() as char);
            index += 3;
            continue;
        }

        if byte.is_ascii() {
            normalized.push(byte as char);
        } else {
            push_percent_encoded(&mut normalized, byte);
        }
        index += 1;
    }

    normalized
}

fn push_percent_encoded(out: &mut String, byte: u8) {
    out.push('%');
    out.push(HEX_DIGITS[usize::from(byte >> 4)] as char);
    out.push(HEX_DIGITS[usize::from(byte & 0x0F)] as char);
}

/// Whether `pattern` matches `request_target`.
///
/// Patterns anchor at the start of the request target. `*` matches any run of
/// characters including none, and a trailing `$` anchors the pattern to the end
/// of the request target. Both are already normalized by
/// [`normalize_percent_encoding`], so this operates on ASCII.
pub(crate) fn pattern_matches(pattern: &str, request_target: &str) -> bool {
    let (body, anchored_to_end) = match pattern.strip_suffix('$') {
        Some(body) => (body, true),
        None => (pattern, false),
    };

    let mut segments = body.split('*');
    // `split` always yields at least one segment, and the first one is anchored
    // to the start of the request target.
    let leading = segments.next().unwrap_or_default();
    let Some(mut rest) = request_target.strip_prefix(leading) else {
        return false;
    };

    let trailing: Vec<&str> = segments.collect();
    if trailing.is_empty() {
        // The pattern held no `*`, so it either is the whole request target or
        // is a prefix of it.
        return !anchored_to_end || rest.is_empty();
    }

    let last_index = trailing.len() - 1;
    for (index, segment) in trailing.iter().enumerate() {
        if index == last_index && anchored_to_end {
            // A trailing `$` forces the final literal to sit flush against the
            // end of the request target.
            return rest.ends_with(segment);
        }
        // Leftmost matching is sufficient: every later segment is free to match
        // anywhere further along, so an earlier match never blocks a solution
        // that a later one would allow.
        match rest.find(segment) {
            Some(offset) => rest = &rest[offset + segment.len()..],
            None => return false,
        }
    }

    true
}

/// How well a `User-agent` value describes `user_agent`.
///
/// `None` means the group does not apply. `Some(0)` is the `*` group, which
/// RFC 9309 §2.2.1 uses only when no named group matches. A larger value is a
/// more specific named match.
pub(crate) fn agent_specificity(group_agent: &str, lowercase_user_agent: &str) -> Option<usize> {
    if group_agent == "*" {
        return Some(0);
    }
    if group_agent.is_empty() {
        return None;
    }
    lowercase_user_agent
        .contains(group_agent)
        .then_some(group_agent.len())
}
