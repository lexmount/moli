pub(crate) struct ParsedCue {
    pub(crate) region: Option<usize>,
    pub(crate) id: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
    pub(crate) text: String,
    pub(crate) line: ParsedCueLine,
    pub(crate) line_align: &'static str,
    pub(crate) position: Option<f64>,
    pub(crate) position_align: &'static str,
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

#[derive(Default)]
pub(crate) struct ParsedWebVtt {
    pub(crate) cues: Vec<ParsedCue>,
    pub(crate) regions: Vec<ParsedRegion>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ParsedRegion {
    pub(crate) id: String,
    pub(crate) width: f64,
    pub(crate) lines: u32,
    pub(crate) region_anchor: (f64, f64),
    pub(crate) viewport_anchor: (f64, f64),
    pub(crate) scroll: &'static str,
}

impl Default for ParsedRegion {
    fn default() -> Self {
        Self {
            id: String::new(),
            width: 100.0,
            lines: 3,
            region_anchor: (0.0, 100.0),
            viewport_anchor: (0.0, 100.0),
            scroll: "",
        }
    }
}

enum ParsedBlock {
    Cue(ParsedCue),
    Region(ParsedRegion),
}

struct WebVttParser<'a> {
    lines: Vec<&'a str>,
    index: usize,
    output: ParsedWebVtt,
}

/// Parse file metadata and cues together so region references retain identity.
pub(crate) fn parse_vtt_file(text: &str) -> Result<ParsedWebVtt, WebVttParseError> {
    validate_vtt_signature(text)?;
    let normalized = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace('\0', "\u{fffd}")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut parser = WebVttParser {
        lines: normalized.lines().collect(),
        index: 1,
        output: ParsedWebVtt::default(),
    };
    if parser
        .lines
        .get(parser.index)
        .is_some_and(|line| !line.is_empty())
    {
        let _ = parser.collect_block(true);
    }
    while parser.index < parser.lines.len() {
        while parser
            .lines
            .get(parser.index)
            .is_some_and(|line| line.is_empty())
        {
            parser.index += 1;
        }
        match parser.collect_block(false) {
            Some(ParsedBlock::Cue(cue)) => parser.output.cues.push(cue),
            Some(ParsedBlock::Region(region)) => parser.output.regions.push(region),
            None => {}
        }
    }
    Ok(parser.output)
}

impl WebVttParser<'_> {
    fn collect_block(&mut self, in_header: bool) -> Option<ParsedBlock> {
        let mut line_count = 0;
        let mut seen_arrow = false;
        let mut buffer = Vec::new();
        let mut cue = None;
        let mut region_block = false;
        while let Some(line) = self.lines.get(self.index) {
            let line_position = self.index;
            self.index += 1;
            line_count += 1;
            if line.contains("-->") {
                if in_header || !(line_count == 1 || line_count == 2 && !seen_arrow) {
                    self.index = line_position;
                    break;
                }
                seen_arrow = true;
                cue = parse_vtt_timing_line(line).map(|(start, end, settings)| {
                    let settings = parse_vtt_cue_settings(settings, &self.output.regions);
                    ParsedCue {
                        region: settings.region,
                        id: buffer.join("\n"),
                        start,
                        end,
                        text: String::new(),
                        line: settings.line,
                        line_align: settings.line_align,
                        position: settings.position,
                        position_align: settings.position_align,
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
                if !in_header
                    && line_count == 2
                    && self.output.cues.is_empty()
                    && buffer
                        .first()
                        .and_then(|line: &&str| line.strip_prefix("REGION"))
                        .is_some_and(|suffix| suffix.chars().all(is_vtt_whitespace))
                {
                    region_block = true;
                    buffer.clear();
                }
                buffer.push(*line);
            }
        }
        if let Some(mut cue) = cue {
            cue.text = buffer.join("\n");
            Some(ParsedBlock::Cue(cue))
        } else if region_block {
            Some(ParsedBlock::Region(parse_vtt_region_settings(
                &buffer.join("\n"),
            )))
        } else {
            None
        }
    }
}

fn parse_vtt_region_settings(settings: &str) -> ParsedRegion {
    let mut region = ParsedRegion::default();
    for token in settings
        .split(is_vtt_whitespace)
        .filter(|token| !token.is_empty())
    {
        let Some((name, value)) = token.split_once(':') else {
            continue;
        };
        if name.is_empty() || value.is_empty() {
            continue;
        }
        match name {
            "id" => region.id = value.to_owned(),
            "width" => {
                if let Some(width) = parse_vtt_percentage_setting(value) {
                    region.width = width;
                }
            }
            "lines" => {
                if value.bytes().all(|byte| byte.is_ascii_digit()) {
                    // VTTRegion exposes an unsigned long. Preserve its low
                    // 32 bits without floating-point rounding for long input.
                    region.lines = value.bytes().fold(0u32, |number, digit| {
                        number
                            .wrapping_mul(10)
                            .wrapping_add(u32::from(digit - b'0'))
                    });
                }
            }
            "regionanchor" => {
                if let Some(anchor) = parse_vtt_anchor(value) {
                    region.region_anchor = anchor;
                }
            }
            "viewportanchor" => {
                if let Some(anchor) = parse_vtt_anchor(value) {
                    region.viewport_anchor = anchor;
                }
            }
            "scroll" if value == "up" => region.scroll = "up",
            _ => {}
        }
    }
    region
}

fn parse_vtt_anchor(value: &str) -> Option<(f64, f64)> {
    let (x, y) = value.split_once(',')?;
    Some((
        parse_vtt_percentage_setting(x)?,
        parse_vtt_percentage_setting(y)?,
    ))
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
    region: Option<usize>,
    line: ParsedCueLine,
    line_align: &'static str,
    position: Option<f64>,
    position_align: &'static str,
    size: Option<f64>,
    align: Option<&'static str>,
    vertical: Option<&'static str>,
}

impl Default for ParsedCueSettings {
    fn default() -> Self {
        Self {
            region: None,
            line: ParsedCueLine::Auto,
            line_align: "start",
            position: None,
            position_align: "auto",
            size: None,
            align: None,
            vertical: None,
        }
    }
}

fn parse_vtt_cue_settings(settings: &str, regions: &[ParsedRegion]) -> ParsedCueSettings {
    let mut parsed = ParsedCueSettings::default();
    for token in settings
        .split(is_vtt_whitespace)
        .filter(|token| !token.is_empty())
    {
        let Some((name, value)) = token.split_once(':') else {
            continue;
        };
        if name.is_empty() || value.is_empty() {
            continue;
        }
        match name {
            "region" => parsed.region = regions.iter().rposition(|region| region.id == value),
            "line" => {
                if let Some((position, alignment)) =
                    split_vtt_setting_alignment(value, &["start", "center", "end"])
                    && let Some(line) = parse_vtt_line_setting(position)
                {
                    parsed.line = line;
                    parsed.region = None;
                    if let Some(alignment) = alignment {
                        parsed.line_align = alignment;
                    }
                }
            }
            "position" => {
                if let Some((position, alignment)) =
                    split_vtt_setting_alignment(value, &["line-left", "center", "line-right"])
                    && let Some(position) = parse_vtt_percentage_setting(position)
                {
                    parsed.position = Some(position);
                    if let Some(alignment) = alignment {
                        parsed.position_align = alignment;
                    }
                }
            }
            "size" => {
                if let Some(value) = parse_vtt_percentage_setting(value) {
                    parsed.size = Some(value);
                    if value != 100.0 {
                        parsed.region = None;
                    }
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
            "vertical" => {
                match value {
                    "rl" => parsed.vertical = Some("rl"),
                    "lr" => parsed.vertical = Some("lr"),
                    _ => {}
                }
                if parsed.vertical.is_some() {
                    parsed.region = None;
                }
            }
            _ => {}
        }
    }
    parsed
}

fn split_vtt_setting_alignment<'a>(
    value: &'a str,
    allowed: &[&'static str],
) -> Option<(&'a str, Option<&'static str>)> {
    let Some((position, alignment)) = value.split_once(',') else {
        return Some((value, None));
    };
    let alignment = allowed.iter().copied().find(|token| *token == alignment)?;
    Some((position, Some(alignment)))
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
    use super::{ParsedCueLine, parse_vtt_file, parse_vtt_time};

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
        let cues = parse_vtt_file(
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
        .unwrap()
        .cues;

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
        let cues = parse_vtt_file(
            "WEBVTT\n\n\
             00:00.000 --> 00:01.000 line:15% position:40% size:10% align:middle vertical:rl\n\
             settings\n\n\
             00:02.000 --> 00:03.000 line:-1 position:150% align: start size:\t50% vertical:bad\n\
             bad settings\n",
        )
        .unwrap()
        .cues;

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
        let cues = parse_vtt_file(
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
        .unwrap()
        .cues;

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
            assert!(parse_vtt_file(input).is_err(), "{input:?}");
        }
        for input in [
            "WEBVTT",
            "WEBVTT ",
            "WEBVTT\t",
            "\u{feff}WEBVTT\r\n\r\n",
            "WEBVTT comment\nmetadata",
        ] {
            assert!(parse_vtt_file(input).unwrap().cues.is_empty(), "{input:?}");
        }
    }

    #[test]
    fn webvtt_blocks_preserve_whitespace_ids_and_normalize_nulls() {
        let cues = parse_vtt_file("WEBVTT\nmetadata\0\n00:00.000 --> 00:01.000\nhead\0text\n\n \n00:01.000 --> 00:02.000\n \n\n").unwrap().cues;
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "head\u{fffd}text");
        assert_eq!(cues[1].id, " ");
        assert_eq!(cues[1].text, " ");
    }

    #[test]
    fn webvtt_any_arrow_terminates_payload_and_recovers_the_next_cue() {
        let cues = parse_vtt_file("WEBVTT\n\n00:00.000 --> 00:01.000\nfirst\ninvalid-->\n00:02.000 --> 00:03.000\nsecond\n").unwrap().cues;
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "first");
        assert_eq!(cues[1].text, "second");
        assert_eq!(cues[1].id, "");
    }

    #[test]
    fn webvtt_timestamp_collection_uses_html_whitespace_and_retains_settings() {
        let cues = parse_vtt_file("WEBVTT\n\n\u{0c}00:00.000\u{0c}-->\u{0c}00:01.000\0 align:end\nvalid\n\n\u{0b}00:00.000 --> 00:01.000\ninvalid\n").unwrap().cues;
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].align, Some("end"));
        assert_eq!(cues[0].text, "valid");
    }

    #[test]
    fn webvtt_settings_accept_decimal_values_and_keep_the_last_valid_setting() {
        let cues = parse_vtt_file("WEBVTT\n\n00:00.000 --> 00:01.000 size:1.5% size:101% line:4294967296 align:end align:middle\nlarge\n\n00:00.000 --> 00:01.000 size:50.5% position:0.25% line:-0.5\nfractional\n\n00:00.000 --> 00:01.000 line:-0\nzero\n").unwrap().cues;
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
    #[test]
    fn webvtt_alignment_settings_validate_the_entire_pair_and_keep_prior_alignment() {
        let cues = parse_vtt_file("WEBVTT\n\n00:00.000 --> 00:01.000 line:10%,center position:40%,line-right line:30,invalid position:90%,invalid\nfirst\n\n00:01.000 --> 00:02.000 line:3,end line:7 position:30%,line-left position:25%\nsecond\n").unwrap().cues;
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].line, ParsedCueLine::Percent(10.0));
        assert_eq!(cues[0].line_align, "center");
        assert_eq!(cues[0].position, Some(40.0));
        assert_eq!(cues[0].position_align, "line-right");
        assert_eq!(cues[1].line, ParsedCueLine::Number(7.0));
        assert_eq!(cues[1].line_align, "end");
        assert_eq!(cues[1].position, Some(25.0));
        assert_eq!(cues[1].position_align, "line-left");
    }
    #[test]
    fn webvtt_region_defaults_and_settings_preserve_last_valid_values() {
        let file = parse_vtt_file("WEBVTT\n\nREGION\nid:a width:35.5% width:101% lines:7 lines:-1 regionanchor:12.5%,25% regionanchor:50%,bad viewportanchor:4%,8% scroll:up scroll:bad\n").unwrap();
        assert!(file.cues.is_empty());
        assert_eq!(file.regions.len(), 1);
        let region = &file.regions[0];
        assert_eq!(region.id, "a");
        assert_eq!(region.width, 35.5);
        assert_eq!(region.lines, 7);
        assert_eq!(region.region_anchor, (12.5, 25.0));
        assert_eq!(region.viewport_anchor, (4.0, 8.0));
        assert_eq!(region.scroll, "up");
        let defaults = parse_vtt_file("WEBVTT\n\nREGION\nid:defaults\n").unwrap();
        assert_eq!(defaults.regions[0].width, 100.0);
        assert_eq!(defaults.regions[0].lines, 3);
        assert_eq!(defaults.regions[0].region_anchor, (0.0, 100.0));
        assert_eq!(defaults.regions[0].viewport_anchor, (0.0, 100.0));
        assert_eq!(defaults.regions[0].scroll, "");
    }

    #[test]
    fn webvtt_regions_use_last_identifier_and_ignore_definitions_after_a_cue() {
        let file = parse_vtt_file("WEBVTT\n\nREGION\nid:a lines:1\n\nREGION\nid:a lines:2\n\n00:00.000 --> 00:01.000 region:a\none\n\nREGION\nid:late lines:9\n\n00:01.000 --> 00:02.000 region:a\ntwo\n\n00:02.000 --> 00:03.000 region:late\nthree\n").unwrap();
        assert_eq!(file.regions.len(), 2);
        assert_eq!(file.cues.len(), 3);
        assert_eq!(file.cues[0].region, Some(1));
        assert_eq!(file.cues[1].region, Some(1));
        assert_eq!(file.cues[2].region, None);
    }

    #[test]
    fn webvtt_region_association_follows_setting_order_and_empty_tokens() {
        let settings = [
            "region:a vertical:rl",
            "vertical:rl region:a",
            "region:a line:5",
            "line:5 region:a",
            "region:a size:50%",
            "size:50% region:a",
            "region:a size:100%",
            "region:a line:5,bad",
            "region:a region:",
            "region:a region:missing",
            "region:",
        ];
        let mut source = "WEBVTT\n\nREGION\nid:a\n\nREGION\nwidth:10%\n\n".to_owned();
        for setting in settings {
            source.push_str(&format!("00:00.000 --> 00:01.000 {setting}\ntext\n\n"));
        }
        let file = parse_vtt_file(&source).unwrap();
        let regions = file.cues.iter().map(|cue| cue.region).collect::<Vec<_>>();
        assert_eq!(
            regions,
            vec![
                None,
                Some(0),
                None,
                Some(0),
                None,
                Some(0),
                Some(0),
                Some(0),
                Some(0),
                None,
                None
            ]
        );
    }

    #[test]
    fn webvtt_regions_recover_arrows_and_reject_header_or_vertical_tab_markers() {
        let file = parse_vtt_file("WEBVTT\nREGION\nid:header\n\nREGION\u{0b}\nid:invalid\n\nREGION\u{0c}\nid:valid lines:2\n00:00.000 --> 00:01.000 region:valid\nREGION\ntext\n").unwrap();
        assert_eq!(file.regions.len(), 1);
        assert_eq!(file.regions[0].id, "valid");
        assert_eq!(file.cues.len(), 1);
        assert_eq!(file.cues[0].region, Some(0));
        assert_eq!(file.cues[0].text, "REGION\ntext");
    }
}
