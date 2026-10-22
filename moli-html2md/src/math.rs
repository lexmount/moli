/// Locate a complete author-supplied TeX span before ordinary Markdown escaping.
/// Dollar boundaries follow the usual non-space/non-digit rules, so prices do
/// not consume the prose between two currency amounts.
pub(crate) fn spans(text: &str) -> Vec<(usize, usize, String)> {
    let mut steps = 0;
    spans_counted(text, &mut steps)
}

fn spans_counted(text: &str, steps: &mut usize) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut backslashes = 0;
    let mut dollar = None;
    let mut double_dollar = None;
    let mut paren = None;
    let mut bracket = None;
    let mut candidates = Vec::new();
    while index < bytes.len() {
        *steps += 1;
        if bytes[index] == b'$' && backslashes % 2 == 0 {
            if bytes.get(index + 1) == Some(&b'$') {
                if let Some(start) = double_dollar.take() {
                    accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                } else {
                    double_dollar = Some(index);
                }
                index += 2;
                backslashes = 0;
                continue;
            }
            if let Some(start) = dollar.take() {
                if !accept_candidate(text, start, index + 1, 1, true, &mut candidates)
                    && text[index + 1..]
                        .chars()
                        .next()
                        .is_some_and(|ch| !ch.is_whitespace())
                {
                    dollar = Some(index);
                }
            } else if text[index + 1..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace())
            {
                dollar = Some(index);
            }
            index += 1;
            backslashes = 0;
            continue;
        }
        if bytes[index] == b'\\' && backslashes % 2 == 0 {
            match bytes.get(index + 1) {
                Some(b'(') => {
                    paren.get_or_insert(index);
                }
                Some(b'[') => {
                    bracket.get_or_insert(index);
                }
                Some(b')') => {
                    if let Some(start) = paren.take() {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                    }
                }
                Some(b']') => {
                    if let Some(start) = bracket.take() {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                    }
                }
                _ => {}
            }
            if matches!(bytes.get(index + 1), Some(b'(' | b'[' | b')' | b']')) {
                index += 2;
                backslashes = 0;
                continue;
            }
        }
        if bytes[index] == b'\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
        index += 1;
    }
    candidates.sort_unstable_by_key(|candidate| candidate.0);
    let mut end = 0;
    candidates.retain(|candidate| {
        let keep = candidate.0 >= end;
        if keep {
            end = candidate.1;
        }
        keep
    });
    candidates
}

fn accept_candidate(
    text: &str,
    start: usize,
    end: usize,
    delimiter: usize,
    single_dollar: bool,
    candidates: &mut Vec<(usize, usize, String)>,
) -> bool {
    let content_start = start + delimiter;
    let content_end = end - delimiter;
    let followed_by_digit = text.as_bytes().get(end).is_some_and(u8::is_ascii_digit);
    if content_start == content_end
        || single_dollar
            && (text[..content_end]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
                || followed_by_digit)
    {
        return false;
    }
    let content = &text[content_start..content_end];
    if contains_markdown_resource(content) {
        return false;
    }
    let Some(normalized) = normalized_span(&text[start..end]) else {
        return false;
    };
    candidates.push((start, end, normalized));
    true
}

fn contains_markdown_resource(text: &str) -> bool {
    text.contains("](") || text.contains("][") || text.contains("![[")
}

/// Keep TeX on one Markdown line. Comments and ordinary whitespace normalize;
/// control spaces and verbatim payloads remain literal. HTML-like text inside
/// text/verbatim commands cannot be preserved safely and rejects the span.
/// Backslash delimiters become dollar delimiters understood by Markdown math.
fn normalized_span(text: &str) -> Option<String> {
    let delimiter = if text.starts_with("$$") || text.starts_with('\\') {
        2
    } else {
        1
    };
    let mut output = String::new();
    let mut chars = text[delimiter..text.len() - delimiter].chars().peekable();
    let mut escaped = false;
    let mut space = false;
    let mut text_depth = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\\' && !escaped {
            let command: String = chars
                .clone()
                .take_while(char::is_ascii_alphabetic)
                .collect();
            if command == "verb" {
                for _ in 0..command.len() {
                    chars.next();
                }
                flush_space(&mut output, &mut space);
                output.push_str(r"\verb");
                if chars.peek() == Some(&'*') {
                    output.push('*');
                    chars.next();
                }
                let delimiter = chars
                    .next()
                    .filter(|c| !c.is_whitespace() && !c.is_ascii_alphabetic() && *c != '<')?;
                output.push(delimiter);
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if matches!(c, '\n' | '\r') || html_start(c, chars.peek().copied()) {
                        return None;
                    }
                    output.push(c);
                    if c == delimiter {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return None;
                }
                continue;
            }
            const TEXT_COMMANDS: &[&str] = &[
                "text",
                "mbox",
                "textrm",
                "textbf",
                "textit",
                "texttt",
                "textsf",
                "textnormal",
                "hbox",
            ];
            if TEXT_COMMANDS.contains(&command.as_str()) {
                let mut probe = chars.clone();
                for _ in 0..command.len() {
                    probe.next();
                }
                while probe.peek().is_some_and(char::is_ascii_whitespace) {
                    probe.next();
                }
                if probe.next() == Some('{') {
                    flush_space(&mut output, &mut space);
                    output.push('\\');
                    output.push_str(&command);
                    output.push('{');
                    chars = probe;
                    text_depth += 1;
                    continue;
                }
            }
        }
        if ch == '%' && !escaped {
            for part in chars.by_ref() {
                if matches!(part, '\n' | '\r') {
                    break;
                }
            }
            space = true;
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            if escaped {
                output.push(' ');
            } else {
                space = true;
            }
            escaped = false;
            continue;
        }
        flush_space(&mut output, &mut space);
        if html_start(ch, chars.peek().copied()) {
            if text_depth > 0 {
                return None;
            }
            output.push_str("< ");
        } else {
            output.push(ch);
        }
        if !escaped && text_depth > 0 {
            if ch == '{' {
                text_depth += 1;
            } else if ch == '}' {
                text_depth -= 1;
            }
        }
        escaped = ch == '\\' && !escaped;
    }
    if output.is_empty() || text_depth > 0 {
        return None;
    }
    let inline = text.starts_with("\\(") || delimiter == 1;
    if inline && output.ends_with(char::is_whitespace) {
        // An empty TeX group preserves the preceding control space while
        // keeping the inline Markdown closer after a non-space character.
        output.push_str("{}");
    }
    let marker = if inline { "$" } else { "$$" };
    Some(format!("{marker}{output}{marker}"))
}

fn flush_space(output: &mut String, pending: &mut bool) {
    if *pending && !output.is_empty() {
        output.push(' ');
    }
    *pending = false;
}

fn html_start(ch: char, next: Option<char>) -> bool {
    ch == '<'
        && next.is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?'))
}

#[cfg(test)]
mod tests {
    use super::spans_counted;

    #[test]
    fn math_scan_work_is_linear_for_backslashes_and_unclosed_delimiters() {
        for text in ["\\\\".repeat(8_192), "\\(x ".repeat(4_096)] {
            let mut steps = 0;
            let _ = spans_counted(&text, &mut steps);
            assert!(
                steps <= text.len(),
                "{steps} steps for {} bytes",
                text.len()
            );
        }
    }

    #[test]
    fn mixed_unclosed_and_valid_math_is_scanned_once() {
        for size in [256, 512, 1024, 2048] {
            let text = r"\( $x_i$ ".repeat(size);
            let mut steps = 0;
            let spans = spans_counted(&text, &mut steps);
            assert_eq!(spans.len(), size);
            assert!(
                steps <= text.len(),
                "{steps} steps for {} bytes",
                text.len()
            );
        }
    }
}
