//! Codec recognition for the Chromium-style software compatibility profile.
//! This describes the advertised Web API surface, not installed OS decoders.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Codec {
    Av1,
    Vp8,
    Vp9,
    H264,
    Aac,
    Flac,
    Mp3,
    Opus,
    Pcm,
    Vorbis,
}

impl Codec {
    pub(super) fn parse(label: &str) -> Option<Self> {
        let label = label.trim_ascii();
        match label {
            "vp8" | "vp8.0" => return Some(Self::Vp8),
            "vp9" | "vp9.0" => return Some(Self::Vp9),
            "mp4a.40.2" | "mp4a.40.5" | "mp4a.40.29" => return Some(Self::Aac),
            "flac" => return Some(Self::Flac),
            "mp3" | "mp4a.69" | "mp4a.6B" => return Some(Self::Mp3),
            "opus" => return Some(Self::Opus),
            "1" => return Some(Self::Pcm),
            "vorbis" => return Some(Self::Vorbis),
            _ => {}
        }
        let (name, parameters) = label.split_once('.')?;
        match name {
            "avc1" | "avc3" => {
                if parameters.len() != 6 || !parameters.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return None;
                }
                let value = u32::from_str_radix(parameters, 16).ok()?;
                let profile = value >> 16;
                let level = value & 0xff;
                (matches!(profile, 0x42 | 0x4d | 0x58 | 0x64 | 0x6e | 0x7a | 0xf4)
                    && matches!(level, 10..=13 | 20..=22 | 30..=32 | 40..=42 | 50..=52 | 60..=62))
                .then_some(Self::H264)
            }
            "vp09" => valid_vp9(parameters).then_some(Self::Vp9),
            "av01" => valid_av1(parameters).then_some(Self::Av1),
            // Platform-dependent codecs (HEVC, Dolby, etc.) are not part of
            // this software profile. Unknown labels are never a blanket yes.
            _ => None,
        }
    }

    pub(super) fn is_audio(self) -> bool {
        !matches!(self, Self::Av1 | Self::Vp8 | Self::Vp9 | Self::H264)
    }

    pub(super) fn allowed_in(self, mime: &str) -> bool {
        match mime {
            "audio/mp4" => matches!(self, Self::Aac | Self::Flac | Self::Mp3 | Self::Opus),
            "video/mp4" => {
                matches!(self, Self::Av1 | Self::Vp9 | Self::H264) || self.allowed_in("audio/mp4")
            }
            "audio/webm" => matches!(self, Self::Opus | Self::Vorbis),
            "video/webm" => {
                matches!(self, Self::Av1 | Self::Vp8 | Self::Vp9) || self.allowed_in("audio/webm")
            }
            "audio/ogg" | "video/ogg" | "application/ogg" => {
                matches!(self, Self::Opus | Self::Vorbis | Self::Flac)
            }
            "audio/wav" | "audio/x-wav" => self == Self::Pcm,
            "audio/mpeg" => self == Self::Mp3,
            _ => false,
        }
    }
}

fn decimal(value: &str, width: usize) -> Option<u8> {
    (value.len() == width && value.bytes().all(|c| c.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn valid_vp9(parameters: &str) -> bool {
    let fields: Vec<_> = parameters.split('.').collect();
    if !matches!(fields.len(), 3 | 8) {
        return false;
    }
    let Some(profile @ 0..=3) = decimal(fields[0], 2) else {
        return false;
    };
    let Some(10 | 11 | 20 | 21 | 30 | 31 | 40 | 41 | 50 | 51 | 52 | 60 | 61 | 62) =
        decimal(fields[1], 2)
    else {
        return false;
    };
    let depth = decimal(fields[2], 2);
    if !(if profile < 2 {
        depth == Some(8)
    } else {
        matches!(depth, Some(10 | 12))
    }) {
        return false;
    }
    fields.len() == 3
        || (matches!(decimal(fields[3], 2), Some(0..=3))
            && fields[4..7].iter().all(|field| decimal(field, 2).is_some())
            && matches!(decimal(fields[7], 2), Some(0 | 1)))
}

fn valid_av1(parameters: &str) -> bool {
    let fields: Vec<_> = parameters.split('.').collect();
    if !matches!(fields.len(), 3 | 9) {
        return false;
    }
    let Some(profile @ 0..=2) = decimal(fields[0], 1) else {
        return false;
    };
    let level_and_tier = fields[1].as_bytes();
    if level_and_tier.len() != 3 || !level_and_tier[..2].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let level = (level_and_tier[0] - b'0') * 10 + level_and_tier[1] - b'0';
    if level > 23
        || !matches!(level_and_tier[2], b'M' | b'H')
        || (level_and_tier[2] == b'H' && level < 8)
    {
        return false;
    }
    let depth = decimal(fields[2], 2);
    if !(matches!(depth, Some(8 | 10)) || profile == 2 && depth == Some(12)) {
        return false;
    }
    fields.len() == 3
        || (matches!(decimal(fields[3], 1), Some(0 | 1))
            && fields[4].len() == 3
            && fields[4].bytes().all(|c| c.is_ascii_digit())
            && fields[5..8].iter().all(|field| decimal(field, 2).is_some())
            && matches!(decimal(fields[8], 1), Some(0 | 1)))
}

#[cfg(test)]
mod tests {
    use super::Codec;

    #[test]
    fn codec_parsing_rejects_non_ascii_fields_without_slicing_utf8() {
        // MIME parsing can discard invalid parameters. Exercise the codec
        // boundary directly rather than mistaking that for a bare-container query.
        for label in ["av01.0.☃.08", "av01.0.éM.08", "vp09.☃.10.08", "avc1.é42E"] {
            assert_eq!(Codec::parse(label), None, "{label}");
        }
    }
}
