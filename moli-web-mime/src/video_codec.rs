//! Color requirements carried by RFC 6381 video codec identifiers.

/// CICP color identifiers. `None` represents the explicit "unspecified" code 2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoCodecColorSpace {
    pub primaries: Option<u8>,
    pub transfer: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidVideoCodecColorSpace;

/// Parses VP9 and AV1 color descriptions, including their BT.709 defaults.
/// Other codec families return `Ok(None)`: their identifiers do not signal
/// color in either of these formats. Parsing does not establish decoder support.
pub fn parse_video_codec_color_space(
    codec: &str,
) -> Result<Option<VideoCodecColorSpace>, InvalidVideoCodecColorSpace> {
    // Bound the number of borrowed fields even for arbitrarily long input.
    let fields: Vec<_> = codec.splitn(11, '.').collect();
    match fields[0] {
        "vp09" => vp9_color_space(&fields).map(Some),
        "av01" => av1_color_space(&fields).map(Some),
        _ => Ok(None),
    }
}

fn decimal(field: &str, width: usize) -> Result<u8, InvalidVideoCodecColorSpace> {
    if field.len() != width || !field.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InvalidVideoCodecColorSpace);
    }
    field.parse().map_err(|_| InvalidVideoCodecColorSpace)
}

fn color_space(
    primaries: u8,
    transfer: u8,
    matrix: u8,
) -> Result<VideoCodecColorSpace, InvalidVideoCodecColorSpace> {
    // H.273 / CICP assigned values; reserved identifiers cannot describe a
    // supported color space. Code 2 deliberately leaves a component unspecified.
    if !matches!(primaries, 1 | 2 | 4..=12 | 22)
        || !matches!(transfer, 1 | 2 | 4..=18)
        || !matches!(matrix, 0..=2 | 4..=14)
    {
        return Err(InvalidVideoCodecColorSpace);
    }
    Ok(VideoCodecColorSpace {
        primaries: (primaries != 2).then_some(primaries),
        transfer: (transfer != 2).then_some(transfer),
    })
}

fn vp9_color_space(fields: &[&str]) -> Result<VideoCodecColorSpace, InvalidVideoCodecColorSpace> {
    // https://www.webmproject.org/vp9/mp4/#codecs-parameter-string
    // The five optional fields are mutually inclusive, not a partial suffix.
    if !matches!(fields.len(), 4 | 9) {
        return Err(InvalidVideoCodecColorSpace);
    }
    let profile = decimal(fields[1], 2)?;
    let level = decimal(fields[2], 2)?;
    let depth = decimal(fields[3], 2)?;
    if profile > 3
        || !matches!(level, 0 | 10 | 11 | 20 | 21 | 30 | 31 | 40 | 41 | 50..=52 | 60..=62)
        || !matches!((profile, depth), (0 | 1, 8) | (2 | 3, 10 | 12))
    {
        return Err(InvalidVideoCodecColorSpace);
    }
    let (chroma, primaries, transfer, matrix, full_range) = if fields.len() == 4 {
        (1, 1, 1, 1, 0)
    } else {
        (
            decimal(fields[4], 2)?,
            decimal(fields[5], 2)?,
            decimal(fields[6], 2)?,
            decimal(fields[7], 2)?,
            decimal(fields[8], 2)?,
        )
    };
    if chroma > 3
        || full_range > 1
        || (matches!(profile, 0 | 2) != (chroma <= 1))
        || (matrix == 0 && chroma != 3)
    {
        return Err(InvalidVideoCodecColorSpace);
    }
    color_space(primaries, transfer, matrix)
}

fn av1_color_space(fields: &[&str]) -> Result<VideoCodecColorSpace, InvalidVideoCodecColorSpace> {
    // https://aomediacodec.github.io/av1-isobmff/#codecsparam
    if !matches!(fields.len(), 4 | 10) {
        return Err(InvalidVideoCodecColorSpace);
    }
    let profile = decimal(fields[1], 1)?;
    let level_tier = fields[2].as_bytes();
    if level_tier.len() != 3 || !matches!(level_tier[2], b'M' | b'H') {
        return Err(InvalidVideoCodecColorSpace);
    }
    let level = decimal(&fields[2][..2], 2)?;
    let depth = decimal(fields[3], 2)?;
    if profile > 2
        || level > 31
        || (level_tier[2] == b'H' && level < 8)
        || !matches!(depth, 8 | 10 | 12)
        || (depth == 12 && profile != 2)
    {
        return Err(InvalidVideoCodecColorSpace);
    }
    let (monochrome, chroma, primaries, transfer, matrix, full_range) = if fields.len() == 4 {
        (0, "110", 1, 1, 1, 0)
    } else {
        (
            decimal(fields[4], 1)?,
            fields[5],
            decimal(fields[6], 2)?,
            decimal(fields[7], 2)?,
            decimal(fields[8], 2)?,
            decimal(fields[9], 1)?,
        )
    };
    let valid_chroma = matches!(chroma, "000" | "100" | "110" | "111" | "112");
    if monochrome > 1
        || full_range > 1
        || !valid_chroma
        || (monochrome == 1 && (profile == 1 || chroma != "110"))
        || (monochrome == 0
            && match profile {
                0 => !chroma.starts_with("11"),
                1 => chroma != "000",
                2 if depth < 12 => chroma != "100",
                _ => false,
            })
        || (matrix == 0 && chroma != "000")
    {
        return Err(InvalidVideoCodecColorSpace);
    }
    color_space(primaries, transfer, matrix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_codec_color_fields_mean_bt709() {
        for codec in ["vp09.00.10.08", "av01.0.04M.08"] {
            assert_eq!(
                parse_video_codec_color_space(codec),
                Ok(Some(VideoCodecColorSpace {
                    primaries: Some(1),
                    transfer: Some(1),
                })),
                "{codec}"
            );
        }
    }

    #[test]
    fn explicit_codec_colors_preserve_hdr_and_unspecified_components() {
        for (codec, primaries, transfer) in [
            ("vp09.02.10.10.01.09.16.09.01", Some(9), Some(16)),
            ("av01.0.04M.10.0.112.09.18.09.0", Some(9), Some(18)),
            ("vp09.00.10.08.01.02.02.02.00", None, None),
            ("av01.0.04M.10.0.110.02.16.09.0", None, Some(16)),
            ("vp09.02.10.10.01.09.02.09.00", Some(9), None),
            ("av01.1.08M.08.0.000.01.13.00.1", Some(1), Some(13)),
        ] {
            assert_eq!(
                parse_video_codec_color_space(codec),
                Ok(Some(VideoCodecColorSpace {
                    primaries,
                    transfer,
                })),
                "{codec}"
            );
        }
    }

    #[test]
    fn codec_fields_require_the_registered_widths_ranges_and_complete_suffix() {
        for codec in [
            "vp09",
            "vp09.00.10",
            "vp09.0.10.08",
            "vp09.00.10.8",
            "vp09.04.10.08",
            "vp09.00.99.08",
            "vp09.00.10.10",
            "vp09.00.10.08.01.09.16",
            "vp09.00.10.08.01.09.16.09.02",
            "vp09.00.10.08.01.09.16.00.00",
            "vp09.00.10.08.01.99.16.09.00",
            "vp09.00.10.08.01.09.99.09.00",
            "vp09.00.10.08.01.09.16.09.00.00",
            "vp09.-0.10.08",
            "vp09.00.10.08.01..16.09.00",
            "av01",
            "av01.00.04M.08",
            "av01.0.4M.08",
            "av01.0.04m.08",
            "av01.0.éM.08",
            "av01.3.04M.08",
            "av01.0.32M.08",
            "av01.0.04H.08",
            "av01.0.04M.12",
            "av01.0.04M.08.0.110.09.16",
            "av01.0.04M.08.0.113.09.16.09.0",
            "av01.0.04M.08.0.100.09.16.09.0",
            "av01.1.08M.08.1.110.09.16.09.0",
            "av01.0.04M.08.0.110.09.16.09.2",
            "av01.0.04M.08.0.110.09.16.09.0.0",
        ] {
            assert_eq!(
                parse_video_codec_color_space(codec),
                Err(InvalidVideoCodecColorSpace),
                "{codec}"
            );
        }
    }

    #[test]
    fn other_codec_identifiers_do_not_acquire_vp9_color_defaults() {
        for codec in ["avc1.42e01e", "vp8", "vp9", "hvc1.1.6.L93.B0", ""] {
            assert_eq!(parse_video_codec_color_space(codec), Ok(None), "{codec}");
        }
    }
}
