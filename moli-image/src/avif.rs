//! Still AVIF images: container parsing, AV1 decode, and YUV to RGBA8.
//!
//! `avif-parse` extracts the primary (and optional alpha auxiliary) AV1
//! payloads, `rav1d` decodes them, and the `yuv` crate converts the decoded
//! planes into the crate's straight-alpha RGBA8 contract.
//!
//! Animated (`avis`) files, grid collages, HDR transfer functions,
//! premultiplied alpha, and high bit depth monochrome/identity/YCgCo pictures
//! (which `yuv` cannot convert to RGBA8) are outside this contract.
//! `irot`/`imir`/`clap` item transforms are not applied because the container
//! parser does not expose them.

use std::ffi::{c_int, c_uint};
use std::fmt::Debug;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use avif_parse::{AV1Metadata, AvifData};
use rav1d::Dav1dResult;
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_COLOR_PRI_BT709, DAV1D_COLOR_PRI_BT2020, DAV1D_MC_BT709, DAV1D_MC_BT2020_CL,
    DAV1D_MC_BT2020_NCL, DAV1D_MC_CHROMAT_NCL, DAV1D_MC_FCC, DAV1D_MC_IDENTITY,
    DAV1D_MC_SMPTE_YCGCO, DAV1D_MC_SMPTE240, DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420,
    DAV1D_PIXEL_LAYOUT_I422, DAV1D_PIXEL_LAYOUT_I444, Dav1dColorPrimaries, Dav1dMatrixCoefficients,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib as dav1d;
use yuv::{
    YuvChromaSubsampling, YuvError, YuvGrayAlphaImage, YuvGrayImage, YuvPlanarImage,
    YuvPlanarImageWithAlpha, YuvRange, YuvStandardMatrix,
};

use crate::{MAX_DECODED_RGBA_BYTES, RgbaImage, RgbaImageError};

/// Upper bound on decoder iterations for one still frame. A conforming still
/// image needs one send and one receive; the bound only stops a decoder that
/// keeps reporting `EAGAIN` without consuming input.
const MAX_DECODE_STEPS: usize = 64;

/// Largest frame area the AV1 decoder may allocate, matching the RGBA8 budget.
const MAX_DECODED_PIXELS: usize = MAX_DECODED_RGBA_BYTES / 4;

#[derive(Debug, thiserror::Error)]
pub enum AvifDecodeError {
    #[error("AVIF container could not be parsed: {0}")]
    Container(#[source] avif_parse::Error),
    #[error("AV1 sequence header could not be parsed: {0}")]
    SequenceHeader(#[source] avif_parse::Error),
    #[error("AV1 payload is empty")]
    EmptyPayload,
    #[error("AV1 decoder failed during {operation} with code {code}")]
    Decoder {
        operation: &'static str,
        code: c_int,
    },
    #[error("AV1 decoder produced no picture")]
    MissingPicture,
    #[error("AV1 decoder produced an unsupported picture: {0}")]
    UnsupportedPicture(&'static str),
    #[error(
        "AVIF alpha plane {alpha_width}x{alpha_height} does not match color plane {width}x{height}"
    )]
    AlphaDimensionMismatch {
        width: u32,
        height: u32,
        alpha_width: u32,
        alpha_height: u32,
    },
    #[error("YUV to RGBA conversion failed: {0}")]
    Conversion(#[source] YuvError),
    #[error(transparent)]
    InvalidOutput(#[from] RgbaImageError),
}

/// Reads AVIF dimensions from the AV1 sequence header without decoding pixels.
///
/// Pictures that [`decode_avif`] always rejects are rejected here too, as far
/// as the sequence-header fields exposed by `avif-parse` reveal (the color
/// matrix is not among them), so a probe does not admit an image whose pixels
/// can never be produced.
pub(crate) fn avif_dimensions(bytes: &[u8]) -> Result<(u32, u32), AvifDecodeError> {
    let metadata = check_container(&parse_container(bytes)?)?;
    Ok((
        metadata.max_frame_width.get(),
        metadata.max_frame_height.get(),
    ))
}

/// Decodes the primary AVIF item (plus its alpha auxiliary, if any) into
/// straight-alpha RGBA8.
pub fn decode_avif(bytes: &[u8]) -> Result<RgbaImage, AvifDecodeError> {
    let data = parse_container(bytes)?;
    // Reject what the container already reveals before paying for AV1 decode.
    check_container(&data)?;
    let color = decode_av1_still(&data.primary_item)?;
    let color = color.picture()?;
    let alpha = data
        .alpha_item
        .as_deref()
        .map(decode_av1_still)
        .transpose()?;
    let alpha = alpha.as_ref().map(DecodedPicture::picture).transpose()?;
    if let Some(alpha) = &alpha
        && (alpha.width, alpha.height) != (color.width, color.height)
    {
        return Err(AvifDecodeError::AlphaDimensionMismatch {
            width: color.width,
            height: color.height,
            alpha_width: alpha.width,
            alpha_height: alpha.height,
        });
    }
    check_supported(color.traits(alpha.as_ref(), data.premultiplied_alpha))?;
    let rgba = yuv_to_rgba(&color, alpha.as_ref())?;
    Ok(RgbaImage::try_new(color.width, color.height, rgba)?)
}

fn parse_container(bytes: &[u8]) -> Result<AvifData, AvifDecodeError> {
    check_box_sizes(bytes)?;
    avif_parse::read_avif(&mut &*bytes).map_err(AvifDecodeError::Container)
}

/// Applies [`check_supported`] to what the sequence headers reveal and returns
/// the primary item's metadata.
fn check_container(data: &AvifData) -> Result<AV1Metadata, AvifDecodeError> {
    let color = data
        .primary_item_metadata()
        .map_err(AvifDecodeError::SequenceHeader)?;
    let alpha = data
        .alpha_item_metadata()
        .map_err(AvifDecodeError::SequenceHeader)?;
    check_supported(PictureTraits {
        bit_depth: color.bit_depth,
        monochrome: color.monochrome,
        // AV1 signals monochrome as 4:2:0, so only color pictures count.
        subsampled: !color.monochrome && (color.chroma_subsampling.0 || color.chroma_subsampling.1),
        model: None,
        alpha_bit_depth: alpha.map(|alpha| alpha.bit_depth),
        premultiplied_alpha: data.premultiplied_alpha,
    })?;
    Ok(color)
}

/// Rejects boxes whose declared size exceeds the bytes that contain them.
///
/// `avif-parse` reserves the `mdat`, `iloc`, and `auxC` buffers from the
/// declared box size before reading, so without this check a tiny file could
/// request gigabytes regardless of the encoded-input budget. Only the
/// containers on the path to those boxes are descended into; malformed
/// headers are left for `avif-parse` to report.
fn check_box_sizes(bytes: &[u8]) -> Result<(), AvifDecodeError> {
    fn walk(mut bytes: &[u8], depth: u8) -> Result<(), AvifDecodeError> {
        while bytes.len() >= 8 {
            let size32 = u32::from_be_bytes(bytes[..4].try_into().expect("4 bytes"));
            let name = &bytes[4..8];
            let (size, header) = match size32 {
                // "Extends to end of input"; `avif-parse` rejects it anyway.
                0 => return Ok(()),
                1 => {
                    let Some(size64) = bytes.get(8..16) else {
                        return Ok(());
                    };
                    (u64::from_be_bytes(size64.try_into().expect("8 bytes")), 16)
                }
                size => (u64::from(size), 8),
            };
            if size > bytes.len() as u64 {
                return Err(AvifDecodeError::Container(avif_parse::Error::InvalidData(
                    "box size exceeds its container",
                )));
            }
            let size = size as usize;
            if size < header {
                return Ok(());
            }
            let content = &bytes[header..size];
            let children = match name {
                // `meta` is a full box: skip its version and flags.
                b"meta" => content.get(4..),
                b"iprp" | b"ipco" => Some(content),
                _ => None,
            };
            if let Some(children) = children
                && depth < 4
            {
                walk(children, depth + 1)?;
            }
            bytes = &bytes[size..];
        }
        Ok(())
    }
    walk(bytes, 0)
}

// ---------------------------------------------------------------------------
// Support rules
// ---------------------------------------------------------------------------

/// Properties that decide whether a picture can be converted to RGBA8.
#[derive(Clone, Copy, Debug)]
struct PictureTraits {
    bit_depth: u8,
    monochrome: bool,
    subsampled: bool,
    /// `None` when only container metadata is available (probe).
    model: Option<ColorModel>,
    alpha_bit_depth: Option<u8>,
    premultiplied_alpha: bool,
}

/// The single source of the conversions this module refuses.
fn check_supported(traits: PictureTraits) -> Result<(), AvifDecodeError> {
    let unsupported = |reason| Err(AvifDecodeError::UnsupportedPicture(reason));
    if let Some(alpha_bit_depth) = traits.alpha_bit_depth {
        // `yuv` can premultiply but not reverse premultiplication, and the
        // RGBA8 contract is straight alpha.
        if traits.premultiplied_alpha {
            return unsupported("premultiplied alpha");
        }
        if alpha_bit_depth != traits.bit_depth {
            return unsupported("alpha bit depth differs from color");
        }
    }
    // `yuv` converts these from high bit depth only into 10/12-bit RGBA.
    let high_bit_depth = traits.bit_depth > 8;
    if high_bit_depth && traits.monochrome {
        return unsupported("high bit depth monochrome");
    }
    match traits.model {
        Some(ColorModel::Identity | ColorModel::YCgCo) if high_bit_depth => {
            unsupported("high bit depth identity or YCgCo matrix")
        }
        Some(ColorModel::Identity) if traits.subsampled => {
            unsupported("identity matrix with subsampled chroma")
        }
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// rav1d (dav1d C API) ownership wrappers
// ---------------------------------------------------------------------------

/// Maps a dav1d result to `Ok(false)` on success and `Ok(true)` when dav1d
/// asks to be called again (`EAGAIN`).
fn pending(operation: &'static str, result: Dav1dResult) -> Result<bool, AvifDecodeError> {
    match result.0 {
        0 => Ok(false),
        code if code == -libc::EAGAIN => Ok(true),
        code => Err(AvifDecodeError::Decoder { operation, code }),
    }
}

/// An open single-threaded decoder context, closed on drop.
struct Decoder {
    context: Option<Dav1dContext>,
}

impl Decoder {
    fn open() -> Result<Self, AvifDecodeError> {
        let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
        // SAFETY: `dav1d_default_settings` fully initializes the pointee and
        // does not read the previous (uninitialized) contents.
        let mut settings = unsafe {
            dav1d::dav1d_default_settings(NonNull::from(&mut settings).cast());
            settings.assume_init()
        };
        // Still images decode synchronously on the caller's thread; resource
        // owners already schedule decode work.
        settings.n_threads = 1;
        settings.max_frame_delay = 1;
        // Output only the highest spatial layer, as AVIF readers do.
        settings.all_layers = 0;
        settings.frame_size_limit = c_uint::try_from(MAX_DECODED_PIXELS).unwrap_or(c_uint::MAX);
        // The default logger writes to stderr, which would corrupt CLI output
        // streams and leak decoder noise. `Dav1dLogger` is a `repr(C)` pair of
        // nullable pointers whose all-zero value means "no logger".
        // SAFETY: `settings.logger` is a valid, exclusively borrowed
        // `Dav1dLogger`; writing zeroes produces `{ cookie: None, callback: None }`.
        unsafe { std::ptr::write_bytes(&raw mut settings.logger, 0, 1) };

        let mut context = None;
        // SAFETY: both pointers reference live, exclusively borrowed locals.
        let result = unsafe {
            dav1d::dav1d_open(
                Some(NonNull::from(&mut context)),
                Some(NonNull::from(&mut settings)),
            )
        };
        if pending("open", result)? || context.is_none() {
            return Err(AvifDecodeError::Decoder {
                operation: "open",
                code: result.0,
            });
        }
        Ok(Self { context })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: `context` came from `dav1d_open` and is closed exactly once;
        // `dav1d_close` resets it to `None`.
        unsafe { dav1d::dav1d_close(Some(NonNull::from(&mut self.context))) };
    }
}

/// Decoder-owned input buffer, released on drop.
struct InputData(Dav1dData);

impl InputData {
    fn copy_from(bytes: &[u8]) -> Result<Self, AvifDecodeError> {
        if bytes.is_empty() {
            return Err(AvifDecodeError::EmptyPayload);
        }
        let mut data = Self(Dav1dData::default());
        // SAFETY: `data.0` is a valid default `Dav1dData` to write into.
        let buffer =
            unsafe { dav1d::dav1d_data_create(Some(NonNull::from(&mut data.0)), bytes.len()) };
        if buffer.is_null() {
            return Err(AvifDecodeError::Decoder {
                operation: "data_create",
                code: -libc::ENOMEM,
            });
        }
        // SAFETY: `dav1d_data_create` returned a writable allocation of
        // exactly `bytes.len()` bytes that does not alias `bytes`.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len()) };
        Ok(data)
    }
}

impl Drop for InputData {
    fn drop(&mut self) {
        // SAFETY: `self.0` is either default or created by `dav1d_data_create`
        // and possibly partially consumed by `dav1d_send_data`.
        unsafe { dav1d::dav1d_data_unref(Some(NonNull::from(&mut self.0))) };
    }
}

/// A decoded picture reference, released on drop.
struct DecodedPicture(Dav1dPicture);

impl Drop for DecodedPicture {
    fn drop(&mut self) {
        // SAFETY: `self.0` is either default or filled by `dav1d_get_picture`.
        unsafe { dav1d::dav1d_picture_unref(Some(NonNull::from(&mut self.0))) };
    }
}

fn decode_av1_still(payload: &[u8]) -> Result<DecodedPicture, AvifDecodeError> {
    let decoder = Decoder::open()?;
    let mut data = InputData::copy_from(payload)?;
    let mut picture = DecodedPicture(Dav1dPicture::default());
    for _ in 0..MAX_DECODE_STEPS {
        if data.0.sz > 0 {
            // SAFETY: `decoder.context` is open and `data.0` is a live
            // decoder-owned buffer.
            let result = unsafe {
                dav1d::dav1d_send_data(decoder.context, Some(NonNull::from(&mut data.0)))
            };
            pending("send_data", result)?;
        }
        // SAFETY: `decoder.context` is open and `picture.0` is a default
        // (unreferenced) picture that may be overwritten.
        let result = unsafe {
            dav1d::dav1d_get_picture(decoder.context, Some(NonNull::from(&mut picture.0)))
        };
        if !pending("get_picture", result)? {
            return Ok(picture);
        }
    }
    Err(AvifDecodeError::MissingPicture)
}

// ---------------------------------------------------------------------------
// Plane access
// ---------------------------------------------------------------------------

/// Plane sample types: plain integers, valid for any bit pattern.
trait Sample: Copy + Debug {}
impl Sample for u8 {}
impl Sample for u16 {}

#[derive(Clone, Copy)]
struct Plane<'a, T> {
    samples: &'a [T],
    /// Row stride in samples (not bytes).
    stride: u32,
}

#[derive(Clone, Copy)]
struct Planes<'a, T> {
    y: Plane<'a, T>,
    /// Chroma layout and U/V planes; absent for monochrome pictures.
    uv: Option<(YuvChromaSubsampling, [Plane<'a, T>; 2])>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HighBitDepth {
    Ten,
    Twelve,
}

/// Planes in the element type `yuv` expects for the bit depth.
enum Samples<'a> {
    Eight(Planes<'a, u8>),
    High(HighBitDepth, Planes<'a, u16>),
}

struct Picture<'a> {
    width: u32,
    height: u32,
    full_range: bool,
    model: ColorModel,
    samples: Samples<'a>,
}

impl Picture<'_> {
    fn bit_depth(&self) -> u8 {
        match self.samples {
            Samples::Eight(_) => 8,
            Samples::High(HighBitDepth::Ten, _) => 10,
            Samples::High(HighBitDepth::Twelve, _) => 12,
        }
    }

    fn subsampling(&self) -> Option<YuvChromaSubsampling> {
        match &self.samples {
            Samples::Eight(planes) => planes.uv.map(|(subsampling, _)| subsampling),
            Samples::High(_, planes) => planes.uv.map(|(subsampling, _)| subsampling),
        }
    }

    fn traits(&self, alpha: Option<&Picture<'_>>, premultiplied_alpha: bool) -> PictureTraits {
        let subsampling = self.subsampling();
        PictureTraits {
            bit_depth: self.bit_depth(),
            monochrome: subsampling.is_none(),
            subsampled: matches!(
                subsampling,
                Some(YuvChromaSubsampling::Yuv420 | YuvChromaSubsampling::Yuv422)
            ),
            model: Some(self.model),
            alpha_bit_depth: alpha.map(Picture::bit_depth),
            premultiplied_alpha,
        }
    }
}

impl DecodedPicture {
    fn picture(&self) -> Result<Picture<'_>, AvifDecodeError> {
        let picture = &self.0;
        let width = u32::try_from(picture.p.w)
            .ok()
            .filter(|width| *width > 0)
            .ok_or(AvifDecodeError::UnsupportedPicture("non-positive width"))?;
        let height = u32::try_from(picture.p.h)
            .ok()
            .filter(|height| *height > 0)
            .ok_or(AvifDecodeError::UnsupportedPicture("non-positive height"))?;
        let subsampling = match picture.p.layout {
            DAV1D_PIXEL_LAYOUT_I400 => None,
            DAV1D_PIXEL_LAYOUT_I420 => Some(YuvChromaSubsampling::Yuv420),
            DAV1D_PIXEL_LAYOUT_I422 => Some(YuvChromaSubsampling::Yuv422),
            DAV1D_PIXEL_LAYOUT_I444 => Some(YuvChromaSubsampling::Yuv444),
            _ => return Err(AvifDecodeError::UnsupportedPicture("pixel layout")),
        };
        let samples = match picture.p.bpc {
            8 => Samples::Eight(self.planes(width, height, subsampling)?),
            10 => Samples::High(HighBitDepth::Ten, self.planes(width, height, subsampling)?),
            12 => Samples::High(
                HighBitDepth::Twelve,
                self.planes(width, height, subsampling)?,
            ),
            _ => return Err(AvifDecodeError::UnsupportedPicture("bit depth")),
        };
        let sequence = picture.seq_hdr.ok_or(AvifDecodeError::UnsupportedPicture(
            "missing sequence header",
        ))?;
        // SAFETY: the sequence header is reference-counted by the picture and
        // stays alive until `self` is dropped.
        let sequence = unsafe { sequence.as_ref() };
        Ok(Picture {
            width,
            height,
            full_range: sequence.color_range != 0,
            model: ColorModel::from_cicp(sequence.mtrx, sequence.pri),
            samples,
        })
    }

    fn planes<T: Sample>(
        &self,
        width: u32,
        height: u32,
        subsampling: Option<YuvChromaSubsampling>,
    ) -> Result<Planes<'_, T>, AvifDecodeError> {
        let (width, height) = (width as usize, height as usize);
        let uv = match subsampling {
            None => None,
            Some(subsampling) => {
                let (chroma_width, chroma_height) = match subsampling {
                    YuvChromaSubsampling::Yuv420 => (width.div_ceil(2), height.div_ceil(2)),
                    YuvChromaSubsampling::Yuv422 => (width.div_ceil(2), height),
                    YuvChromaSubsampling::Yuv444 => (width, height),
                };
                Some((
                    subsampling,
                    [
                        self.plane(1, chroma_width, chroma_height)?,
                        self.plane(2, chroma_width, chroma_height)?,
                    ],
                ))
            }
        };
        Ok(Planes {
            y: self.plane(0, width, height)?,
            uv,
        })
    }

    fn plane<T: Sample>(
        &self,
        index: usize,
        width: usize,
        height: usize,
    ) -> Result<Plane<'_, T>, AvifDecodeError> {
        let picture = &self.0;
        let sample_bytes = size_of::<T>();
        let stride = usize::try_from(picture.stride[usize::from(index > 0)])
            .map_err(|_| AvifDecodeError::UnsupportedPicture("negative plane stride"))?;
        let row_bytes = width
            .checked_mul(sample_bytes)
            .ok_or(AvifDecodeError::UnsupportedPicture("plane row overflow"))?;
        if stride < row_bytes || stride % sample_bytes != 0 {
            return Err(AvifDecodeError::UnsupportedPicture("invalid plane stride"));
        }
        let len = stride
            .checked_mul(height - 1)
            .and_then(|len| len.checked_add(row_bytes))
            .ok_or(AvifDecodeError::UnsupportedPicture("plane size overflow"))?;
        let data = picture.data[index]
            .ok_or(AvifDecodeError::UnsupportedPicture("missing plane data"))?
            .cast::<T>();
        if !data.as_ptr().is_aligned() {
            return Err(AvifDecodeError::UnsupportedPicture("misaligned plane"));
        }
        // SAFETY: dav1d allocates each plane with `stride` bytes per row for
        // at least `height` rows of `width` samples, so `len` bytes are in
        // bounds; `len` is a multiple of the sample size because `stride` and
        // `row_bytes` are, and the pointer is aligned for `T`, whose every bit
        // pattern is valid (`Sample`). The buffer is kept alive by the picture
        // reference held by `self` and is never written after
        // `dav1d_get_picture` returns it.
        let samples = unsafe { std::slice::from_raw_parts(data.as_ptr(), len / sample_bytes) };
        Ok(Plane {
            samples,
            stride: u32::try_from(stride / sample_bytes)
                .map_err(|_| AvifDecodeError::UnsupportedPicture("plane stride overflow"))?,
        })
    }
}

// ---------------------------------------------------------------------------
// YUV -> RGBA8 (delegated to the `yuv` crate)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum ColorModel {
    YCbCr(YuvStandardMatrix),
    /// GBR stored as Y=G, U=B, V=R.
    Identity,
    YCgCo,
}

impl ColorModel {
    fn from_cicp(matrix: Dav1dMatrixCoefficients, primaries: Dav1dColorPrimaries) -> Self {
        match matrix {
            DAV1D_MC_IDENTITY => Self::Identity,
            DAV1D_MC_SMPTE_YCGCO => Self::YCgCo,
            DAV1D_MC_BT709 => Self::YCbCr(YuvStandardMatrix::Bt709),
            DAV1D_MC_FCC => Self::YCbCr(YuvStandardMatrix::Fcc),
            DAV1D_MC_SMPTE240 => Self::YCbCr(YuvStandardMatrix::Smpte240),
            // The constant-luminance variant is approximated by its
            // non-constant counterpart.
            DAV1D_MC_BT2020_NCL | DAV1D_MC_BT2020_CL => Self::YCbCr(YuvStandardMatrix::Bt2020),
            DAV1D_MC_CHROMAT_NCL => Self::YCbCr(match primaries {
                DAV1D_COLOR_PRI_BT709 => YuvStandardMatrix::Bt709,
                DAV1D_COLOR_PRI_BT2020 => YuvStandardMatrix::Bt2020,
                _ => YuvStandardMatrix::Bt601,
            }),
            // BT.601 (and BT.470BG), unspecified, and matrices without a
            // linear Y'CbCr form fall back to BT.601 like other browsers.
            _ => Self::YCbCr(YuvStandardMatrix::Bt601),
        }
    }
}

/// A `yuv` planar-to-RGBA8 converter. Converters without a matrix parameter
/// are adapted to ignore it.
type ToRgba<T> =
    fn(&YuvPlanarImage<'_, T>, &mut [u8], u32, YuvRange, YuvStandardMatrix) -> Result<(), YuvError>;
type AlphaToRgba<T> = fn(
    &YuvPlanarImageWithAlpha<'_, T>,
    &mut [u8],
    u32,
    YuvRange,
    YuvStandardMatrix,
) -> Result<(), YuvError>;

/// 8-bit converters; `None` for combinations [`check_supported`] rejects.
fn eight_bit(model: ColorModel, subsampling: YuvChromaSubsampling) -> Option<ToRgba<u8>> {
    use ColorModel::{Identity, YCbCr, YCgCo};
    use YuvChromaSubsampling::{Yuv420, Yuv422, Yuv444};
    let convert: ToRgba<u8> = match (model, subsampling) {
        (YCbCr(_), Yuv420) => yuv::yuv420_to_rgba,
        (YCbCr(_), Yuv422) => yuv::yuv422_to_rgba,
        (YCbCr(_), Yuv444) => yuv::yuv444_to_rgba,
        (YCgCo, Yuv420) => {
            |image, dst, stride, range, _| yuv::ycgco420_to_rgba(image, dst, stride, range)
        }
        (YCgCo, Yuv422) => {
            |image, dst, stride, range, _| yuv::ycgco422_to_rgba(image, dst, stride, range)
        }
        (YCgCo, Yuv444) => {
            |image, dst, stride, range, _| yuv::ycgco444_to_rgba(image, dst, stride, range)
        }
        (Identity, Yuv444) => {
            |image, dst, stride, range, _| yuv::gbr_to_rgba(image, dst, stride, range)
        }
        (Identity, Yuv420 | Yuv422) => return None,
    };
    Some(convert)
}

/// 8-bit converters with alpha; `None` for combinations [`check_supported`]
/// rejects.
fn eight_bit_alpha(
    model: ColorModel,
    subsampling: YuvChromaSubsampling,
) -> Option<AlphaToRgba<u8>> {
    use ColorModel::{Identity, YCbCr, YCgCo};
    use YuvChromaSubsampling::{Yuv420, Yuv422, Yuv444};
    let convert: AlphaToRgba<u8> = match (model, subsampling) {
        (YCbCr(_), Yuv420) => |image, dst, stride, range, matrix| {
            yuv::yuv420_alpha_to_rgba(image, dst, stride, range, matrix, false)
        },
        (YCbCr(_), Yuv422) => |image, dst, stride, range, matrix| {
            yuv::yuv422_alpha_to_rgba(image, dst, stride, range, matrix, false)
        },
        (YCbCr(_), Yuv444) => |image, dst, stride, range, matrix| {
            yuv::yuv444_alpha_to_rgba(image, dst, stride, range, matrix, false)
        },
        (YCgCo, Yuv420) => {
            |image, dst, stride, range, _| yuv::ycgco420_alpha_to_rgba(image, dst, stride, range)
        }
        (YCgCo, Yuv422) => {
            |image, dst, stride, range, _| yuv::ycgco422_alpha_to_rgba(image, dst, stride, range)
        }
        (YCgCo, Yuv444) => {
            |image, dst, stride, range, _| yuv::ycgco444_alpha_to_rgba(image, dst, stride, range)
        }
        (Identity, Yuv444) => {
            |image, dst, stride, range, _| yuv::gbr_with_alpha_to_rgba(image, dst, stride, range)
        }
        (Identity, Yuv420 | Yuv422) => return None,
    };
    Some(convert)
}

/// 10/12-bit Y'CbCr converters; `None` for models [`check_supported`] rejects.
fn high_bit(
    model: ColorModel,
    depth: HighBitDepth,
    subsampling: YuvChromaSubsampling,
) -> Option<ToRgba<u16>> {
    use HighBitDepth::{Ten, Twelve};
    use YuvChromaSubsampling::{Yuv420, Yuv422, Yuv444};
    let ColorModel::YCbCr(_) = model else {
        return None;
    };
    Some(match (depth, subsampling) {
        (Ten, Yuv420) => yuv::i010_to_rgba,
        (Ten, Yuv422) => yuv::i210_to_rgba,
        (Ten, Yuv444) => yuv::i410_to_rgba,
        (Twelve, Yuv420) => yuv::i012_to_rgba,
        (Twelve, Yuv422) => yuv::i212_to_rgba,
        (Twelve, Yuv444) => yuv::i412_to_rgba,
    })
}

/// 10/12-bit Y'CbCr converters with alpha; `None` for models
/// [`check_supported`] rejects.
fn high_bit_alpha(
    model: ColorModel,
    depth: HighBitDepth,
    subsampling: YuvChromaSubsampling,
) -> Option<AlphaToRgba<u16>> {
    use HighBitDepth::{Ten, Twelve};
    use YuvChromaSubsampling::{Yuv420, Yuv422, Yuv444};
    let ColorModel::YCbCr(_) = model else {
        return None;
    };
    Some(match (depth, subsampling) {
        (Ten, Yuv420) => yuv::i010_alpha_to_rgba,
        (Ten, Yuv422) => yuv::i210_alpha_to_rgba,
        (Ten, Yuv444) => yuv::i410_alpha_to_rgba,
        (Twelve, Yuv420) => yuv::i012_alpha_to_rgba,
        (Twelve, Yuv422) => yuv::i212_alpha_to_rgba,
        (Twelve, Yuv444) => yuv::i412_alpha_to_rgba,
    })
}

fn planar<'a, T: Sample>(
    y: Plane<'a, T>,
    [u, v]: [Plane<'a, T>; 2],
    width: u32,
    height: u32,
) -> YuvPlanarImage<'a, T> {
    YuvPlanarImage {
        y_plane: y.samples,
        y_stride: y.stride,
        u_plane: u.samples,
        u_stride: u.stride,
        v_plane: v.samples,
        v_stride: v.stride,
        width,
        height,
    }
}

fn with_alpha<'a, T: Sample>(
    image: YuvPlanarImage<'a, T>,
    alpha: Plane<'a, T>,
) -> YuvPlanarImageWithAlpha<'a, T> {
    YuvPlanarImageWithAlpha {
        y_plane: image.y_plane,
        y_stride: image.y_stride,
        u_plane: image.u_plane,
        u_stride: image.u_stride,
        v_plane: image.v_plane,
        v_stride: image.v_stride,
        a_plane: alpha.samples,
        a_stride: alpha.stride,
        width: image.width,
        height: image.height,
    }
}

/// Converts a picture that passed [`check_supported`] into RGBA8.
fn yuv_to_rgba(
    color: &Picture<'_>,
    alpha: Option<&Picture<'_>>,
) -> Result<Vec<u8>, AvifDecodeError> {
    let (width, height) = (color.width, color.height);
    let mut rgba = vec![0_u8; crate::rgba::checked_rgba_len(width, height)?];
    // `checked_rgba_len` bounds `width * height * 4` by the decoded-image
    // budget, so the row stride fits in `u32`.
    let stride = width * 4;
    // The alpha auxiliary item is read as full range, as `yuv` defines it.
    let range = if color.full_range {
        YuvRange::Full
    } else {
        YuvRange::Limited
    };
    // Only Y'CbCr converters use the matrix; the others ignore it.
    let matrix = match color.model {
        ColorModel::YCbCr(matrix) => matrix,
        ColorModel::Identity | ColorModel::YCgCo => YuvStandardMatrix::Bt601,
    };
    let unsupported = || AvifDecodeError::UnsupportedPicture("color model and chroma layout");
    let dst = rgba.as_mut_slice();
    let result = match (&color.samples, alpha.map(|alpha| &alpha.samples)) {
        (Samples::Eight(planes), None) => match planes.uv {
            None => yuv::yuv400_to_rgba(
                &YuvGrayImage {
                    y_plane: planes.y.samples,
                    y_stride: planes.y.stride,
                    width,
                    height,
                },
                dst,
                stride,
                range,
                matrix,
            ),
            Some((subsampling, uv)) => eight_bit(color.model, subsampling)
                .ok_or_else(unsupported)?(
                &planar(planes.y, uv, width, height),
                dst,
                stride,
                range,
                matrix,
            ),
        },
        (Samples::Eight(planes), Some(Samples::Eight(alpha))) => match planes.uv {
            None => yuv::yuv400_alpha_to_rgba(
                &YuvGrayAlphaImage {
                    y_plane: planes.y.samples,
                    y_stride: planes.y.stride,
                    a_plane: alpha.y.samples,
                    a_stride: alpha.y.stride,
                    width,
                    height,
                },
                dst,
                stride,
                range,
                matrix,
            ),
            Some((subsampling, uv)) => eight_bit_alpha(color.model, subsampling)
                .ok_or_else(unsupported)?(
                &with_alpha(planar(planes.y, uv, width, height), alpha.y),
                dst,
                stride,
                range,
                matrix,
            ),
        },
        (Samples::High(depth, planes), None) => {
            let (subsampling, uv) = planes.uv.ok_or_else(unsupported)?;
            high_bit(color.model, *depth, subsampling).ok_or_else(unsupported)?(
                &planar(planes.y, uv, width, height),
                dst,
                stride,
                range,
                matrix,
            )
        }
        (Samples::High(depth, planes), Some(Samples::High(alpha_depth, alpha)))
            if depth == alpha_depth =>
        {
            let (subsampling, uv) = planes.uv.ok_or_else(unsupported)?;
            high_bit_alpha(color.model, *depth, subsampling).ok_or_else(unsupported)?(
                &with_alpha(planar(planes.y, uv, width, height), alpha.y),
                dst,
                stride,
                range,
                matrix,
            )
        }
        _ => {
            return Err(AvifDecodeError::UnsupportedPicture(
                "alpha bit depth differs from color",
            ));
        }
    };
    result.map_err(AvifDecodeError::Conversion)?;
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YUV420_LIMITED_BT709: &[u8] =
        include_bytes!("../tests/fixtures/avif/yuv420-limited-bt709.avif");
    const YUV444_FULL_BT601: &[u8] =
        include_bytes!("../tests/fixtures/avif/yuv444-full-bt601.avif");
    const YUV420_10BIT: &[u8] = include_bytes!("../tests/fixtures/avif/yuv420-10bit.avif");
    const ALPHA_YUV444: &[u8] = include_bytes!("../tests/fixtures/avif/alpha-yuv444.avif");

    /// Expected straight-alpha RGBA for the 8x8 quadrant fixture.
    fn quadrant(x: u32, y: u32, with_alpha: bool) -> [u8; 4] {
        match (x >= 4, y >= 4) {
            (false, false) => [255, 0, 0, 255],
            (true, false) => [0, 255, 0, 255],
            (false, true) => [0, 0, 255, 255],
            (true, true) => [255, 255, 255, if with_alpha { 128 } else { 255 }],
        }
    }

    /// Compares quadrant interiors only: chroma upsampling legitimately
    /// blends colors along the internal quadrant edges.
    fn assert_quadrants(image: &RgbaImage, with_alpha: bool, tolerance: u8) {
        assert_eq!((image.width, image.height), (8, 8));
        for y in [0, 1, 2, 5, 6, 7] {
            for x in [0, 1, 2, 5, 6, 7] {
                let offset = ((y * 8 + x) * 4) as usize;
                let actual = &image.rgba[offset..offset + 4];
                let expected = quadrant(x, y, with_alpha);
                for (channel, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                    assert!(
                        actual.abs_diff(expected) <= tolerance,
                        "pixel ({x},{y}) channel {channel}: got {actual:?}, expected {expected:?} in {:?}",
                        &image.rgba[offset..offset + 4]
                    );
                }
            }
        }
    }

    #[test]
    fn reads_dimensions_without_decoding() {
        for fixture in [
            YUV420_LIMITED_BT709,
            YUV444_FULL_BT601,
            YUV420_10BIT,
            ALPHA_YUV444,
        ] {
            assert_eq!(avif_dimensions(fixture).unwrap(), (8, 8));
        }
    }

    #[test]
    fn decodes_limited_range_bt709_420() {
        assert_quadrants(&decode_avif(YUV420_LIMITED_BT709).unwrap(), false, 3);
    }

    #[test]
    fn decodes_full_range_bt601_444() {
        assert_quadrants(&decode_avif(YUV444_FULL_BT601).unwrap(), false, 3);
    }

    #[test]
    fn decodes_10_bit_planes() {
        assert_quadrants(&decode_avif(YUV420_10BIT).unwrap(), false, 3);
    }

    #[test]
    fn applies_alpha_auxiliary_plane() {
        assert_quadrants(&decode_avif(ALPHA_YUV444).unwrap(), true, 3);
    }

    #[test]
    fn rejects_truncated_and_corrupted_payloads() {
        assert!(matches!(
            decode_avif(&YUV444_FULL_BT601[..40]),
            Err(AvifDecodeError::Container(_))
        ));
        assert!(decode_avif(b"\0\0\0\x18ftypavif\0\0\0\0avifmif1").is_err());

        // Keep the container intact but corrupt the tail of the AV1 payload.
        let mut corrupted = YUV444_FULL_BT601.to_vec();
        let len = corrupted.len();
        for byte in &mut corrupted[len - 24..] {
            *byte ^= 0xa5;
        }
        assert_eq!(avif_dimensions(&corrupted).unwrap(), (8, 8));
        // A corrupted payload must fail or decode to an image of the probed
        // size; it must never panic or produce a different shape.
        if let Ok(image) = decode_avif(&corrupted) {
            assert_eq!((image.width, image.height), (8, 8));
        }
    }

    #[test]
    fn rejects_boxes_larger_than_their_container() {
        const FTYP: &[u8] = b"\0\0\0\x18ftypavif\0\0\0\0avifmif1";
        // A 64-bit `mdat` size far beyond the input must not reach the
        // parser, which would reserve that many bytes up front; the same
        // applies to an `iloc` nested inside `meta`.
        for oversized in [
            b"\0\0\0\x01mdat\0\0\0\x00\xc0\0\0\0".as_slice(),
            b"\0\0\0\x14meta\0\0\0\0\xc0\0\0\0iloc",
        ] {
            assert!(matches!(
                parse_container(&[FTYP, oversized].concat()),
                Err(AvifDecodeError::Container(avif_parse::Error::InvalidData(
                    "box size exceeds its container"
                )))
            ));
        }
        for fixture in [YUV444_FULL_BT601, ALPHA_YUV444] {
            check_box_sizes(fixture).unwrap();
        }
    }

    #[test]
    fn color_model_follows_cicp_matrix() {
        assert_eq!(
            ColorModel::from_cicp(DAV1D_MC_IDENTITY, 0),
            ColorModel::Identity
        );
        assert_eq!(
            ColorModel::from_cicp(DAV1D_MC_SMPTE_YCGCO, 0),
            ColorModel::YCgCo
        );
        assert_eq!(
            ColorModel::from_cicp(DAV1D_MC_BT709, 0),
            ColorModel::YCbCr(YuvStandardMatrix::Bt709)
        );
        assert_eq!(
            ColorModel::from_cicp(DAV1D_MC_CHROMAT_NCL, DAV1D_COLOR_PRI_BT2020),
            ColorModel::YCbCr(YuvStandardMatrix::Bt2020)
        );
        // Unspecified matrices use BT.601.
        assert_eq!(
            ColorModel::from_cicp(2, 0),
            ColorModel::YCbCr(YuvStandardMatrix::Bt601)
        );
    }

    /// An 8-bit picture with tightly packed rows of `width` samples.
    fn eight_bit_picture<'a>(
        y: &'a [u8],
        uv: Option<[&'a [u8]; 2]>,
        width: u32,
        model: ColorModel,
    ) -> Picture<'a> {
        let plane = |samples| Plane {
            samples,
            stride: width,
        };
        Picture {
            width,
            height: y.len() as u32 / width,
            full_range: true,
            model,
            samples: Samples::Eight(Planes {
                y: plane(y),
                uv: uv.map(|[u, v]| (YuvChromaSubsampling::Yuv444, [plane(u), plane(v)])),
            }),
        }
    }

    #[test]
    fn identity_matrix_maps_planes_to_gbr() {
        // Y=G, U=B, V=R.
        let color = eight_bit_picture(
            &[32, 200],
            Some([&[17, 10], &[51, 20]]),
            2,
            ColorModel::Identity,
        );
        assert_eq!(
            yuv_to_rgba(&color, None).unwrap(),
            [51, 32, 17, 255, 20, 200, 10, 255]
        );
        let alpha = eight_bit_picture(&[128, 0], None, 2, ColorModel::Identity);
        assert_eq!(
            yuv_to_rgba(&color, Some(&alpha)).unwrap(),
            [51, 32, 17, 128, 20, 200, 10, 0]
        );
    }

    #[test]
    fn monochrome_pictures_expand_luma_to_gray() {
        let mut gray = eight_bit_picture(
            &[16, 235],
            None,
            2,
            ColorModel::YCbCr(YuvStandardMatrix::Bt601),
        );
        gray.full_range = false;
        assert_eq!(
            yuv_to_rgba(&gray, None).unwrap(),
            [0, 0, 0, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn mismatched_alpha_bit_depth_is_rejected_by_conversion() {
        let color = eight_bit_picture(&[32, 200], None, 2, ColorModel::Identity);
        let alpha = Picture {
            width: 2,
            height: 1,
            full_range: true,
            model: ColorModel::Identity,
            samples: Samples::High(
                HighBitDepth::Ten,
                Planes {
                    y: Plane {
                        samples: &[512, 0],
                        stride: 2,
                    },
                    uv: None,
                },
            ),
        };
        assert!(matches!(
            yuv_to_rgba(&color, Some(&alpha)),
            Err(AvifDecodeError::UnsupportedPicture(
                "alpha bit depth differs from color"
            ))
        ));
    }

    #[test]
    fn support_rules_reject_conversions_yuv_cannot_produce() {
        let supported = PictureTraits {
            bit_depth: 8,
            monochrome: false,
            subsampled: true,
            model: Some(ColorModel::YCbCr(YuvStandardMatrix::Bt709)),
            alpha_bit_depth: Some(8),
            premultiplied_alpha: false,
        };
        check_supported(supported).unwrap();
        for (traits, reason) in [
            (
                PictureTraits {
                    premultiplied_alpha: true,
                    ..supported
                },
                "premultiplied alpha",
            ),
            (
                PictureTraits {
                    alpha_bit_depth: Some(10),
                    ..supported
                },
                "alpha bit depth differs from color",
            ),
            (
                PictureTraits {
                    bit_depth: 10,
                    monochrome: true,
                    subsampled: false,
                    alpha_bit_depth: None,
                    ..supported
                },
                "high bit depth monochrome",
            ),
            (
                PictureTraits {
                    bit_depth: 10,
                    model: Some(ColorModel::YCgCo),
                    alpha_bit_depth: None,
                    ..supported
                },
                "high bit depth identity or YCgCo matrix",
            ),
            (
                PictureTraits {
                    model: Some(ColorModel::Identity),
                    ..supported
                },
                "identity matrix with subsampled chroma",
            ),
        ] {
            assert!(
                matches!(
                    check_supported(traits),
                    Err(AvifDecodeError::UnsupportedPicture(actual)) if actual == reason
                ),
                "{traits:?} should be rejected as {reason}"
            );
        }
        // Premultiplication only matters when there is an alpha plane.
        check_supported(PictureTraits {
            alpha_bit_depth: None,
            premultiplied_alpha: true,
            ..supported
        })
        .unwrap();
    }
}
