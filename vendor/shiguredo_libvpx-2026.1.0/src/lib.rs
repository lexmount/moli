//! [Hisui] 用の [libvpx] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [libvpx]: https://github.com/webmproject/libvpx
#![warn(missing_docs)]

use std::{
    ffi::{CStr, c_int, c_uint},
    mem::MaybeUninit,
    num::NonZeroUsize,
};

mod codec_info;
mod decoder_buffers;
mod sys;

pub use codec_info::*;

/// ビルド時に参照したリポジトリ URL
pub const BUILD_REPOSITORY: &str = sys::BUILD_METADATA_REPOSITORY;

/// ビルド時に参照したリポジトリのバージョン（タグ）
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// エラー
#[derive(Debug)]
pub struct Error {
    code: sys::vpx_codec_err_t,
    function: &'static str,
    reason: Option<&'static str>,
    detail: Option<String>,
}

impl Error {
    fn check(
        code: sys::vpx_codec_err_t,
        function: &'static str,
        ctx: Option<&sys::vpx_codec_ctx>,
    ) -> Result<(), Self> {
        if code == sys::vpx_codec_err_t_VPX_CODEC_OK {
            Ok(())
        } else {
            let detail = unsafe {
                if let Some(ctx) = ctx {
                    let detail_ptr = sys::vpx_codec_error_detail(ctx);
                    if detail_ptr.is_null() {
                        None
                    } else {
                        CStr::from_ptr(detail_ptr)
                            .to_str()
                            .ok()
                            .map(|s| s.to_owned())
                    }
                } else {
                    None
                }
            };
            Err(Self {
                code,
                function,
                reason: None,
                detail,
            })
        }
    }

    fn with_reason(
        code: sys::vpx_codec_err_t,
        function: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            code,
            function,
            reason: Some(reason),
            detail: None,
        }
    }

    fn reason(&self) -> Option<&str> {
        if self.reason.is_some() {
            return self.reason;
        }

        let reason = unsafe { sys::vpx_codec_err_to_string(self.code) };
        if reason.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(reason) }.to_str().ok()
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: code={}", self.function, self.code)?;
        if let Some(reason) = self.reason() {
            write!(f, ", reason={reason}")?;
        }
        if let Some(detail) = &self.detail {
            write!(f, ", detail={detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

/// デコーダー用コーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderCodec {
    /// VP8
    Vp8,
    /// VP9
    Vp9,
}

/// デコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// デコードするコーデック
    pub codec: DecoderCodec,
}

impl DecoderConfig {
    /// 指定したコーデックでデコーダー設定を生成する
    pub fn new(codec: DecoderCodec) -> Self {
        Self { codec }
    }
}

/// VP8 / VP9 デコーダー
pub struct Decoder {
    ctx: sys::vpx_codec_ctx,
    iter: sys::vpx_codec_iter_t,
    // Native destruction runs before this stable callback state is dropped.
    _buffers: Option<Box<decoder_buffers::DecoderBuffers>>,
}

impl Decoder {
    /// デコーダーインスタンスを生成する
    pub fn new(config: DecoderConfig) -> Result<Self, Error> {
        unsafe {
            let iface = match config.codec {
                DecoderCodec::Vp8 => sys::vpx_codec_vp8_dx(),
                DecoderCodec::Vp9 => sys::vpx_codec_vp9_dx(),
            };
            Self::init(iface, None)
        }
    }

    /// Create a decoder with fallible, bounded external VP9 frame storage.
    /// VP8 allocation dimensions can be checked with peek_stream_dimensions.
    pub fn new_with_max_frame_bytes(config: DecoderConfig, maximum: usize) -> Result<Self, Error> {
        let iface = unsafe {
            match config.codec {
                DecoderCodec::Vp8 => sys::vpx_codec_vp8_dx(),
                DecoderCodec::Vp9 => sys::vpx_codec_vp9_dx(),
            }
        };
        Self::init(
            iface,
            (config.codec == DecoderCodec::Vp9).then_some(maximum),
        )
    }

    /// Raise an external-buffer bound after its owner admits a larger frame.
    pub fn increase_max_frame_bytes(&mut self, maximum: usize) {
        if let Some(buffers) = &self._buffers {
            buffers.increase_limit(maximum);
        }
    }

    /// Inspect independently decodable frame dimensions before allocating
    /// native codec storage. Interframes can rely on codec reference frames.
    pub fn peek_stream_dimensions(codec: DecoderCodec, data: &[u8]) -> Option<(u32, u32)> {
        let length = c_uint::try_from(data.len()).ok()?;
        let iface = unsafe {
            match codec {
                DecoderCodec::Vp8 => sys::vpx_codec_vp8_dx(),
                DecoderCodec::Vp9 => sys::vpx_codec_vp9_dx(),
            }
        };
        let mut info: sys::vpx_codec_stream_info = unsafe { std::mem::zeroed() };
        info.sz = std::mem::size_of_val(&info) as c_uint;
        let code =
            unsafe { sys::vpx_codec_peek_stream_info(iface, data.as_ptr(), length, &mut info) };
        (code == sys::vpx_codec_err_t_VPX_CODEC_OK && info.w != 0 && info.h != 0)
            .then_some((info.w, info.h))
    }

    fn init(iface: *const sys::vpx_codec_iface, maximum: Option<usize>) -> Result<Self, Error> {
        let mut ctx = MaybeUninit::<sys::vpx_codec_ctx>::zeroed();
        unsafe {
            let code = sys::vpx_codec_dec_init_ver(
                ctx.as_mut_ptr(),
                iface,
                std::ptr::null(), // cfg
                0,                // flags
                sys::VPX_DECODER_ABI_VERSION as i32,
            );
            let mut ctx = ctx.assume_init();
            Error::check(code, "vpx_codec_dec_init_ver", Some(&ctx))?;
            let buffers =
                maximum.map(|maximum| Box::new(decoder_buffers::DecoderBuffers::new(maximum)));
            if let Some(buffers) = buffers.as_ref() {
                let code = buffers.install(&mut ctx);
                if let Err(error) =
                    Error::check(code, "vpx_codec_set_frame_buffer_functions", Some(&ctx))
                {
                    sys::vpx_codec_destroy(&mut ctx);
                    return Err(error);
                }
            }
            Ok(Self {
                ctx,
                iter: std::ptr::null(),
                _buffers: buffers,
            })
        }
    }

    /// 圧縮された映像フレームをデコードする
    ///
    /// デコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Decoder::decode",
                "still need to call shiguredo_libvpx::Decoder::next_frame()",
            ));
        }

        let code = unsafe {
            sys::vpx_codec_decode(
                &mut self.ctx,
                data.as_ptr(),
                data.len() as c_uint,
                std::ptr::null_mut(), // user_priv
                0, // deadline (ドキュメントによると、値は無視されるので常に 0 を指定しろとのこと）
            )
        };
        Error::check(code, "vpx_codec_decode", Some(&self.ctx))?;
        Ok(())
    }

    /// これ以上データが来ないことをデコーダーに伝える
    ///
    /// 残りのデコード結果は [`Decoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Decoder::finish",
                "still need to call shiguredo_libvpx::Decoder::next_frame()",
            ));
        }

        let code = unsafe {
            sys::vpx_codec_decode(
                &mut self.ctx,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
            )
        };
        Error::check(code, "vpx_codec_decode", Some(&self.ctx))?;
        Ok(())
    }

    /// デコード済みのフレームを取り出す
    ///
    /// [`Decoder::decode()`] や [`Decoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame<'_>>, Error> {
        unsafe {
            let image = sys::vpx_codec_get_frame(&mut self.ctx, &mut self.iter);
            if image.is_null() {
                self.iter = std::ptr::null();
                return Ok(None);
            }
            let image = &*image;

            // デコーダーは I420 または 16-bit I420 のみ対応
            if !matches!(
                image.fmt,
                sys::vpx_img_fmt_VPX_IMG_FMT_I420 | sys::vpx_img_fmt_VPX_IMG_FMT_I42016
            ) {
                self.iter = std::ptr::null();
                return Err(Error::with_reason(
                    sys::vpx_codec_err_t_VPX_CODEC_UNSUP_FEATURE,
                    "vpx_codec_get_frame",
                    "unsupported image format",
                ));
            }

            Ok(Some(DecodedFrame(image)))
        }
    }
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

/// デコードされた映像フレーム (I420 形式)
pub struct DecodedFrame<'a>(&'a sys::vpx_image);

impl DecodedFrame<'_> {
    /// The libvpx color-space identifier, zero when unspecified.
    pub fn color_space(&self) -> u32 {
        self.0.cs
    }

    /// Whether the decoded samples use the full rather than broadcast range.
    pub fn full_range(&self) -> bool {
        self.0.range == sys::vpx_color_range_VPX_CR_FULL_RANGE
    }

    /// フレームが高ビット深度（16ビット）かどうかを返す
    //
    // libvpx での高ビット深度フォーマットについてのメモ：
    // - libvpx は VP9 の 10-bit プロファイル（Profile 2 など）をサポート
    // - 高ビット深度データは 16-bit リトルエンディアン形式で格納される
    // - 実際の値範囲は 10-bit (0-1023) だが、上位6ビットは未使用
    // - YUV420 サブサンプリングは通常の 8-bit と同様に適用される
    // - ストライドは 16-bit 単位（バイト数は width * 2）で計算される
    pub fn is_high_depth(&self) -> bool {
        self.0.fmt == sys::vpx_img_fmt_VPX_IMG_FMT_I42016
    }

    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(self.0.planes[0], self.0.d_h as usize * self.y_stride())
        }
    }

    /// フレームの U 成分のデータを返す
    pub fn u_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.planes[1],
                self.0.d_h.div_ceil(2) as usize * self.u_stride(),
            )
        }
    }

    /// フレームの V 成分のデータを返す
    pub fn v_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.0.planes[2],
                self.0.d_h.div_ceil(2) as usize * self.v_stride(),
            )
        }
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        self.0.stride[0] as usize
    }

    /// フレームの U 成分のストライドを返す
    pub fn u_stride(&self) -> usize {
        self.0.stride[1] as usize
    }

    /// フレームの V 成分のストライドを返す
    pub fn v_stride(&self) -> usize {
        self.0.stride[2] as usize
    }

    /// フレームの幅を返す
    pub fn width(&self) -> usize {
        self.0.d_w as usize
    }

    /// フレームの高さを返す
    pub fn height(&self) -> usize {
        self.0.d_h as usize
    }
}

/// エンコーダーの入力画像フォーマット
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// YUV 4:2:0 planar (3 プレーン: Y, U, V)
    I420,
    /// YUV 4:2:0 planar (3 プレーン: Y, V, U)
    Yv12,
    /// YUV 4:2:0 semi-planar (2 プレーン: Y, UV interleaved)
    Nv12,
    /// YUV 4:2:2 planar (3 プレーン: Y, U, V)
    I422,
    /// YUV 4:4:4 planar (3 プレーン: Y, U, V)
    I444,
    /// YUV 4:4:0 planar (3 プレーン: Y, U, V)
    I440,
    /// YUV 4:2:0 planar 16-bit (3 プレーン: Y, U, V)
    I42016,
    /// YUV 4:2:2 planar 16-bit (3 プレーン: Y, U, V)
    I42216,
    /// YUV 4:4:4 planar 16-bit (3 プレーン: Y, U, V)
    I44416,
    /// YUV 4:4:0 planar 16-bit (3 プレーン: Y, U, V)
    I44016,
}

/// エンコーダーに渡す画像データ
pub enum ImageData<'a> {
    /// I420 (3 プレーン: Y, U, V)
    I420 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// YV12 (3 プレーン: Y, V, U)
    Yv12 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// NV12 (2 プレーン: Y, UV interleaved)
    Nv12 {
        /// Y プレーン
        y: &'a [u8],
        /// UV interleaved プレーン
        uv: &'a [u8],
    },
    /// I422 (3 プレーン: Y, U, V)
    I422 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// I444 (3 プレーン: Y, U, V)
    I444 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// I440 (3 プレーン: Y, U, V)
    I440 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// I42016 (3 プレーン: Y, U, V / 16-bit)
    I42016 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// I42216 (3 プレーン: Y, U, V / 16-bit)
    I42216 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// I44416 (3 プレーン: Y, U, V / 16-bit)
    I44416 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
    /// I44016 (3 プレーン: Y, U, V / 16-bit)
    I44016 {
        /// Y プレーン
        y: &'a [u8],
        /// U プレーン
        u: &'a [u8],
        /// V プレーン
        v: &'a [u8],
    },
}

impl ImageData<'_> {
    /// この画像データに対応するフォーマットを返す
    fn format(&self) -> ImageFormat {
        match self {
            ImageData::I420 { .. } => ImageFormat::I420,
            ImageData::Yv12 { .. } => ImageFormat::Yv12,
            ImageData::Nv12 { .. } => ImageFormat::Nv12,
            ImageData::I422 { .. } => ImageFormat::I422,
            ImageData::I444 { .. } => ImageFormat::I444,
            ImageData::I440 { .. } => ImageFormat::I440,
            ImageData::I42016 { .. } => ImageFormat::I42016,
            ImageData::I42216 { .. } => ImageFormat::I42216,
            ImageData::I44416 { .. } => ImageFormat::I44416,
            ImageData::I44016 { .. } => ImageFormat::I44016,
        }
    }
}

/// 各プレーンの期待サイズ
enum PlaneSizes {
    /// 3 プレーン (I420, YV12, I422, I444, I440, I42016, I42216, I44416, I44016)
    ThreePlanes {
        y_size: usize,
        u_size: usize,
        v_size: usize,
    },
    /// 2 プレーン (NV12)
    TwoPlanes { y_size: usize, uv_size: usize },
}

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// 入出力画像の幅
    pub width: usize,

    /// 入出力画像の高さ
    pub height: usize,

    /// 入力画像フォーマット
    pub image_format: ImageFormat,

    /// FPS の分子
    pub fps_numerator: usize,

    /// FPS の分母
    pub fps_denominator: usize,

    /// エンコードビットレート (bps 単位)
    pub target_bitrate: usize,

    /// libvpx に指定する品質調整用パラメーター
    pub min_quantizer: usize,

    /// libvpx に指定する品質調整用パラメーター
    pub max_quantizer: usize,

    /// libvpx に指定する品質調整用パラメーター
    pub cq_level: usize,

    /// エンコード速度設定 (VP8: 0-16, VP9: 0-9, 大きいほど高速)
    pub cpu_used: Option<usize>,

    /// エンコード期限設定
    pub deadline: EncodingDeadline,

    /// レート制御モード
    pub rate_control: RateControlMode,

    /// 先読みフレーム数 (None で無効、品質 vs 速度のトレードオフ)
    pub lag_in_frames: Option<NonZeroUsize>,

    /// スレッド数 (None で自動設定)
    pub threads: Option<NonZeroUsize>,

    /// エラー耐性モード (リアルタイム用途で有効)
    pub error_resilient: bool,

    /// キーフレーム間隔 (フレーム数)
    pub keyframe_interval: Option<NonZeroUsize>,

    // TODO(sile): 今は encode() がタイムスタンプの情報を受け取らないので、フレームドロップとは相性が悪い
    /// フレームドロップ閾値 (0-100, リアルタイム用途)
    pub frame_drop_threshold: Option<usize>,

    /// コーデック固有設定
    pub codec: CodecConfig,
}

impl EncoderConfig {
    /// 必須パラメータを指定してエンコーダー設定を生成する
    ///
    /// オプションパラメータは以下の値で初期化される:
    /// - `fps_numerator`: 30
    /// - `fps_denominator`: 1
    /// - `target_bitrate`: 2_000_000
    /// - `min_quantizer`: 0
    /// - `max_quantizer`: 63
    /// - `cq_level`: 10
    /// - `cpu_used`: None
    /// - `deadline`: Good
    /// - `rate_control`: Vbr
    /// - `lag_in_frames`: None
    /// - `threads`: None
    /// - `error_resilient`: false
    /// - `keyframe_interval`: None
    /// - `frame_drop_threshold`: None
    pub fn new(width: usize, height: usize, image_format: ImageFormat, codec: CodecConfig) -> Self {
        Self {
            width,
            height,
            image_format,
            fps_numerator: 30,
            fps_denominator: 1,
            target_bitrate: 2_000_000,
            min_quantizer: 0,
            max_quantizer: 63,
            cq_level: 10,
            cpu_used: None,
            deadline: EncodingDeadline::Good,
            rate_control: RateControlMode::Vbr,
            lag_in_frames: None,
            threads: None,
            error_resilient: false,
            keyframe_interval: None,
            frame_drop_threshold: None,
            codec,
        }
    }
}

/// エンコード期限設定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingDeadline {
    /// 最高品質 (最も時間がかかる)
    Best,
    /// 良い品質 (品質と速度のバランス)
    Good,
    /// リアルタイム (最も高速)
    Realtime,
}

/// レート制御モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    /// Variable Bitrate (可変ビットレート)
    Vbr,
    /// Constant Bitrate (固定ビットレート)
    Cbr,
    /// Constant Quality (固定品質)
    Cq,
}

/// VP9 プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vp9Profile {
    /// Profile 0 (8-bit 4:2:0)
    #[default]
    Profile0,
    /// Profile 2 (10/12-bit 4:2:0)
    Profile2,
}

/// VP9 固有の設定
#[derive(Debug, Clone, Default)]
pub struct Vp9Config {
    /// プロファイル
    pub profile: Vp9Profile,

    /// 適応的量子化モード (0-3)
    pub aq_mode: Option<i32>,

    /// デノイザー設定 (0-3)
    pub noise_sensitivity: Option<i32>,

    /// タイル列数 (並列処理用)
    pub tile_columns: Option<i32>,

    /// タイル行数 (並列処理用)
    pub tile_rows: Option<i32>,

    /// 行マルチスレッド有効
    pub row_mt: bool,

    /// フレーム並列デコード有効
    pub frame_parallel_decoding: bool,

    /// コンテンツタイプ最適化
    pub tune_content: Option<ContentType>,
}

/// VP8 固有の設定
#[derive(Debug, Clone, Default)]
pub struct Vp8Config {
    /// デノイザー設定 (0-3)
    pub noise_sensitivity: Option<i32>,

    /// 静的閾値
    pub static_threshold: Option<i32>,

    /// トークンパーティション数
    pub token_partitions: Option<i32>,

    /// 最大イントラビットレート率
    pub max_intra_bitrate_pct: Option<i32>,

    /// ARNRフィルタ設定
    pub arnr_config: Option<ArnrConfig>,
}

/// コンテンツタイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// 通常の映像
    Default,
    /// スクリーン録画
    Screen,
}

/// エンコーダー用コーデック設定
#[derive(Debug, Clone)]
pub enum CodecConfig {
    /// VP8 コーデック設定
    Vp8(Vp8Config),
    /// VP9 コーデック設定
    Vp9(Vp9Config),
}

/// ARNR フィルタ設定
#[derive(Debug, Clone)]
pub struct ArnrConfig {
    /// 最大フレーム数
    pub max_frames: i32,
    /// 強度
    pub strength: i32,
    /// タイプ
    pub filter_type: i32,
}

/// エンコード時のオプション
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// キーフレームを強制する
    pub force_keyframe: bool,
}

/// VP8 / VP9 エンコーダー
pub struct Encoder {
    ctx: sys::vpx_codec_ctx,
    img: sys::vpx_image,
    iter: sys::vpx_codec_iter_t,
    frame_count: usize,
    deadline: EncodingDeadline,
    image_format: ImageFormat,
    plane_sizes: PlaneSizes,
}

impl Encoder {
    /// エンコーダーインスタンスを生成する
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        let mut cfg = MaybeUninit::<sys::vpx_codec_enc_cfg>::zeroed();
        unsafe {
            let iface = match &config.codec {
                CodecConfig::Vp8(_) => sys::vpx_codec_vp8_cx(),
                CodecConfig::Vp9(_) => sys::vpx_codec_vp9_cx(),
            };
            let usage = 0; // ドキュメントでは、常に 0 を指定しろ、とのこと
            let code = sys::vpx_codec_enc_config_default(iface, cfg.as_mut_ptr(), usage);
            Error::check(code, "vpx_codec_enc_config_default", None)?;

            let cfg = cfg.assume_init();
            Self::init(&config, cfg, iface)
        }
    }

    fn init(
        encoder_config: &EncoderConfig,
        mut vpx_config: sys::vpx_codec_enc_cfg,
        iface: *const sys::vpx_codec_iface,
    ) -> Result<Self, Error> {
        // 基本設定
        vpx_config.g_w = encoder_config.width as c_uint;
        vpx_config.g_h = encoder_config.height as c_uint;
        vpx_config.rc_target_bitrate = encoder_config.target_bitrate as c_uint / 1000;
        vpx_config.rc_min_quantizer = encoder_config.min_quantizer as c_uint;
        vpx_config.rc_max_quantizer = encoder_config.max_quantizer as c_uint;

        // プロファイル設定
        if let CodecConfig::Vp9(vp9_config) = &encoder_config.codec {
            vpx_config.g_profile = match vp9_config.profile {
                Vp9Profile::Profile0 => 0,
                Vp9Profile::Profile2 => 2,
            };
        }

        // FPS とは分子・分母の関係が逆になる
        vpx_config.g_timebase.num = encoder_config.fps_denominator as c_int;
        vpx_config.g_timebase.den = encoder_config.fps_numerator as c_int;

        if let Some(lag) = encoder_config.lag_in_frames {
            vpx_config.g_lag_in_frames = lag.get() as c_uint;
        }

        if let Some(threads) = encoder_config.threads {
            vpx_config.g_threads = threads.get() as c_uint;
        }

        if encoder_config.error_resilient {
            vpx_config.g_error_resilient = 1;
        }

        if let Some(kf_interval) = encoder_config.keyframe_interval {
            vpx_config.kf_max_dist = kf_interval.get() as c_uint;
        }

        if let Some(threshold) = encoder_config.frame_drop_threshold {
            vpx_config.rc_dropframe_thresh = threshold as c_uint;
        }

        // レート制御モード設定
        vpx_config.rc_end_usage = match encoder_config.rate_control {
            RateControlMode::Vbr => sys::vpx_rc_mode_VPX_VBR,
            RateControlMode::Cbr => sys::vpx_rc_mode_VPX_CBR,
            RateControlMode::Cq => sys::vpx_rc_mode_VPX_CQ,
        };

        let mut ctx = MaybeUninit::<sys::vpx_codec_ctx>::zeroed();
        unsafe {
            let code = sys::vpx_codec_enc_init_ver(
                ctx.as_mut_ptr(),
                iface,
                &vpx_config,
                0, // flags
                sys::VPX_ENCODER_ABI_VERSION as i32,
            );
            Error::check(code, "vpx_codec_enc_init_ver", None)?;

            let img_fmt = match encoder_config.image_format {
                ImageFormat::I420 => sys::vpx_img_fmt_VPX_IMG_FMT_I420,
                ImageFormat::Yv12 => sys::vpx_img_fmt_VPX_IMG_FMT_YV12,
                ImageFormat::Nv12 => sys::vpx_img_fmt_VPX_IMG_FMT_NV12,
                ImageFormat::I422 => sys::vpx_img_fmt_VPX_IMG_FMT_I422,
                ImageFormat::I444 => sys::vpx_img_fmt_VPX_IMG_FMT_I444,
                ImageFormat::I440 => sys::vpx_img_fmt_VPX_IMG_FMT_I440,
                ImageFormat::I42016 => sys::vpx_img_fmt_VPX_IMG_FMT_I42016,
                ImageFormat::I42216 => sys::vpx_img_fmt_VPX_IMG_FMT_I42216,
                ImageFormat::I44416 => sys::vpx_img_fmt_VPX_IMG_FMT_I44416,
                ImageFormat::I44016 => sys::vpx_img_fmt_VPX_IMG_FMT_I44016,
            };

            let mut img = MaybeUninit::zeroed();
            let result = sys::vpx_img_alloc(
                img.as_mut_ptr(),
                img_fmt,
                vpx_config.g_w,
                vpx_config.g_h,
                1, // align に 1 を指定することで width == y_stride となることが保証される
            );
            if result.is_null() {
                // vpx_img_alloc が失敗した場合、初期化済みの codec context を手動で解放する
                sys::vpx_codec_destroy(ctx.as_mut_ptr());
                return Err(Error::with_reason(
                    sys::vpx_codec_err_t_VPX_CODEC_MEM_ERROR,
                    "vpx_img_alloc",
                    "image allocation failed",
                ));
            }

            let img = img.assume_init();
            let height = encoder_config.height;
            let plane_sizes = match encoder_config.image_format {
                ImageFormat::Nv12 => PlaneSizes::TwoPlanes {
                    y_size: height * img.stride[0] as usize,
                    uv_size: height.div_ceil(2) * img.stride[1] as usize,
                },
                // 4:2:0 系 (U/V は幅・高さともに半分)
                ImageFormat::I420 | ImageFormat::Yv12 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height.div_ceil(2) * img.stride[1] as usize,
                    v_size: height.div_ceil(2) * img.stride[2] as usize,
                },
                // 4:2:2 系 (U/V は幅が半分、高さは同じ)
                ImageFormat::I422 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height * img.stride[1] as usize,
                    v_size: height * img.stride[2] as usize,
                },
                // 4:4:4 系 (U/V は Y と同サイズ)
                ImageFormat::I444 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height * img.stride[1] as usize,
                    v_size: height * img.stride[2] as usize,
                },
                // 4:4:0 系 (U/V は幅は同じ、高さが半分)
                ImageFormat::I440 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height.div_ceil(2) * img.stride[1] as usize,
                    v_size: height.div_ceil(2) * img.stride[2] as usize,
                },
                // 16-bit 4:2:0 系
                ImageFormat::I42016 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height.div_ceil(2) * img.stride[1] as usize,
                    v_size: height.div_ceil(2) * img.stride[2] as usize,
                },
                // 16-bit 4:2:2 系
                ImageFormat::I42216 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height * img.stride[1] as usize,
                    v_size: height * img.stride[2] as usize,
                },
                // 16-bit 4:4:4 系
                ImageFormat::I44416 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height * img.stride[1] as usize,
                    v_size: height * img.stride[2] as usize,
                },
                // 16-bit 4:4:0 系
                ImageFormat::I44016 => PlaneSizes::ThreePlanes {
                    y_size: height * img.stride[0] as usize,
                    u_size: height.div_ceil(2) * img.stride[1] as usize,
                    v_size: height.div_ceil(2) * img.stride[2] as usize,
                },
            };

            let mut this = Self {
                ctx: ctx.assume_init(),
                img,
                iter: std::ptr::null(),
                frame_count: 0,
                deadline: encoder_config.deadline,
                image_format: encoder_config.image_format,
                plane_sizes,
            };
            // NOTE: これ以降の操作に失敗しても ctx は Drop によって確実に解放される

            // CQ Level設定
            let code = sys::vpx_codec_control_(
                &mut this.ctx,
                sys::vp8e_enc_control_id_VP8E_SET_CQ_LEVEL as c_int,
                encoder_config.cq_level as c_uint,
            );
            Error::check(code, "vpx_codec_control_", Some(&this.ctx))?;

            // CPU使用率設定
            if let Some(cpu_used) = encoder_config.cpu_used {
                let code = sys::vpx_codec_control_(
                    &mut this.ctx,
                    sys::vp8e_enc_control_id_VP8E_SET_CPUUSED as c_int,
                    cpu_used,
                );
                Error::check(code, "vpx_codec_control_", Some(&this.ctx))?;
            }

            // コーデック固有設定
            match &encoder_config.codec {
                CodecConfig::Vp8(vp8_config) => this.configure_vp8(vp8_config)?,
                CodecConfig::Vp9(vp9_config) => this.configure_vp9(vp9_config)?,
            }

            Ok(this)
        }
    }

    fn configure_vp9(&mut self, vp9_config: &Vp9Config) -> Result<(), Error> {
        // 適応的量子化モード
        if let Some(aq_mode) = vp9_config.aq_mode {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_AQ_MODE as c_int,
                    aq_mode,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // デノイザー設定
        if let Some(noise_sensitivity) = vp9_config.noise_sensitivity {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_NOISE_SENSITIVITY as c_int,
                    noise_sensitivity,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // タイル列数
        if let Some(tile_columns) = vp9_config.tile_columns {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_TILE_COLUMNS as c_int,
                    tile_columns,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // タイル行数
        if let Some(tile_rows) = vp9_config.tile_rows {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_TILE_ROWS as c_int,
                    tile_rows,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // 行マルチスレッド
        if vp9_config.row_mt {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_ROW_MT as c_int,
                    1,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // フレーム並列デコード
        if vp9_config.frame_parallel_decoding {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_FRAME_PARALLEL_DECODING as c_int,
                    1,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // コンテンツタイプ最適化
        if let Some(tune_content) = vp9_config.tune_content {
            let content_type = match tune_content {
                ContentType::Default => sys::vp9e_tune_content_VP9E_CONTENT_DEFAULT,
                ContentType::Screen => sys::vp9e_tune_content_VP9E_CONTENT_SCREEN,
            };
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP9E_SET_TUNE_CONTENT as c_int,
                    content_type as c_int,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        Ok(())
    }

    fn configure_vp8(&mut self, vp8_config: &Vp8Config) -> Result<(), Error> {
        // デノイザー設定
        if let Some(noise_sensitivity) = vp8_config.noise_sensitivity {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP8E_SET_NOISE_SENSITIVITY as c_int,
                    noise_sensitivity,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // 静的閾値
        if let Some(static_threshold) = vp8_config.static_threshold {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP8E_SET_STATIC_THRESHOLD as c_int,
                    static_threshold,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // トークンパーティション数
        if let Some(token_partitions) = vp8_config.token_partitions {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP8E_SET_TOKEN_PARTITIONS as c_int,
                    token_partitions,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // 最大イントラビットレート率
        if let Some(max_intra_bitrate_pct) = vp8_config.max_intra_bitrate_pct {
            let code = unsafe {
                sys::vpx_codec_control_(
                    &mut self.ctx,
                    sys::vp8e_enc_control_id_VP8E_SET_MAX_INTRA_BITRATE_PCT as c_int,
                    max_intra_bitrate_pct,
                )
            };
            Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;
        }

        // ARNRフィルタ設定
        if let Some(arnr_config) = &vp8_config.arnr_config {
            self.configure_vp8_arnr(arnr_config)?;
        }

        Ok(())
    }

    fn configure_vp8_arnr(&mut self, arnr_config: &ArnrConfig) -> Result<(), Error> {
        // ARNRを有効化
        let code = unsafe {
            sys::vpx_codec_control_(
                &mut self.ctx,
                sys::vp8e_enc_control_id_VP8E_SET_ENABLEAUTOALTREF as c_int,
                1,
            )
        };
        Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;

        // ARNR最大フレーム数
        let code = unsafe {
            sys::vpx_codec_control_(
                &mut self.ctx,
                sys::vp8e_enc_control_id_VP8E_SET_ARNR_MAXFRAMES as c_int,
                arnr_config.max_frames,
            )
        };
        Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;

        // ARNR強度
        let code = unsafe {
            sys::vpx_codec_control_(
                &mut self.ctx,
                sys::vp8e_enc_control_id_VP8E_SET_ARNR_STRENGTH as c_int,
                arnr_config.strength,
            )
        };
        Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;

        // ARNRタイプ
        let code = unsafe {
            sys::vpx_codec_control_(
                &mut self.ctx,
                sys::vp8e_enc_control_id_VP8E_SET_ARNR_TYPE as c_int,
                arnr_config.filter_type,
            )
        };
        Error::check(code, "vpx_codec_control_", Some(&self.ctx))?;

        Ok(())
    }

    /// 画像データをエンコードする
    ///
    /// エンコード結果は [`Encoder::next_frame()`] で取得できる
    ///
    /// `image` のフォーマットはエンコーダー初期化時に指定した `ImageFormat` と一致する必要がある
    pub fn encode(&mut self, image: &ImageData<'_>, options: &EncodeOptions) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Encoder::encode",
                "still need to call shiguredo_libvpx::Encoder::next_frame()",
            ));
        }

        // フォーマット整合性チェック
        if image.format() != self.image_format {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_INVALID_PARAM,
                "shiguredo_libvpx::Encoder::encode",
                "image format mismatch",
            ));
        }

        // プレーンサイズ検証
        match (image, &self.plane_sizes) {
            (
                ImageData::I420 { y, u, v }
                | ImageData::Yv12 { y, u, v }
                | ImageData::I422 { y, u, v }
                | ImageData::I444 { y, u, v }
                | ImageData::I440 { y, u, v }
                | ImageData::I42016 { y, u, v }
                | ImageData::I42216 { y, u, v }
                | ImageData::I44416 { y, u, v }
                | ImageData::I44016 { y, u, v },
                PlaneSizes::ThreePlanes {
                    y_size,
                    u_size,
                    v_size,
                },
            ) => {
                if y.len() != *y_size || u.len() != *u_size || v.len() != *v_size {
                    return Err(Error::with_reason(
                        sys::vpx_codec_err_t_VPX_CODEC_INVALID_PARAM,
                        "shiguredo_libvpx::Encoder::encode",
                        "invalid plane sizes",
                    ));
                }
            }
            (ImageData::Nv12 { y, uv }, PlaneSizes::TwoPlanes { y_size, uv_size }) => {
                if y.len() != *y_size || uv.len() != *uv_size {
                    return Err(Error::with_reason(
                        sys::vpx_codec_err_t_VPX_CODEC_INVALID_PARAM,
                        "shiguredo_libvpx::Encoder::encode",
                        "invalid plane sizes",
                    ));
                }
            }
            _ => unreachable!(),
        }

        // deadline 設定を適用
        let deadline = match self.deadline {
            EncodingDeadline::Best => sys::VPX_DL_BEST_QUALITY,
            EncodingDeadline::Good => sys::VPX_DL_GOOD_QUALITY,
            EncodingDeadline::Realtime => sys::VPX_DL_REALTIME,
        };

        // フラグ設定
        let mut flags: sys::vpx_enc_frame_flags_t = 0;
        if options.force_keyframe {
            flags |= sys::VPX_EFLAG_FORCE_KF as sys::vpx_enc_frame_flags_t;
        }

        let code = unsafe {
            // 画像データをバッファにコピー
            match image {
                ImageData::I420 { y, u, v }
                | ImageData::Yv12 { y, u, v }
                | ImageData::I422 { y, u, v }
                | ImageData::I444 { y, u, v }
                | ImageData::I440 { y, u, v }
                | ImageData::I42016 { y, u, v }
                | ImageData::I42216 { y, u, v }
                | ImageData::I44416 { y, u, v }
                | ImageData::I44016 { y, u, v } => {
                    std::slice::from_raw_parts_mut(self.img.planes[0], y.len()).copy_from_slice(y);
                    std::slice::from_raw_parts_mut(self.img.planes[1], u.len()).copy_from_slice(u);
                    std::slice::from_raw_parts_mut(self.img.planes[2], v.len()).copy_from_slice(v);
                }
                ImageData::Nv12 { y, uv } => {
                    std::slice::from_raw_parts_mut(self.img.planes[0], y.len()).copy_from_slice(y);
                    std::slice::from_raw_parts_mut(self.img.planes[1], uv.len())
                        .copy_from_slice(uv);
                }
            }

            // エンコード実行
            sys::vpx_codec_encode(
                &mut self.ctx,
                &self.img,
                self.frame_count as sys::vpx_codec_pts_t,
                1, // duration: 1 は「1 フレーム分」を意味する
                flags,
                deadline as sys::vpx_enc_deadline_t,
            )
        };
        Error::check(code, "vpx_codec_encode", Some(&self.ctx))?;
        self.frame_count += 1;
        Ok(())
    }

    /// これ以上データが来ないことをエンコーダーに伝える
    ///
    /// 残りのエンコード結果は [`Encoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        if !self.iter.is_null() {
            return Err(Error::with_reason(
                sys::vpx_codec_err_t_VPX_CODEC_ERROR,
                "shiguredo_libvpx::Encoder::finish",
                "still need to call shiguredo_libvpx::Encoder::next_frame()",
            ));
        }

        let code = unsafe {
            sys::vpx_codec_encode(
                &mut self.ctx,
                std::ptr::null(),
                -1, // pts
                0,  // duration
                0,  // flags
                sys::VPX_DL_REALTIME as sys::vpx_enc_deadline_t,
            )
        };
        Error::check(code, "vpx_codec_encode", Some(&self.ctx))?;
        Ok(())
    }

    /// エンコード済みのフレームを取り出す
    ///
    /// [`Encoder::encode()`] や [`Encoder::finish()`] の後には、
    /// このメソッドを、結果が `None` になるまで呼び出し続ける必要がある
    pub fn next_frame(&mut self) -> Option<EncodedFrame<'_>> {
        unsafe {
            loop {
                let pkt = sys::vpx_codec_get_cx_data(&mut self.ctx, &mut self.iter);
                if pkt.is_null() {
                    self.iter = std::ptr::null();
                    break;
                }

                let pkt = &*pkt;
                if pkt.kind != sys::vpx_codec_cx_pkt_kind_VPX_CODEC_CX_FRAME_PKT {
                    continue;
                }

                return Some(EncodedFrame(&pkt.data.frame));
            }
        }
        None
    }
}

unsafe impl Send for Encoder {}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            sys::vpx_img_free(&mut self.img);
            sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder").finish_non_exhaustive()
    }
}

/// エンコードされた映像フレーム
pub struct EncodedFrame<'a>(&'a sys::vpx_codec_cx_pkt__bindgen_ty_1__bindgen_ty_1);

impl EncodedFrame<'_> {
    /// 圧縮データ
    pub fn data(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.buf as *mut u8, self.0.sz) }
    }

    /// フレームの幅
    pub fn width(&self) -> u16 {
        self.0.width[0] as u16
    }

    /// フレームの高さ
    pub fn height(&self) -> u16 {
        self.0.height[0] as u16
    }

    /// キーフレームかどうか
    pub fn is_keyframe(&self) -> bool {
        (self.0.flags & sys::VPX_FRAME_IS_KEY) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_vp8_decoder() {
        let config = DecoderConfig {
            codec: DecoderCodec::Vp8,
        };
        assert!(Decoder::new(config).is_ok());
    }

    #[test]
    fn init_vp9_decoder() {
        let config = DecoderConfig {
            codec: DecoderCodec::Vp9,
        };
        assert!(Decoder::new(config).is_ok());
    }

    #[test]
    fn decode_vp8_black() {
        let data = [
            80, 66, 0, 157, 1, 42, 128, 2, 224, 1, 2, 199, 8, 133, 133, 136, 153, 132, 136, 15, 2,
            0, 6, 22, 4, 247, 6, 129, 100, 159, 107, 219, 155, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39,
            56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123,
            39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56,
            123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 56, 123, 39, 55, 128, 254,
            250, 215, 128,
        ];
        let config = DecoderConfig {
            codec: DecoderCodec::Vp8,
        };
        let mut decoder = Decoder::new(config).expect("failed to create decoder");
        let mut decoded_count = 0;

        decoder.decode(&data).expect("failed to decode");
        while decoder
            .next_frame()
            .expect("failed to get next frame")
            .is_some()
        {
            decoded_count += 1;
        }

        decoder.finish().expect("failed to finish");
        while decoder
            .next_frame()
            .expect("failed to get next frame")
            .is_some()
        {
            decoded_count += 1;
        }

        assert_eq!(decoded_count, 1);
    }

    #[test]
    fn decode_vp9_black() {
        let data = [
            130, 73, 131, 66, 0, 39, 240, 29, 246, 0, 56, 36, 28, 24, 74, 16, 0, 80, 97, 246, 58,
            246, 128, 92, 209, 238, 0, 0, 0, 0, 0, 20, 103, 26, 154, 224, 98, 35, 126, 68, 120,
            240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0, 103, 26, 154, 224, 98, 35, 126, 68,
            120, 240, 227, 199, 143, 30, 28, 238, 113, 218, 24, 0,
        ];
        let config = DecoderConfig {
            codec: DecoderCodec::Vp9,
        };
        let mut decoder = Decoder::new(config).expect("failed to create decoder");
        let mut decoded_count = 0;

        decoder.decode(&data).expect("failed to decode");
        while decoder
            .next_frame()
            .expect("failed to get next frame")
            .is_some()
        {
            decoded_count += 1;
        }

        decoder.finish().expect("failed to finish");
        while decoder
            .next_frame()
            .expect("failed to get next frame")
            .is_some()
        {
            decoded_count += 1;
        }

        assert_eq!(decoded_count, 1);
    }

    #[test]
    fn init_vp8_encoder() {
        // OK
        let config = vp8_encoder_config(ImageFormat::I420);
        assert!(Encoder::new(config).is_ok());

        // NG
        let mut config = vp8_encoder_config(ImageFormat::I420);
        config.fps_denominator = 0;
        assert!(Encoder::new(config).is_err());
    }

    #[test]
    fn init_vp9_encoder() {
        // OK
        let config = vp9_encoder_config(ImageFormat::I420);
        assert!(Encoder::new(config).is_ok());

        // NG
        let mut config = vp9_encoder_config(ImageFormat::I420);
        config.fps_denominator = 0;
        assert!(Encoder::new(config).is_err());
    }

    #[test]
    fn encode_vp8_i420_black() {
        let config = vp8_encoder_config(ImageFormat::I420);
        let size = config.width * config.height;
        let mut encoder = Encoder::new(config).expect("failed to create");
        let mut encoded_count = 0;

        let y = vec![0; size];
        let u = vec![0; size / 4];
        let v = vec![0; size / 4];

        encoder
            .encode(
                &ImageData::I420 {
                    y: &y,
                    u: &u,
                    v: &v,
                },
                &EncodeOptions {
                    force_keyframe: false,
                },
            )
            .expect("failed to encode");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 1);
    }

    #[test]
    fn encode_vp9_i420_black() {
        let config = vp9_encoder_config(ImageFormat::I420);
        let size = config.width * config.height;
        let mut encoder = Encoder::new(config).expect("failed to create");
        let mut encoded_count = 0;

        let y = vec![0; size];
        let u = vec![0; size / 4];
        let v = vec![0; size / 4];

        encoder
            .encode(
                &ImageData::I420 {
                    y: &y,
                    u: &u,
                    v: &v,
                },
                &EncodeOptions {
                    force_keyframe: false,
                },
            )
            .expect("failed to encode");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 1);
    }

    #[test]
    fn encode_vp8_nv12_black() {
        let config = vp8_encoder_config(ImageFormat::Nv12);
        let size = config.width * config.height;
        let mut encoder = Encoder::new(config).expect("failed to create");
        let mut encoded_count = 0;

        let y = vec![0; size];
        let uv = vec![0; size / 2];

        encoder
            .encode(
                &ImageData::Nv12 { y: &y, uv: &uv },
                &EncodeOptions {
                    force_keyframe: false,
                },
            )
            .expect("failed to encode");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 1);
    }

    #[test]
    fn encode_vp9_nv12_black() {
        let config = vp9_encoder_config(ImageFormat::Nv12);
        let size = config.width * config.height;
        let mut encoder = Encoder::new(config).expect("failed to create");
        let mut encoded_count = 0;

        let y = vec![0; size];
        let uv = vec![0; size / 2];

        encoder
            .encode(
                &ImageData::Nv12 { y: &y, uv: &uv },
                &EncodeOptions {
                    force_keyframe: false,
                },
            )
            .expect("failed to encode");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        encoder.finish().expect("failed to finish");
        while encoder.next_frame().is_some() {
            encoded_count += 1;
        }

        assert_eq!(encoded_count, 1);
    }

    #[test]
    fn encode_format_mismatch() {
        // I420 エンコーダーに NV12 データを渡す
        let config = vp9_encoder_config(ImageFormat::I420);
        let size = config.width * config.height;
        let mut encoder = Encoder::new(config).expect("failed to create");

        let y = vec![0; size];
        let uv = vec![0; size / 2];

        let result = encoder.encode(
            &ImageData::Nv12 { y: &y, uv: &uv },
            &EncodeOptions {
                force_keyframe: false,
            },
        );
        assert!(result.is_err());

        // NV12 エンコーダーに I420 データを渡す
        let config = vp9_encoder_config(ImageFormat::Nv12);
        let size = config.width * config.height;
        let mut encoder = Encoder::new(config).expect("failed to create");

        let y = vec![0; size];
        let u = vec![0; size / 4];
        let v = vec![0; size / 4];

        let result = encoder.encode(
            &ImageData::I420 {
                y: &y,
                u: &u,
                v: &v,
            },
            &EncodeOptions {
                force_keyframe: false,
            },
        );
        assert!(result.is_err());
    }

    fn vp8_encoder_config(image_format: ImageFormat) -> EncoderConfig {
        let mut config = EncoderConfig::new(
            128,
            128,
            image_format,
            CodecConfig::Vp8(Vp8Config::default()),
        );
        config.target_bitrate = 1_000_000;
        config.min_quantizer = 1;
        config.max_quantizer = 1;
        config.cq_level = 1;
        config
    }

    fn vp9_encoder_config(image_format: ImageFormat) -> EncoderConfig {
        let mut config = EncoderConfig::new(
            128,
            128,
            image_format,
            CodecConfig::Vp9(Vp9Config::default()),
        );
        config.target_bitrate = 1_000_000;
        config.min_quantizer = 1;
        config.max_quantizer = 1;
        config.cq_level = 1;
        config
    }

    #[test]
    fn error_reason() {
        let e = Error::check(sys::vpx_codec_err_t_VPX_CODEC_MEM_ERROR, "test", None)
            .expect_err("not an error");
        assert!(e.reason().is_some());
    }
}
