//! CPU frame layout, shared by construction and copies. Sizes are Web IDL u32s.

use super::super::dom_rect::DomRectInit;
use crate::webidl;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, webidl::WebIdlEnum,
)]
#[webidl(name = "VideoPixelFormat", rename_all = "none")]
pub(crate) enum PixelFormat {
    I420,
    #[webidl(token = "I420P10")]
    I420p10,
    #[webidl(token = "I420P12")]
    I420p12,
    I420A,
    #[webidl(token = "I420AP10")]
    I420Ap10,
    #[webidl(token = "I420AP12")]
    I420Ap12,
    I422,
    #[webidl(token = "I422P10")]
    I422p10,
    #[webidl(token = "I422P12")]
    I422p12,
    I422A,
    #[webidl(token = "I422AP10")]
    I422Ap10,
    #[webidl(token = "I422AP12")]
    I422Ap12,
    I444,
    #[webidl(token = "I444P10")]
    I444p10,
    #[webidl(token = "I444P12")]
    I444p12,
    I444A,
    #[webidl(token = "I444AP10")]
    I444Ap10,
    #[webidl(token = "I444AP12")]
    I444Ap12,
    NV12,
    #[webidl(token = "RGBA")]
    Rgba,
    #[webidl(token = "RGBX")]
    Rgbx,
    #[webidl(token = "BGRA")]
    Bgra,
    #[webidl(token = "BGRX")]
    Bgrx,
}

impl PixelFormat {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::I420 => "I420",
            Self::I420p10 => "I420P10",
            Self::I420p12 => "I420P12",
            Self::I420A => "I420A",
            Self::I420Ap10 => "I420AP10",
            Self::I420Ap12 => "I420AP12",
            Self::I422 => "I422",
            Self::I422p10 => "I422P10",
            Self::I422p12 => "I422P12",
            Self::I422A => "I422A",
            Self::I422Ap10 => "I422AP10",
            Self::I422Ap12 => "I422AP12",
            Self::I444 => "I444",
            Self::I444p10 => "I444P10",
            Self::I444p12 => "I444P12",
            Self::I444A => "I444A",
            Self::I444Ap10 => "I444AP10",
            Self::I444Ap12 => "I444AP12",
            Self::NV12 => "NV12",
            Self::Rgba => "RGBA",
            Self::Rgbx => "RGBX",
            Self::Bgra => "BGRA",
            Self::Bgrx => "BGRX",
        }
    }

    pub(super) fn rgb(self) -> bool {
        matches!(self, Self::Rgba | Self::Rgbx | Self::Bgra | Self::Bgrx)
    }

    pub(super) fn depth(self) -> u32 {
        match self {
            Self::I420p10
            | Self::I420Ap10
            | Self::I422p10
            | Self::I422Ap10
            | Self::I444p10
            | Self::I444Ap10 => 10,
            Self::I420p12
            | Self::I420Ap12
            | Self::I422p12
            | Self::I422Ap12
            | Self::I444p12
            | Self::I444Ap12 => 12,
            _ => 8,
        }
    }

    pub(super) fn has_alpha(self) -> bool {
        matches!(
            self,
            Self::Rgba
                | Self::Bgra
                | Self::I420A
                | Self::I420Ap10
                | Self::I420Ap12
                | Self::I422A
                | Self::I422Ap10
                | Self::I422Ap12
                | Self::I444A
                | Self::I444Ap10
                | Self::I444Ap12
        )
    }

    pub(super) fn opaque(self) -> Self {
        match self {
            Self::Rgba => Self::Rgbx,
            Self::Bgra => Self::Bgrx,
            Self::I420A => Self::I420,
            Self::I420Ap10 => Self::I420p10,
            Self::I420Ap12 => Self::I420p12,
            Self::I422A => Self::I422,
            Self::I422Ap10 => Self::I422p10,
            Self::I422Ap12 => Self::I422p12,
            Self::I444A => Self::I444,
            Self::I444Ap10 => Self::I444p10,
            Self::I444Ap12 => Self::I444p12,
            _ => self,
        }
    }

    pub(super) fn planes(self) -> Vec<Plane> {
        if self.rgb() {
            return vec![Plane {
                width: 1,
                height: 1,
                bytes: 4,
            }];
        }
        let bytes = if self.depth() > 8 { 2 } else { 1 };
        let y = Plane {
            width: 1,
            height: 1,
            bytes,
        };
        let (width, height) = match self {
            Self::I420
            | Self::I420p10
            | Self::I420p12
            | Self::I420A
            | Self::I420Ap10
            | Self::I420Ap12
            | Self::NV12 => (2, 2),
            Self::I422
            | Self::I422p10
            | Self::I422p12
            | Self::I422A
            | Self::I422Ap10
            | Self::I422Ap12 => (2, 1),
            _ => (1, 1),
        };
        let chroma = Plane {
            width,
            height,
            bytes,
        };
        if self == Self::NV12 {
            return vec![y, Plane { bytes: 2, ..chroma }];
        }
        let mut planes = vec![y, chroma, chroma];
        if self.has_alpha() {
            planes.push(y);
        }
        planes
    }
}

#[derive(Clone, Copy)]
pub(super) struct Plane {
    pub width: u32,
    pub height: u32,
    pub bytes: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, webidl::WebIdlDictionary)]
#[webidl(prefix = "PlaneLayout")]
pub(crate) struct PlaneLayout {
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    pub offset: u32,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    pub stride: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub(super) fn coded(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
    pub(super) fn values(self) -> [f64; 4] {
        [
            self.x as f64,
            self.y as f64,
            self.width as f64,
            self.height as f64,
        ]
    }
    pub(super) fn parse(
        default: Self,
        override_rect: Option<DomRectInit>,
        coded: Self,
        format: PixelFormat,
    ) -> Result<Self, &'static str> {
        let rect = if let Some(r) = override_rect {
            if [r.x, r.y, r.width, r.height]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
                || r.width < 1.0
                || r.height < 1.0
                || r.x + r.width > coded.width as f64
                || r.y + r.height > coded.height as f64
            {
                return Err("The frame rectangle is empty or outside the coded size.");
            }
            if format
                .planes()
                .iter()
                .any(|p| r.x % p.width as f64 != 0.0 || r.y % p.height as f64 != 0.0)
            {
                return Err("The frame rectangle offset is not sample-aligned.");
            }
            Self {
                x: r.x as u32,
                y: r.y as u32,
                width: r.width as u32,
                height: r.height as u32,
            }
        } else {
            default
        };
        if format
            .planes()
            .iter()
            .any(|p| !rect.x.is_multiple_of(p.width) || !rect.y.is_multiple_of(p.height))
        {
            return Err("The frame rectangle is not aligned to its subsampled planes.");
        }
        Ok(rect)
    }
}

#[derive(Clone, Copy)]
pub(super) struct CopyPlane {
    pub layout: PlaneLayout,
    pub top: u32,
    pub rows: u32,
    pub left: u32,
    pub row_bytes: u32,
}

pub(super) struct Layout {
    pub planes: Vec<CopyPlane>,
    pub size: u32,
}

impl Layout {
    pub(super) fn compute(
        rect: Rect,
        format: PixelFormat,
        supplied: Option<&[PlaneLayout]>,
    ) -> Result<Self, &'static str> {
        let specs = format.planes();
        if supplied.is_some_and(|layout| layout.len() != specs.len()) {
            return Err("The layout must describe every plane.");
        }
        let mut planes: Vec<CopyPlane> = Vec::with_capacity(specs.len());
        let mut ends = Vec::with_capacity(specs.len());
        let mut size = 0;
        for (index, spec) in specs.iter().enumerate() {
            let rows = rect.height.div_ceil(spec.height);
            let row_bytes = rect
                .width
                .div_ceil(spec.width)
                .checked_mul(spec.bytes)
                .ok_or("Frame row size overflow.")?;
            let layout = supplied
                .map(|layouts| layouts[index])
                .unwrap_or(PlaneLayout {
                    offset: size,
                    stride: row_bytes,
                });
            if layout.stride < row_bytes {
                return Err("The plane stride is too small.");
            }
            let end = layout
                .stride
                .checked_mul(rows)
                .and_then(|n| layout.offset.checked_add(n))
                .ok_or("Frame plane size overflow.")?;
            if planes
                .iter()
                .zip(&ends)
                .any(|(p, end_before)| layout.offset < *end_before && p.layout.offset < end)
            {
                return Err("Frame planes overlap.");
            }
            size = size.max(end);
            ends.push(end);
            planes.push(CopyPlane {
                layout,
                top: rect.y / spec.height,
                rows,
                left: (rect.x / spec.width)
                    .checked_mul(spec.bytes)
                    .ok_or("Frame plane offset overflow.")?,
                row_bytes,
            });
        }
        Ok(Self { planes, size })
    }
}

pub(super) fn orientation(
    base_rotation: u32,
    base_flip: bool,
    rotation: f64,
    flip: bool,
) -> (u32, bool) {
    // Reduce first so even a finite double near f64::MAX cannot overflow.
    let delta = ((rotation % 360.0) / 90.0 + 0.5).floor() as i32 * 90;
    let rotation =
        (base_rotation as i32 + if base_flip { -delta } else { delta }).rem_euclid(360) as u32;
    (rotation, base_flip ^ flip)
}
