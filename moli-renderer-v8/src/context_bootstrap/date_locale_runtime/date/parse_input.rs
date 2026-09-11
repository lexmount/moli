//! Classify timezone *ownership*, not date validity. V8 still parses the original
//! input first. Only timezone-less input needs a second parse as UTC wall-clock
//! fields before applying the target's timezone/DST policy.

pub(super) fn local_date_parse_input_as_utc(input: &str) -> Option<String> {
    // DateParser's InputReader treats NUL as end-of-input. Any timezone or
    // comment after that point must not affect classification or consume the
    // annotation we append for a second native parse.
    let input = input.split_once('\0').map_or(input, |(prefix, _)| prefix);
    // Do not trim: whitespace can turn an ISO date-only form (UTC) into a
    // legacy date (local time). Fixed offsets below index bytes, never UTF-8 str.
    if let Some(date_end) = iso_date_prefix_len(input.as_bytes()) {
        let rest = input.as_bytes().get(date_end..)?;
        if rest.is_empty() {
            return None;
        }
        if matches!(rest.first(), Some(b'T' | b't')) {
            if rest
                .iter()
                .any(|byte| matches!(byte, b'Z' | b'z' | b'+' | b'-'))
            {
                return None;
            }
            return Some(format!("{input}Z"));
        }
    }
    let (explicit_timezone, open_comments) = legacy_timezone(input);
    if explicit_timezone {
        return None;
    }
    // V8's legacy scanner ignores parenthesized text, including an unclosed
    // trailing comment. Close that ignored suffix so the UTC annotation is
    // outside it; never interpret a timezone name found inside a comment.
    Some(format!("{input}{} UTC", ")".repeat(open_comments)))
}

fn iso_date_prefix_len(bytes: &[u8]) -> Option<usize> {
    let signed = matches!(bytes.first(), Some(b'+' | b'-'));
    let year_len = if signed { 7 } else { 4 };
    let year = bytes.get(usize::from(signed)..year_len)?;
    if !year.iter().all(u8::is_ascii_digit) || (bytes.first() == Some(&b'-') && year == b"000000") {
        return None;
    }
    let mut end = year_len;
    for _ in 0..2 {
        if bytes.get(end) != Some(&b'-') {
            break;
        }
        if !bytes.get(end + 1..end + 3)?.iter().all(u8::is_ascii_digit) {
            return None;
        }
        end += 3;
    }
    Some(end)
}

fn legacy_timezone(input: &str) -> (bool, usize) {
    // Match V8 DateParser's TIME_ZONE_NAME tokens (dateparser.cc), not
    // substrings. In particular Tue/Thu/Sat and PST's final T are not ISO
    // separators. Like V8, names only designate a zone after a date number.
    const ZONES: &[&str] = &[
        "ut", "utc", "gmt", "z", "est", "edt", "cst", "cdt", "mst", "mdt", "pst", "pdt",
    ];
    const MONTHS: &[&str] = &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let mut chars = input.char_indices().peekable();
    let mut comments = 0;
    let mut number_seen = false;
    let mut time_fields = 0;
    while let Some((start, ch)) = chars.next() {
        if ch == '(' {
            comments += 1;
        } else if comments > 0 {
            if ch == ')' {
                comments -= 1;
            }
        } else if ch.is_ascii_digit() {
            number_seen = true;
            let mut number = ch.to_digit(10).unwrap();
            while let Some((_, digit)) = chars.next_if(|(_, ch)| ch.is_ascii_digit()) {
                number = number
                    .saturating_mul(10)
                    .saturating_add(digit.to_digit(10).unwrap());
            }
            // Mirror DateParser's token consumption, not a full date parser:
            // day numbers and month names consume their following '-' before
            // the timezone branch sees it. A completed time does NOT consume
            // that sign (00:00:00-0100 is an offset). The original native parse
            // above still decides validity, ranges and calendar composition.
            if chars.next_if(|(_, ch)| *ch == ':').is_some() {
                time_fields += 1;
                if chars.next_if(|(_, ch)| *ch == ':').is_some() {
                    time_fields += 1;
                } else {
                    let _ = chars.next_if(|(_, ch)| *ch == '.');
                }
            } else if ((time_fields == 1 || time_fields == 2) && number <= 59)
                || (time_fields == 3 && number <= 999)
            {
                time_fields = 4;
                if chars.next_if(|(_, ch)| *ch == '.').is_some() {
                    while chars.next_if(|(_, ch)| ch.is_ascii_digit()).is_some() {}
                }
            } else {
                let _ = chars.next_if(|(_, ch)| *ch == '-');
            }
        } else if matches!(ch, '+' | '-') && time_fields > 0 {
            return (true, comments);
        } else if is_legacy_word_char(ch) {
            while chars.peek().is_some_and(|(_, ch)| is_legacy_word_char(*ch)) {
                chars.next();
            }
            let end = chars.peek().map_or(input.len(), |(index, _)| *index);
            // Both bounds come from char_indices, so Unicode input is safe.
            let word = &input[start..end];
            if number_seen && ZONES.iter().any(|zone| word.eq_ignore_ascii_case(zone)) {
                return (true, comments);
            }
            if word.as_bytes().get(..3).is_some_and(|prefix| {
                MONTHS
                    .iter()
                    .any(|month| prefix.eq_ignore_ascii_case(month.as_bytes()))
            }) {
                let _ = chars.next_if(|(_, ch)| *ch == '-');
            }
        }
    }
    (false, comments)
}

fn is_legacy_word_char(ch: char) -> bool {
    ch >= 'A' && !ch.is_whitespace() && ch != '\u{feff}'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_only_recognition_rejects_unicode_without_panicking() {
        for input in ["你好", "🙂", "+你好啊", "-🙂abc", "2024-你好", "ééé"] {
            assert_ne!(
                iso_date_prefix_len(input.as_bytes()),
                Some(input.len()),
                "{input:?}"
            );
            let _ = local_date_parse_input_as_utc(input);
        }
    }

    #[test]
    fn date_timezone_ownership_distinguishes_iso_legacy_and_comments() {
        for input in [
            "2024",
            "2024-01",
            "2024-01-01",
            "+002024-01-01",
            "2024-01-01T00:00:00Z",
            "2024-01-01t00:00:00-02:30",
            "Jan 1 2024 00:00:00 PST",
            "Jan 1 2024 00:00:00 eSt\u{a0}",
            "Jan 1 2024 00:00:00 +0530",
            "00:00:00-0100 Jan-01-2024",
            "00:00-01:00 01-01-2024",
            "00:00:00.000-0100 Jan-01-2024",
        ] {
            assert_eq!(local_date_parse_input_as_utc(input), None, "{input}");
        }
        for input in [
            "Tue Jan 02 2024 00:00:00",
            "Thu Jan 04 2024 00:00:00",
            "Jan 1 2024 (PST)",
            " 2024-01-01 ",
            "00:00:00 Jan-01-2024",
            "00:00:00 01-01-2024",
            "00:00 January-01-2024",
            "00:00:00.000 Jan-01-2024",
        ] {
            assert_eq!(
                local_date_parse_input_as_utc(input),
                Some(format!("{input} UTC"))
            );
        }
        assert_eq!(
            local_date_parse_input_as_utc("2024-01-01T00:00:00"),
            Some("2024-01-01T00:00:00Z".into())
        );
        assert_eq!(
            local_date_parse_input_as_utc("Jan 1 2024 (PST"),
            Some("Jan 1 2024 (PST) UTC".into())
        );
    }
}
