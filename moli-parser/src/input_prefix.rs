pub(crate) fn split_parser_input_prefix(
    input: String,
    max_bytes: usize,
) -> (String, Option<String>) {
    if max_bytes == 0 || input.len() <= max_bytes {
        return (input, None);
    }

    let mut split = input.len();
    for (index, character) in input.char_indices() {
        let end = index + character.len_utf8();
        if end > max_bytes {
            split = if index == 0 { end } else { index };
            break;
        }
    }
    let remainder = input[split..].to_owned();
    let prefix = input[..split].to_owned();
    (prefix, (!remainder.is_empty()).then_some(remainder))
}
