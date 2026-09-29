/// Locate a complete author-supplied TeX span before ordinary Markdown escaping.
/// Dollar boundaries follow the usual non-space/non-digit rules, so prices do
/// not consume the prose between two currency amounts.
pub(crate) fn next_span(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut dollar = None;
    let mut double_dollar = None;
    let mut paren = None;
    let mut bracket = None;
    let mut candidate = None;
    while index < bytes.len() {
        if bytes[index] == b'$' && !escaped(bytes, index) {
            if bytes.get(index + 1) == Some(&b'$') {
                if let Some(start) = double_dollar.take() {
                    accept_candidate(text, start, index + 2, 2, false, &mut candidate);
                } else {
                    double_dollar = Some(index);
                }
                index += 2;
                if let Some(ready) =
                    ready_candidate(candidate, [dollar, double_dollar, paren, bracket])
                {
                    return Some(ready);
                }
                continue;
            }
            if let Some(start) = dollar.take() {
                accept_candidate(text, start, index + 1, 1, true, &mut candidate);
            } else if text[index + 1..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace())
            {
                dollar = Some(index);
            }
            index += 1;
            if let Some(ready) = ready_candidate(candidate, [dollar, double_dollar, paren, bracket])
            {
                return Some(ready);
            }
            continue;
        }
        if bytes[index] == b'\\' && !escaped(bytes, index) {
            match bytes.get(index + 1) {
                Some(b'(') => {
                    paren.get_or_insert(index);
                }
                Some(b'[') => {
                    bracket.get_or_insert(index);
                }
                Some(b')') => {
                    if let Some(start) = paren.take() {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidate);
                    }
                }
                Some(b']') => {
                    if let Some(start) = bracket.take() {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidate);
                    }
                }
                _ => {}
            }
            if matches!(bytes.get(index + 1), Some(b'(' | b'[' | b')' | b']')) {
                index += 2;
                if let Some(ready) =
                    ready_candidate(candidate, [dollar, double_dollar, paren, bracket])
                {
                    return Some(ready);
                }
                continue;
            }
        }
        index += 1;
    }
    candidate
}

fn ready_candidate(
    candidate: Option<(usize, usize)>,
    openers: [Option<usize>; 4],
) -> Option<(usize, usize)> {
    let candidate = candidate?;
    openers
        .into_iter()
        .flatten()
        .all(|start| start >= candidate.0)
        .then_some(candidate)
}

fn accept_candidate(
    text: &str,
    start: usize,
    end: usize,
    delimiter: usize,
    single_dollar: bool,
    candidate: &mut Option<(usize, usize)>,
) {
    let content_start = start + delimiter;
    let content_end = end - delimiter;
    if content_start == content_end
        || single_dollar
            && (text[..content_end]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
                || text.as_bytes().get(end).is_some_and(u8::is_ascii_digit))
    {
        return;
    }
    let content = &text[content_start..content_end];
    if content.as_bytes().windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
    }) || contains_markdown_resource(content)
    {
        return;
    }
    if candidate.is_none_or(|(current, _)| start < current) {
        *candidate = Some((start, end));
    }
}

fn contains_markdown_resource(text: &str) -> bool {
    text.contains("](") || text.contains("][") || text.contains("![[")
}

fn escaped(bytes: &[u8], index: usize) -> bool {
    bytes[..index]
        .iter()
        .rev()
        .take_while(|&&ch| ch == b'\\')
        .count()
        % 2
        == 1
}
