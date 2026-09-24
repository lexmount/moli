/// Locate a complete author-supplied TeX span before ordinary Markdown escaping.
/// Dollar boundaries follow the usual non-space/non-digit rules, so prices do
/// not consume the prose between two currency amounts.
pub(crate) fn next_span(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let (open, close, single_dollar) = if bytes[index] == b'$' && !escaped(bytes, index) {
            if bytes.get(index + 1) == Some(&b'$') {
                (2, "$$", false)
            } else {
                (1, "$", true)
            }
        } else if bytes[index] == b'\\' && !escaped(bytes, index) {
            match bytes.get(index + 1) {
                Some(b'(') => (2, "\\)", false),
                Some(b'[') => (2, "\\]", false),
                _ => {
                    index += 1;
                    continue;
                }
            }
        } else {
            index += 1;
            continue;
        };
        let content_start = index + open;
        if single_dollar
            && text[content_start..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        {
            index = content_start;
            continue;
        }
        let mut search = content_start;
        while let Some(relative) = text[search..].find(close) {
            let end = search + relative;
            search = end + close.len();
            if escaped(bytes, end) {
                continue;
            }
            if end == content_start {
                break;
            }
            if single_dollar
                && (text[..end]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
                    || bytes.get(end + 1).is_some_and(u8::is_ascii_digit))
            {
                break;
            }
            // A plain Markdown reader must never reinterpret source text as
            // executable HTML merely because it was surrounded by dollars.
            if bytes[content_start..end].windows(2).any(|pair| {
                pair[0] == b'<'
                    && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
            }) {
                break;
            }
            return Some((index, search));
        }
        index = search.max(content_start);
    }
    None
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
