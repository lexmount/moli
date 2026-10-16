pub(crate) struct ParsedCue {
    pub(crate) id: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
    pub(crate) text: String,
    pub(crate) line: ParsedCueLine,
    pub(crate) position: Option<f64>,
    pub(crate) size: Option<f64>,
    pub(crate) align: Option<&'static str>,
    pub(crate) vertical: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ParsedCueLine {
    Auto,
    Number(f64),
    Percent(f64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebVttParseError {
    InvalidSignature,
}

/// Validate the UTF-8-decoded signature, consuming at most one leading BOM.
pub(crate) fn validate_vtt_signature(text: &str) -> Result<(), WebVttParseError> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .strip_prefix("WEBVTT")
        .filter(|suffix| suffix.is_empty() || suffix.starts_with([' ', '\t', '\r', '\n']))
        .map(|_| ())
        .ok_or(WebVttParseError::InvalidSignature)
}

/// Parse cue blocks independently from DOM wrappers and event tasks. Invalid
/// cue blocks recover at the next arrow or empty line without failing the file.
pub(crate) fn parse_vtt_cues(text: &str) -> Result<Vec<ParsedCue>, WebVttParseError> {
    validate_vtt_signature(text)?;
    let normalized = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace('\0', "\u{fffd}")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut index = 1;
    // Header metadata is not a cue, but an arrow must be reconsumed as the
    // beginning of the first cue even when the blank separator is missing.
    if lines.get(index).is_some_and(|line| !line.is_empty()) {
        let _ = collect_vtt_cue_block(&lines, &mut index, true);
    }
    let mut cues = Vec::new();
    while index < lines.len() {
        while lines.get(index).is_some_and(|line| line.is_empty()) {
            index += 1;
        }
        if let Some(cue) = collect_vtt_cue_block(&lines, &mut index, false) {
            cues.push(cue);
        }
    }
    Ok(cues)
}

fn collect_vtt_cue_block(lines: &[&str], index: &mut usize, in_header: bool) -> Option<ParsedCue> {
    let mut line_count = 0;
    let mut seen_arrow = false;
    let mut buffer = Vec::new();
    let mut cue = None;
    while let Some(line) = lines.get(*index) {
        let line_position = *index;
        *index += 1;
        line_count += 1;
        if line.contains("-->") {
            if in_header || !(line_count == 1 || line_count == 2 && !seen_arrow) {
                *index = line_position;
                break;
            }
            seen_arrow = true;
            cue = parse_vtt_timing_line(line).map(|(start, end, settings)| {
                let settings = parse_vtt_cue_settings(settings);
                ParsedCue {
                    id: buffer.join("\n"),
                    start,
                    end,
                    text: String::new(),
                    line: settings.line,
                    position: settings.position,
                    size: settings.size,
                    align: settings.align,
                    vertical: settings.vertical,
                }
            });
            if cue.is_some() {
                buffer.clear();
            }
        } else if line.is_empty() {
            break;
        } else {
            buffer.push(*line);
        }
    }
    if let Some(cue) = &mut cue {
        cue.text = buffer.join("\n");
    }
    cue
}

fn is_vtt_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{0c}')
}

fn parse_vtt_timing_line(line: &str) -> Option<(f64, f64, &str)> {
    let mut input = line.trim_start_matches(is_vtt_whitespace);
    let start = collect_vtt_timestamp(&mut input)?;
    input = input
        .trim_start_matches(is_vtt_whitespace)
        .strip_prefix("-->")?;
    input = input.trim_start_matches(is_vtt_whitespace);
    let end = collect_vtt_timestamp(&mut input)?;
    Some((start, end, input))
}

fn collect_vtt_digits<'a>(input: &mut &'a str) -> &'a str {
    let length = input.bytes().take_while(u8::is_ascii_digit).count();
    let (digits, remaining) = input.split_at(length);
    *input = remaining;
    digits
}

fn collect_vtt_timestamp(input: &mut &str) -> Option<f64> {
    let original = *input;
    let first = collect_vtt_digits(input);
    if first.is_empty() {
        return None;
    }
    *input = input.strip_prefix(':')?;
    if collect_vtt_digits(input).len() != 2 {
        return None;
    }
    if first.len() != 2 || parse_vtt_digits(first)? > 59 || input.starts_with(':') {
        *input = input.strip_prefix(':')?;
        if collect_vtt_digits(input).len() != 2 {
            return None;
        }
    }
    *input = input.strip_prefix('.')?;
    if collect_vtt_digits(input).len() != 3 {
        return None;
    }
    parse_vtt_time(&original[..original.len() - input.len()])
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ParsedCueSettings {
    line: ParsedCueLine,
    position: Option<f64>,
    size: Option<f64>,
    align: Option<&'static str>,
    vertical: Option<&'static str>,
}

impl Default for ParsedCueSettings {
    fn default() -> Self {
        Self {
            line: ParsedCueLine::Auto,
            position: None,
            size: None,
            align: None,
            vertical: None,
        }
    }
}

fn parse_vtt_cue_settings(settings: &str) -> ParsedCueSettings {
    let mut parsed = ParsedCueSettings::default();
    for token in settings
        .split(is_vtt_whitespace)
        .filter(|token| !token.is_empty())
    {
        let Some((name, value)) = token.split_once(':') else {
            continue;
        };
        match name {
            "line" => {
                if let Some(value) = parse_vtt_line_setting(value) {
                    parsed.line = value;
                }
            }
            "position" => {
                if let Some(value) = parse_vtt_percentage_setting(value) {
                    parsed.position = Some(value);
                }
            }
            "size" => {
                if let Some(value) = parse_vtt_percentage_setting(value) {
                    parsed.size = Some(value);
                }
            }
            "align" => match value {
                "start" => parsed.align = Some("start"),
                "center" => parsed.align = Some("center"),
                "end" => parsed.align = Some("end"),
                "left" => parsed.align = Some("left"),
                "right" => parsed.align = Some("right"),
                _ => {}
            },
            "vertical" => match value {
                "rl" => parsed.vertical = Some("rl"),
                "lr" => parsed.vertical = Some("lr"),
                _ => {}
            },
            _ => {}
        }
    }
    parsed
}

fn parse_vtt_line_setting(value: &str) -> Option<ParsedCueLine> {
    if let Some(percent) = value.strip_suffix('%') {
        return parse_vtt_percentage_number(percent).map(ParsedCueLine::Percent);
    }
    parse_vtt_decimal(value).map(ParsedCueLine::Number)
}

fn parse_vtt_percentage_setting(value: &str) -> Option<f64> {
    parse_vtt_percentage_number(value.strip_suffix('%')?)
}

fn parse_vtt_percentage_number(value: &str) -> Option<f64> {
    if value.starts_with('-') {
        return None;
    }
    let number = parse_vtt_decimal(value)?;
    (0.0..=100.0).contains(&number).then_some(number)
}

fn parse_vtt_decimal(value: &str) -> Option<f64> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let valid_digits =
        |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let valid = match digits.split_once('.') {
        Some((whole, fraction)) => valid_digits(whole) && valid_digits(fraction),
        None => valid_digits(digits),
    };
    if !valid {
        return None;
    }
    let number = value.parse::<f64>().ok()?;
    // HTML floating-point parsing chooses from finite doubles excluding -0.
    number
        .is_finite()
        .then_some(if number == 0.0 { 0.0 } else { number })
}

fn parse_vtt_time(value: &str) -> Option<f64> {
    let parts = value.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = parse_vtt_two_digit_range(minutes, 59)?;
            let seconds = parse_vtt_seconds_millis(seconds)?;
            Some(minutes as f64 * 60.0 + seconds)
        }
        [hours, minutes, seconds] => {
            let hours = parse_vtt_digits(hours)?;
            let minutes = parse_vtt_two_digit_range(minutes, 59)?;
            let seconds = parse_vtt_seconds_millis(seconds)?;
            Some(hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds)
        }
        _ => None,
    }
}

fn parse_vtt_digits(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_vtt_two_digit_range(value: &str, max: u64) -> Option<u64> {
    if value.len() != 2 {
        return None;
    }
    let value = parse_vtt_digits(value)?;
    (value <= max).then_some(value)
}

fn parse_vtt_seconds_millis(value: &str) -> Option<f64> {
    let (seconds, milliseconds) = value.split_once('.')?;
    let seconds = parse_vtt_two_digit_range(seconds, 59)?;
    if milliseconds.len() != 3 || !milliseconds.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let milliseconds = milliseconds.parse::<u16>().ok()?;
    Some(seconds as f64 + f64::from(milliseconds) / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::{ParsedCueLine, parse_vtt_cues, parse_vtt_time};

    #[test]
    fn parse_vtt_time_rejects_malformed_timestamps() {
        assert_eq!(parse_vtt_time("00:30.500"), Some(30.5));
        assert_eq!(parse_vtt_time("00:01:00.500"), Some(60.5));
        assert_eq!(parse_vtt_time("100:20:00.500"), Some(361200.5));
        assert_eq!(parse_vtt_time("00:00.000"), Some(0.0));

        assert_eq!(parse_vtt_time("00.00.000"), None);
        assert_eq!(parse_vtt_time("01:00:500"), None);
        assert_eq!(parse_vtt_time("120:00.500"), None);
        assert_eq!(parse_vtt_time("00:120:00.500"), None);
        assert_eq!(parse_vtt_time("03m:00.500"), None);
    }

    #[test]
    fn parse_vtt_cues_keeps_non_positive_ranges_and_skips_invalid_timestamps() {
        let cues = parse_vtt_cues(
            "\u{feff}WEBVTT\n\n\
             valid\n\
             00:00.000 --> 00:01.500\n\
             text\n\n\
             invalid\n\
             00:31.000 --> 01:00:500\n\
             ignored\n\n\
             negative duration\n\
             00:03.000 --> 00:01.000\n\
             ignored\n\n\
             zero\n\
             00:02.000 --> 00:02.000\n\
             ignored\n",
        )
        .unwrap();

        assert_eq!(cues.len(), 3);
        assert_eq!((cues[1].start, cues[1].end), (3.0, 1.0));
        assert_eq!((cues[2].start, cues[2].end), (2.0, 2.0));
        assert_eq!(cues[0].id, "valid");
        assert_eq!(cues[0].start, 0.0);
        assert_eq!(cues[0].end, 1.5);
        assert_eq!(cues[0].text, "text");
    }

    #[test]
    fn parse_vtt_cue_settings_from_timing_line() {
        let cues = parse_vtt_cues(
            "WEBVTT\n\n\
             00:00.000 --> 00:01.000 line:15% position:40% size:10% align:middle vertical:rl\n\
             settings\n\n\
             00:02.000 --> 00:03.000 line:-1 position:150% align: start size:\t50% vertical:bad\n\
             bad settings\n",
        )
        .unwrap();

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].line, ParsedCueLine::Percent(15.0));
        assert_eq!(cues[0].position, Some(40.0));
        assert_eq!(cues[0].size, Some(10.0));
        assert_eq!(
            cues[0].align, None,
            "obsolete middle does not override the default"
        );
        assert_eq!(cues[0].vertical, Some("rl"));
        assert_eq!(cues[1].line, ParsedCueLine::Number(-1.0));
        assert_eq!(cues[1].position, None);
        assert_eq!(cues[1].size, None);
        assert_eq!(cues[1].align, None);
        assert_eq!(cues[1].vertical, None);
    }

    #[test]
    fn parse_vtt_recovers_timing_lines_without_blank_separator() {
        let cues = parse_vtt_cues(
            "WEBVTT\n\
             00:00.000 --> 00:01.000\n\
             Valid cue 1\n\
             label kept as text\n\
             00:02.000 --> 00:03.000\n\
             Valid cue 2\n\n\
             --> bad id\n\
             00:04.000 --> 00:05.000\n\
             Recovered cue\n\n\
             00:06.000 --> 00:07.000\n\
             00:08.000 --> 00:09.000\n",
        )
        .unwrap();

        assert_eq!(cues.len(), 5);
        assert_eq!(cues[0].text, "Valid cue 1\nlabel kept as text");
        assert_eq!(cues[1].id, "");
        assert_eq!(cues[1].text, "Valid cue 2");
        assert_eq!(cues[2].id, "");
        assert_eq!(cues[2].text, "Recovered cue");
        assert_eq!(cues[3].start, 6.0);
        assert_eq!(cues[3].end, 7.0);
        assert_eq!(cues[3].text, "");
        assert_eq!(cues[4].start, 8.0);
        assert_eq!(cues[4].end, 9.0);
        assert_eq!(cues[4].text, "");
    }

    #[test]
    fn webvtt_signature_failure_is_distinct_from_an_empty_valid_file() {
        for input in [
            "",
            "webvtt\n\n",
            " WEBVTT\n\n",
            "WEBVTTx\n\n",
            "WEBVTT\u{0c}\n",
            "\u{feff}\u{feff}WEBVTT\n",
        ] {
            assert!(parse_vtt_cues(input).is_err(), "{input:?}");
        }
        for input in [
            "WEBVTT",
            "WEBVTT ",
            "WEBVTT\t",
            "\u{feff}WEBVTT\r\n\r\n",
            "WEBVTT comment\nmetadata",
        ] {
            assert!(parse_vtt_cues(input).unwrap().is_empty(), "{input:?}");
        }
    }

    #[test]
    fn webvtt_blocks_preserve_whitespace_ids_and_normalize_nulls() {
        let cues = parse_vtt_cues("WEBVTT\nmetadata\0\n00:00.000 --> 00:01.000\nhead\0text\n\n \n00:01.000 --> 00:02.000\n \n\n").unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "head\u{fffd}text");
        assert_eq!(cues[1].id, " ");
        assert_eq!(cues[1].text, " ");
    }

    #[test]
    fn webvtt_any_arrow_terminates_payload_and_recovers_the_next_cue() {
        let cues = parse_vtt_cues("WEBVTT\n\n00:00.000 --> 00:01.000\nfirst\ninvalid-->\n00:02.000 --> 00:03.000\nsecond\n").unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "first");
        assert_eq!(cues[1].text, "second");
        assert_eq!(cues[1].id, "");
    }

    #[test]
    fn webvtt_timestamp_collection_uses_html_whitespace_and_retains_settings() {
        let cues = parse_vtt_cues("WEBVTT\n\n\u{0c}00:00.000\u{0c}-->\u{0c}00:01.000\0 align:end\nvalid\n\n\u{0b}00:00.000 --> 00:01.000\ninvalid\n").unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].align, Some("end"));
        assert_eq!(cues[0].text, "valid");
    }

    #[test]
    fn webvtt_settings_accept_decimal_values_and_keep_the_last_valid_setting() {
        let cues = parse_vtt_cues("WEBVTT\n\n00:00.000 --> 00:01.000 size:1.5% size:101% line:4294967296 align:end align:middle\nlarge\n\n00:00.000 --> 00:01.000 size:50.5% position:0.25% line:-0.5\nfractional\n\n00:00.000 --> 00:01.000 line:-0\nzero\n").unwrap();
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].size, Some(1.5));
        assert_eq!(cues[0].line, ParsedCueLine::Number(4294967296.0));
        assert_eq!(cues[0].align, Some("end"));
        assert_eq!(cues[1].size, Some(50.5));
        assert_eq!(cues[1].position, Some(0.25));
        assert_eq!(cues[1].line, ParsedCueLine::Number(-0.5));
        let ParsedCueLine::Number(zero) = cues[2].line else {
            panic!("numeric line");
        };
        assert_eq!(zero.to_bits(), 0.0_f64.to_bits());
    }
}
