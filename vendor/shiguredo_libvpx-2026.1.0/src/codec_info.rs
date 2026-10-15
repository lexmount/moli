//! コーデック情報の照会

/// コーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodecType {
    /// VP8
    Vp8,
    /// VP9
    Vp9,
}

impl VideoCodecType {
    /// すべてのコーデック種別を返す
    fn all() -> &'static [Self] {
        &[Self::Vp8, Self::Vp9]
    }
}

/// コーデックごとの情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecInfo {
    /// コーデック種別
    pub codec: VideoCodecType,
    /// デコード情報
    pub decoding: DecodingInfo,
    /// エンコード情報
    pub encoding: EncodingInfo,
}

/// デコード情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodingInfo {
    /// デコードが可能か
    pub supported: bool,
    /// ハードウェアアクセラレーションが利用可能か
    pub hardware_accelerated: bool,
}

/// エンコード情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingInfo {
    /// エンコードが可能か
    pub supported: bool,
    /// ハードウェアアクセラレーションが利用可能か
    pub hardware_accelerated: bool,
    /// コーデック固有のプロファイル情報
    pub profiles: EncodingProfiles,
}

/// コーデック固有のエンコードプロファイル情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodingProfiles {
    /// VP9 プロファイル一覧
    Vp9(Vec<Vp9EncodingProfile>),
    /// プロファイル情報なし（プロファイルの概念がないコーデック向け）
    Unsupported,
}

/// VP9 エンコードプロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vp9EncodingProfile {
    /// Profile 0 (8-bit 4:2:0)
    Profile0,
    /// Profile 2 (10/12-bit 4:2:0)
    Profile2,
}

/// このバックエンドで利用可能なコーデック情報の一覧を返す
///
/// libvpx はソフトウェアコーデックであるため、VP8/VP9 ともに常にデコード・エンコードが可能で、
/// ハードウェアアクセラレーションは利用できない。
pub fn supported_codecs() -> Vec<CodecInfo> {
    VideoCodecType::all()
        .iter()
        .map(|&codec| CodecInfo {
            codec,
            decoding: decoding_info(codec),
            encoding: encoding_info(codec),
        })
        .collect()
}

/// デコード情報を返す
///
/// libvpx はソフトウェアデコーダーであるため、supported は常に true、
/// hardware_accelerated は常に false になる。
fn decoding_info(_codec: VideoCodecType) -> DecodingInfo {
    DecodingInfo {
        supported: true,
        hardware_accelerated: false,
    }
}

/// エンコード情報を返す
///
/// libvpx はソフトウェアエンコーダーであるため、supported は常に true、
/// hardware_accelerated は常に false になる。
fn encoding_info(codec: VideoCodecType) -> EncodingInfo {
    let profiles = match codec {
        VideoCodecType::Vp8 => EncodingProfiles::Unsupported,
        VideoCodecType::Vp9 => EncodingProfiles::Vp9(vec![
            Vp9EncodingProfile::Profile0,
            Vp9EncodingProfile::Profile2,
        ]),
    };
    EncodingInfo {
        supported: true,
        hardware_accelerated: false,
        profiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_codecs_returns_two_codecs() {
        let codecs = supported_codecs();
        assert_eq!(codecs.len(), 2);
        assert_eq!(codecs[0].codec, VideoCodecType::Vp8);
        assert_eq!(codecs[1].codec, VideoCodecType::Vp9);
    }

    #[test]
    fn vp8_codec_info() {
        let codecs = supported_codecs();
        let vp8 = &codecs[0];
        assert_eq!(
            *vp8,
            CodecInfo {
                codec: VideoCodecType::Vp8,
                decoding: DecodingInfo {
                    supported: true,
                    hardware_accelerated: false,
                },
                encoding: EncodingInfo {
                    supported: true,
                    hardware_accelerated: false,
                    profiles: EncodingProfiles::Unsupported,
                },
            }
        );
    }

    #[test]
    fn vp9_codec_info() {
        let codecs = supported_codecs();
        let vp9 = &codecs[1];
        assert_eq!(
            *vp9,
            CodecInfo {
                codec: VideoCodecType::Vp9,
                decoding: DecodingInfo {
                    supported: true,
                    hardware_accelerated: false,
                },
                encoding: EncodingInfo {
                    supported: true,
                    hardware_accelerated: false,
                    profiles: EncodingProfiles::Vp9(vec![
                        Vp9EncodingProfile::Profile0,
                        Vp9EncodingProfile::Profile2,
                    ]),
                },
            }
        );
    }
}
