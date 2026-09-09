/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified types for SVG Path.

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::values::animated::{lists, Animate, Procedure};
use crate::values::distance::{ComputeSquaredDistance, SquaredDistance};
use crate::values::generics::basic_shape::GenericShapeCommand;
use crate::values::generics::basic_shape::{
    ArcRadii, ArcSize, ArcSweep, AxisEndPoint, AxisPosition, CommandEndPoint, ControlPoint,
    ControlReference, CoordinatePair, RelativeControlPoint,
};
use crate::values::generics::position::GenericPosition;
use crate::values::CSSFloat;
use cssparser::Parser;
use std::fmt::{self, Write};
use std::iter::{Cloned, Peekable};
use std::ops;
use std::slice;
use style_traits::values::SequenceWriter;
use style_traits::{CssWriter, ParseError, StyleParseErrorKind, ToCss};

/// Whether to allow empty string in the parser.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum AllowEmpty {
    Yes,
    No,
}

/// The SVG path data.
///
/// https://www.w3.org/TR/SVG11/paths.html#PathData
#[derive(
    Clone,
    Debug,
    Deserialize,
    MallocSizeOf,
    PartialEq,
    Serialize,
    SpecifiedValueInfo,
    ToAnimatedZero,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct SVGPathData(
    // TODO(emilio): Should probably measure this somehow only from the
    // specified values.
    #[ignore_malloc_size_of = "Arc"] pub crate::ArcSlice<PathCommand>,
);

impl SVGPathData {
    /// Get the array of PathCommand.
    #[inline]
    pub fn commands(&self) -> &[PathCommand] {
        &self.0
    }

    /// Create a normalized copy of this path by converting each relative
    /// command to an absolute command.
    pub fn normalize(&self, reduce: bool) -> Self {
        let mut state = PathTraversalState {
            subpath_start: CoordPair::new(0.0, 0.0),
            pos: CoordPair::new(0.0, 0.0),
            last_command: PathCommand::Close,
            last_control: CoordPair::new(0.0, 0.0),
        };
        let iter = self.0.iter().map(|seg| seg.normalize(&mut state, reduce));
        SVGPathData(crate::ArcSlice::from_iter(iter))
    }

    /// Parse this SVG path string with the argument that indicates whether we should allow the
    /// empty string.
    // We cannot use cssparser::Parser to parse a SVG path string because the spec wants to make
    // the SVG path string as compact as possible. (i.e. The whitespaces may be dropped.)
    // e.g. "M100 200L100 200" is a valid SVG path string. If we use tokenizer, the first ident
    // is "M100", instead of "M", and this is not correct. Therefore, we use a Peekable
    // str::Char iterator to check each character.
    //
    // css-shapes-1 says a path data string that does conform but defines an empty path is
    // invalid and causes the entire path() to be invalid, so we use allow_empty to decide
    // whether we should allow it.
    // https://drafts.csswg.org/css-shapes-1/#typedef-basic-shape
    pub fn parse<'i, 't>(
        input: &mut Parser<'i, 't>,
        allow_empty: AllowEmpty,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        let path_string = input.expect_string()?.as_ref();
        let (path, ok) = Self::parse_bytes(path_string.as_bytes());
        if !ok || (allow_empty == AllowEmpty::No && path.0.is_empty()) {
            return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }
        return Ok(path);
    }

    /// As above, but just parsing the raw byte stream.
    ///
    /// Returns the (potentially empty or partial) path, and whether the parsing was ok or we found
    /// an error. The API is a bit weird because some SVG callers require "parse until first error"
    /// behavior.
    pub fn parse_bytes(input: &[u8]) -> (Self, bool) {
        // Parse the svg path string as multiple sub-paths.
        let mut ok = true;
        let mut path_parser = PathParser::new(input);

        while skip_wsp(&mut path_parser.chars) {
            if path_parser.parse_subpath().is_err() {
                ok = false;
                break;
            }
        }

        let path = Self(crate::ArcSlice::from_iter(path_parser.path.into_iter()));
        (path, ok)
    }

    /// Serializes to the path string, potentially including quotes.
    pub fn to_css<W>(&self, dest: &mut CssWriter<W>, quote: bool) -> fmt::Result
    where
        W: fmt::Write,
    {
        if quote {
            dest.write_char('"')?;
        }
        let mut writer = SequenceWriter::new(dest, " ");
        for command in self.commands() {
            writer.write_item(|inner| command.to_css_for_svg(inner))?;
        }
        if quote {
            dest.write_char('"')?;
        }
        Ok(())
    }
}

impl ToCss for SVGPathData {
    #[inline]
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        self.to_css(dest, /* quote = */ true)
    }
}

impl Parse for SVGPathData {
    fn parse<'i, 't>(
        _context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        // Note that the EBNF allows the path data string in the d property to be empty, so we
        // don't reject empty SVG path data.
        // https://svgwg.org/svg2-draft/single-page.html#paths-PathDataBNF
        SVGPathData::parse(input, AllowEmpty::Yes)
    }
}

impl Animate for SVGPathData {
    fn animate(&self, other: &Self, procedure: Procedure) -> Result<Self, ()> {
        if self.0.len() != other.0.len() {
            return Err(());
        }

        // FIXME(emilio): This allocates three copies of the path, that's not
        // great! Specially, once we're normalized once, we don't need to
        // re-normalize again.
        let left = self.normalize(false);
        let right = other.normalize(false);

        let items: Vec<_> = lists::by_computed_value::animate(&left.0, &right.0, procedure)?;
        Ok(SVGPathData(crate::ArcSlice::from_iter(items.into_iter())))
    }
}

impl ComputeSquaredDistance for SVGPathData {
    fn compute_squared_distance(&self, other: &Self) -> Result<SquaredDistance, ()> {
        if self.0.len() != other.0.len() {
            return Err(());
        }
        let left = self.normalize(false);
        let right = other.normalize(false);
        lists::by_computed_value::squared_distance(&left.0, &right.0)
    }
}

/// A position type for SVG path coordinates (just a pair of floats).
pub type SVGPathPosition = GenericPosition<CSSFloat, CSSFloat>;

/// The SVG path command.
/// The fields of these commands are self-explanatory, so we skip the documents.
/// Note: the index of the control points, e.g. control1, control2, are mapping to the control
/// points of the Bézier curve in the spec.
///
/// https://www.w3.org/TR/SVG11/paths.html#PathData
pub type PathCommand = GenericShapeCommand<CSSFloat, SVGPathPosition, CSSFloat>;

/// For internal SVGPath normalization.
#[allow(missing_docs)]
struct PathTraversalState {
    subpath_start: CoordPair,
    pos: CoordPair,
    last_command: PathCommand,
    last_control: CoordPair,
}

impl PathCommand {
    /// Create a normalized copy of this PathCommand. Absolute commands will be copied as-is while
    /// for relative commands an equivalent absolute command will be returned.
    ///
    /// See discussion: https://github.com/w3c/svgwg/issues/321
    /// If reduce is true then the path will be restricted to
    /// "M", "L", "C", "A" and "Z" commands.
    fn normalize(&self, state: &mut PathTraversalState, reduce: bool) -> Self {
        use crate::values::generics::basic_shape::GenericShapeCommand::*;
        match *self {
            Close => {
                state.pos = state.subpath_start;
                if reduce {
                    state.last_command = *self;
                }
                Close
            },
            Move { mut point } => {
                point = point.to_abs(state.pos);
                state.pos = point.into();
                state.subpath_start = point.into();
                if reduce {
                    state.last_command = *self;
                }
                Move { point }
            },
            Line { mut point } => {
                point = point.to_abs(state.pos);
                state.pos = point.into();
                if reduce {
                    state.last_command = *self;
                }
                Line { point }
            },
            HLine { mut x } => {
                x = x.to_abs(state.pos.x);
                state.pos.x = x.into();
                if reduce {
                    state.last_command = *self;
                    PathCommand::Line {
                        point: CommandEndPoint::ToPosition(state.pos.into()),
                    }
                } else {
                    HLine { x }
                }
            },
            VLine { mut y } => {
                y = y.to_abs(state.pos.y);
                state.pos.y = y.into();
                if reduce {
                    state.last_command = *self;
                    PathCommand::Line {
                        point: CommandEndPoint::ToPosition(state.pos.into()),
                    }
                } else {
                    VLine { y }
                }
            },
            CubicCurve {
                mut point,
                mut control1,
                mut control2,
            } => {
                control1 = control1.to_abs(state.pos, point);
                control2 = control2.to_abs(state.pos, point);
                point = point.to_abs(state.pos);
                state.pos = point.into();
                if reduce {
                    state.last_command = *self;
                    state.last_control = control2.into();
                }
                CubicCurve {
                    point,
                    control1,
                    control2,
                }
            },
            QuadCurve {
                mut point,
                mut control1,
            } => {
                control1 = control1.to_abs(state.pos, point);
                point = point.to_abs(state.pos);
                if reduce {
                    let c1 = state.pos + 2. * (CoordPair::from(control1) - state.pos) / 3.;
                    let control2 = CoordPair::from(point)
                        + 2. * (CoordPair::from(control1) - point.into()) / 3.;
                    state.pos = point.into();
                    state.last_command = *self;
                    state.last_control = control1.into();
                    CubicCurve {
                        point,
                        control1: ControlPoint::Absolute(c1.into()),
                        control2: ControlPoint::Absolute(control2.into()),
                    }
                } else {
                    state.pos = point.into();
                    QuadCurve { point, control1 }
                }
            },
            SmoothCubic {
                mut point,
                mut control2,
            } => {
                control2 = control2.to_abs(state.pos, point);
                point = point.to_abs(state.pos);
                if reduce {
                    let control1 = match state.last_command {
                        PathCommand::CubicCurve {
                            point: _,
                            control1: _,
                            control2: _,
                        }
                        | PathCommand::SmoothCubic {
                            point: _,
                            control2: _,
                        } => state.pos + state.pos - state.last_control,
                        _ => state.pos,
                    };
                    state.pos = point.into();
                    state.last_control = control2.into();
                    state.last_command = *self;
                    CubicCurve {
                        point,
                        control1: ControlPoint::Absolute(control1.into()),
                        control2,
                    }
                } else {
                    state.pos = point.into();
                    SmoothCubic { point, control2 }
                }
            },
            SmoothQuad { mut point } => {
                point = point.to_abs(state.pos);
                if reduce {
                    let control = match state.last_command {
                        PathCommand::QuadCurve {
                            point: _,
                            control1: _,
                        }
                        | PathCommand::SmoothQuad { point: _ } => {
                            state.pos + state.pos - state.last_control
                        },
                        _ => state.pos,
                    };
                    let control1 = state.pos + 2. * (control - state.pos) / 3.;
                    let control2 = CoordPair::from(point) + 2. * (control - point.into()) / 3.;
                    state.pos = point.into();
                    state.last_command = *self;
                    state.last_control = control;
                    CubicCurve {
                        point,
                        control1: ControlPoint::Absolute(control1.into()),
                        control2: ControlPoint::Absolute(control2.into()),
                    }
                } else {
                    state.pos = point.into();
                    SmoothQuad { point }
                }
            },
            Arc {
                mut point,
                radii,
                arc_sweep,
                arc_size,
                rotate,
            } => {
                point = point.to_abs(state.pos);
                state.pos = point.into();
                if reduce {
                    state.last_command = *self;
                    if radii.rx == 0. && radii.ry.as_ref().is_none_or(|v| *v == 0.) {
                        let end_point = CoordPair::from(point);
                        CubicCurve {
                            point: CommandEndPoint::ToPosition(state.pos.into()),
                            control1: ControlPoint::Absolute(end_point.into()),
                            control2: ControlPoint::Absolute(end_point.into()),
                        }
                    } else {
                        Arc {
                            point,
                            radii,
                            arc_sweep,
                            arc_size,
                            rotate,
                        }
                    }
                } else {
                    Arc {
                        point,
                        radii,
                        arc_sweep,
                        arc_size,
                        rotate,
                    }
                }
            },
        }
    }

    /// The serialization of the svg path.
    fn to_css_for_svg<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        use crate::values::generics::basic_shape::GenericShapeCommand::*;
        match *self {
            Close => dest.write_char('Z'),
            Move { point } => {
                dest.write_char(if point.is_abs() { 'M' } else { 'm' })?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
            Line { point } => {
                dest.write_char(if point.is_abs() { 'L' } else { 'l' })?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
            CubicCurve {
                point,
                control1,
                control2,
            } => {
                dest.write_char(if point.is_abs() { 'C' } else { 'c' })?;
                dest.write_char(' ')?;
                control1.to_css(dest, point.is_abs())?;
                dest.write_char(' ')?;
                control2.to_css(dest, point.is_abs())?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
            QuadCurve { point, control1 } => {
                dest.write_char(if point.is_abs() { 'Q' } else { 'q' })?;
                dest.write_char(' ')?;
                control1.to_css(dest, point.is_abs())?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
            Arc {
                point,
                radii,
                arc_sweep,
                arc_size,
                rotate,
            } => {
                dest.write_char(if point.is_abs() { 'A' } else { 'a' })?;
                dest.write_char(' ')?;
                radii.to_css(dest)?;
                dest.write_char(' ')?;
                rotate.to_css(dest)?;
                dest.write_char(' ')?;
                (arc_size as i32).to_css(dest)?;
                dest.write_char(' ')?;
                (arc_sweep as i32).to_css(dest)?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
            HLine { x } => {
                dest.write_char(if x.is_abs() { 'H' } else { 'h' })?;
                dest.write_char(' ')?;
                CSSFloat::from(x).to_css(dest)
            },
            VLine { y } => {
                dest.write_char(if y.is_abs() { 'V' } else { 'v' })?;
                dest.write_char(' ')?;
                CSSFloat::from(y).to_css(dest)
            },
            SmoothCubic { point, control2 } => {
                dest.write_char(if point.is_abs() { 'S' } else { 's' })?;
                dest.write_char(' ')?;
                control2.to_css(dest, point.is_abs())?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
            SmoothQuad { point } => {
                dest.write_char(if point.is_abs() { 'T' } else { 't' })?;
                dest.write_char(' ')?;
                CoordPair::from(point).to_css(dest)
            },
        }
    }
}

/// The path coord type.
pub type CoordPair = CoordinatePair<CSSFloat>;

impl ops::Add<CoordPair> for CoordPair {
    type Output = CoordPair;

    fn add(self, rhs: CoordPair) -> CoordPair {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl ops::Sub<CoordPair> for CoordPair {
    type Output = CoordPair;

    fn sub(self, rhs: CoordPair) -> CoordPair {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl ops::Mul<CSSFloat> for CoordPair {
    type Output = CoordPair;

    fn mul(self, f: CSSFloat) -> CoordPair {
        Self {
            x: self.x * f,
            y: self.y * f,
        }
    }
}

impl ops::Mul<CoordPair> for CSSFloat {
    type Output = CoordPair;

    fn mul(self, rhs: CoordPair) -> CoordPair {
        rhs * self
    }
}

impl ops::Div<CSSFloat> for CoordPair {
    type Output = CoordPair;

    fn div(self, f: CSSFloat) -> CoordPair {
        Self {
            x: self.x / f,
            y: self.y / f,
        }
    }
}

impl CommandEndPoint<SVGPathPosition, CSSFloat> {
    /// Converts <command-end-point> into absolutely positioned type.
    pub fn to_abs(self, state_pos: CoordPair) -> Self {
        // Consume self value.
        match self {
            CommandEndPoint::ToPosition(_) => self,
            CommandEndPoint::ByCoordinate(coord) => {
                let pos = GenericPosition {
                    horizontal: coord.x + state_pos.x,
                    vertical: coord.y + state_pos.y,
                };
                CommandEndPoint::ToPosition(pos)
            },
        }
    }
}

impl AxisEndPoint<CSSFloat> {
    /// Converts possibly relative end point into absolutely positioned type.
    pub fn to_abs(self, base: CSSFloat) -> AxisEndPoint<CSSFloat> {
        // Consume self value.
        match self {
            AxisEndPoint::ToPosition(_) => self,
            AxisEndPoint::ByCoordinate(coord) => {
                AxisEndPoint::ToPosition(AxisPosition::LengthPercent(coord + base))
            },
        }
    }
}

impl ControlPoint<SVGPathPosition, CSSFloat> {
    /// Converts <control-point> into absolutely positioned control point type.
    pub fn to_abs(
        self,
        state_pos: CoordPair,
        end_point: CommandEndPoint<SVGPathPosition, CSSFloat>,
    ) -> Self {
        // Consume self value.
        match self {
            ControlPoint::Absolute(_) => self,
            ControlPoint::Relative(point) => {
                let mut pos = GenericPosition {
                    horizontal: point.coord.x,
                    vertical: point.coord.y,
                };

                match point.reference {
                    ControlReference::Start => {
                        pos.horizontal += state_pos.x;
                        pos.vertical += state_pos.y;
                    },
                    ControlReference::End => {
                        let end = CoordPair::from(end_point);
                        pos.horizontal += end.x;
                        pos.vertical += end.y;
                    },
                    _ => (),
                }
                ControlPoint::Absolute(pos)
            },
        }
    }
}

impl From<CommandEndPoint<SVGPathPosition, CSSFloat>> for CoordPair {
    #[inline]
    fn from(p: CommandEndPoint<SVGPathPosition, CSSFloat>) -> Self {
        match p {
            CommandEndPoint::ToPosition(pos) => CoordPair {
                x: pos.horizontal,
                y: pos.vertical,
            },
            CommandEndPoint::ByCoordinate(coord) => coord,
        }
    }
}

impl From<ControlPoint<SVGPathPosition, CSSFloat>> for CoordPair {
    #[inline]
    fn from(point: ControlPoint<SVGPathPosition, CSSFloat>) -> Self {
        match point {
            ControlPoint::Absolute(pos) => CoordPair {
                x: pos.horizontal,
                y: pos.vertical,
            },
            ControlPoint::Relative(_) => {
                panic!(
                    "Attempted to convert a relative ControlPoint to CoordPair, which is lossy. \
                        Consider converting it to absolute type first using `.to_abs()`."
                )
            },
        }
    }
}

impl From<CoordPair> for CommandEndPoint<SVGPathPosition, CSSFloat> {
    #[inline]
    fn from(coord: CoordPair) -> Self {
        CommandEndPoint::ByCoordinate(coord)
    }
}

impl From<CoordPair> for SVGPathPosition {
    #[inline]
    fn from(coord: CoordPair) -> Self {
        GenericPosition {
            horizontal: coord.x,
            vertical: coord.y,
        }
    }
}

impl From<AxisEndPoint<CSSFloat>> for CSSFloat {
    #[inline]
    fn from(p: AxisEndPoint<CSSFloat>) -> Self {
        match p {
            AxisEndPoint::ToPosition(AxisPosition::LengthPercent(a)) => a,
            AxisEndPoint::ToPosition(AxisPosition::Keyword(_)) => {
                unreachable!("Invalid state: SVG path commands cannot contain a keyword.")
            },
            AxisEndPoint::ByCoordinate(a) => a,
        }
    }
}

impl ToCss for SVGPathPosition {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        self.horizontal.to_css(dest)?;
        dest.write_char(' ')?;
        self.vertical.to_css(dest)
    }
}

/// SVG Path parser.
struct PathParser<'a> {
    chars: Peekable<Cloned<slice::Iter<'a, u8>>>,
    path: Vec<PathCommand>,
}

macro_rules! parse_arguments {
    (
        $parser:ident,
        $enum:ident,
        $( $field:ident : $value:expr, )*
        [ $para:ident => $func:ident $(, $other_para:ident => $other_func:ident)* ]
    ) => {
        {
            loop {
                let $para = $func(&mut $parser.chars)?;
                $(
                    skip_comma_wsp(&mut $parser.chars);
                    let $other_para = $other_func(&mut $parser.chars)?;
                )*
                $parser.path.push(
                    PathCommand::$enum { $( $field: $value, )* $para $(, $other_para)* }
                );

                // End of string or the next character is a possible new command.
                if !skip_wsp(&mut $parser.chars) ||
                   $parser.chars.peek().map_or(true, |c| c.is_ascii_alphabetic()) {
                    break;
                }
                skip_comma_wsp(&mut $parser.chars);
            }
            Ok(())
        }
    }
}

impl<'a> PathParser<'a> {
    /// Return a PathParser.
    #[inline]
    fn new(bytes: &'a [u8]) -> Self {
        PathParser {
            chars: bytes.iter().cloned().peekable(),
            path: Vec::new(),
        }
    }

    /// Parse a sub-path.
    fn parse_subpath(&mut self) -> Result<(), ()> {
        // Handle "moveto" Command first. If there is no "moveto", this is not a valid sub-path
        // (i.e. not a valid moveto-drawto-command-group).
        self.parse_moveto()?;

        // Handle other commands.
        loop {
            skip_wsp(&mut self.chars);
            if self.chars.peek().map_or(true, |&m| m == b'M' || m == b'm') {
                break;
            }

            let command = self.chars.next().unwrap();

            skip_wsp(&mut self.chars);
            match command {
                b'Z' | b'z' => self.parse_closepath(),
                b'L' => self.parse_line_abs(),
                b'l' => self.parse_line_rel(),
                b'H' => self.parse_h_line_abs(),
                b'h' => self.parse_h_line_rel(),
                b'V' => self.parse_v_line_abs(),
                b'v' => self.parse_v_line_rel(),
                b'C' => self.parse_curve_abs(),
                b'c' => self.parse_curve_rel(),
                b'S' => self.parse_smooth_curve_abs(),
                b's' => self.parse_smooth_curve_rel(),
                b'Q' => self.parse_quadratic_bezier_curve_abs(),
                b'q' => self.parse_quadratic_bezier_curve_rel(),
                b'T' => self.parse_smooth_quadratic_bezier_curve_abs(),
                b't' => self.parse_smooth_quadratic_bezier_curve_rel(),
                b'A' => self.parse_elliptical_arc_abs(),
                b'a' => self.parse_elliptical_arc_rel(),
                _ => return Err(()),
            }?;
        }
        Ok(())
    }

    /// Parse "moveto" command.
    fn parse_moveto(&mut self) -> Result<(), ()> {
        let command = match self.chars.next() {
            Some(c) if c == b'M' || c == b'm' => c,
            _ => return Err(()),
        };

        skip_wsp(&mut self.chars);
        let point = if command == b'M' {
            parse_command_end_abs(&mut self.chars)
        } else {
            parse_command_end_rel(&mut self.chars)
        }?;
        self.path.push(PathCommand::Move { point });

        // End of string or the next character is a possible new command.
        if !skip_wsp(&mut self.chars) || self.chars.peek().map_or(true, |c| c.is_ascii_alphabetic())
        {
            return Ok(());
        }
        skip_comma_wsp(&mut self.chars);

        // If a moveto is followed by multiple pairs of coordinates, the subsequent
        // pairs are treated as implicit lineto commands.
        if point.is_abs() {
            self.parse_line_abs()
        } else {
            self.parse_line_rel()
        }
    }

    /// Parse "closepath" command.
    fn parse_closepath(&mut self) -> Result<(), ()> {
        self.path.push(PathCommand::Close);
        Ok(())
    }

    /// Parse an absolute "lineto" ("L") command.
    fn parse_line_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, Line, [ point => parse_command_end_abs ])
    }

    /// Parse a relative "lineto" ("l") command.
    fn parse_line_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, Line, [ point => parse_command_end_rel ])
    }

    /// Parse an absolute horizontal "lineto" ("H") command.
    fn parse_h_line_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, HLine, [ x => parse_axis_end_abs ])
    }

    /// Parse a relative horizontal "lineto" ("h") command.
    fn parse_h_line_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, HLine, [ x => parse_axis_end_rel ])
    }

    /// Parse an absolute vertical "lineto" ("V") command.
    fn parse_v_line_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, VLine, [ y => parse_axis_end_abs ])
    }

    /// Parse a relative vertical "lineto" ("v") command.
    fn parse_v_line_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, VLine, [ y => parse_axis_end_rel ])
    }

    /// Parse an absolute cubic Bézier curve ("C") command.
    fn parse_curve_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, CubicCurve, [
            control1 => parse_control_point_abs, control2 => parse_control_point_abs, point => parse_command_end_abs
        ])
    }

    /// Parse a relative cubic Bézier curve ("c") command.
    fn parse_curve_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, CubicCurve, [
            control1 => parse_control_point_rel, control2 => parse_control_point_rel, point => parse_command_end_rel
        ])
    }

    /// Parse an absolute smooth "curveto" ("S") command.
    fn parse_smooth_curve_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, SmoothCubic, [
            control2 => parse_control_point_abs, point => parse_command_end_abs
        ])
    }

    /// Parse a relative smooth "curveto" ("s") command.
    fn parse_smooth_curve_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, SmoothCubic, [
            control2 => parse_control_point_rel, point => parse_command_end_rel
        ])
    }

    /// Parse an absolute quadratic Bézier curve ("Q") command.
    fn parse_quadratic_bezier_curve_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, QuadCurve, [
            control1 => parse_control_point_abs, point => parse_command_end_abs
        ])
    }

    /// Parse a relative quadratic Bézier curve ("q") command.
    fn parse_quadratic_bezier_curve_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, QuadCurve, [
            control1 => parse_control_point_rel, point => parse_command_end_rel
        ])
    }

    /// Parse an absolute smooth quadratic Bézier curveto ("T") command.
    fn parse_smooth_quadratic_bezier_curve_abs(&mut self) -> Result<(), ()> {
        parse_arguments!(self, SmoothQuad, [ point => parse_command_end_abs ])
    }

    /// Parse a relative smooth quadratic Bézier curveto ("t") command.
    fn parse_smooth_quadratic_bezier_curve_rel(&mut self) -> Result<(), ()> {
        parse_arguments!(self, SmoothQuad, [ point => parse_command_end_rel ])
    }

    /// Parse an absolute elliptical arc curve ("A") command.
    fn parse_elliptical_arc_abs(&mut self) -> Result<(), ()> {
        let (parse_arc_size, parse_arc_sweep) = Self::arc_flag_parsers();
        parse_arguments!(self, Arc, [
            radii => parse_arc_radii,
            rotate => parse_number,
            arc_size => parse_arc_size,
            arc_sweep => parse_arc_sweep,
            point => parse_command_end_abs
        ])
    }

    /// Parse a relative elliptical arc curve ("a") command.
    fn parse_elliptical_arc_rel(&mut self) -> Result<(), ()> {
        let (parse_arc_size, parse_arc_sweep) = Self::arc_flag_parsers();
        parse_arguments!(self, Arc, [
            radii => parse_arc_radii,
            rotate => parse_number,
            arc_size => parse_arc_size,
            arc_sweep => parse_arc_sweep,
            point => parse_command_end_rel
        ])
    }

    /// Helper that returns parsers for the arc-size and arc-sweep flags.
    fn arc_flag_parsers() -> (
        impl Fn(&mut Peekable<Cloned<slice::Iter<'_, u8>>>) -> Result<ArcSize, ()>,
        impl Fn(&mut Peekable<Cloned<slice::Iter<'_, u8>>>) -> Result<ArcSweep, ()>,
    ) {
        // Parse a flag whose value is '0' or '1'; otherwise, return Err(()).
        let parse_arc_size = |iter: &mut Peekable<Cloned<slice::Iter<u8>>>| match iter.next() {
            Some(c) if c == b'1' => Ok(ArcSize::Large),
            Some(c) if c == b'0' => Ok(ArcSize::Small),
            _ => Err(()),
        };
        let parse_arc_sweep = |iter: &mut Peekable<Cloned<slice::Iter<u8>>>| match iter.next() {
            Some(c) if c == b'1' => Ok(ArcSweep::Cw),
            Some(c) if c == b'0' => Ok(ArcSweep::Ccw),
            _ => Err(()),
        };
        (parse_arc_size, parse_arc_sweep)
    }
}

/// Parse a pair of numbers into CoordPair.
fn parse_coord(iter: &mut Peekable<Cloned<slice::Iter<u8>>>) -> Result<CoordPair, ()> {
    let x = parse_number(iter)?;
    skip_comma_wsp(iter);
    let y = parse_number(iter)?;
    Ok(CoordPair::new(x, y))
}

/// Parse a pair of numbers that describes the absolutely positioned end point.
fn parse_command_end_abs(
    iter: &mut Peekable<Cloned<slice::Iter<u8>>>,
) -> Result<CommandEndPoint<SVGPathPosition, CSSFloat>, ()> {
    let coord = parse_coord(iter)?;
    Ok(CommandEndPoint::ToPosition(coord.into()))
}

/// Parse a pair of numbers that describes the relatively positioned end point.
fn parse_command_end_rel(
    iter: &mut Peekable<Cloned<slice::Iter<u8>>>,
) -> Result<CommandEndPoint<SVGPathPosition, CSSFloat>, ()> {
    let coord = parse_coord(iter)?;
    Ok(CommandEndPoint::ByCoordinate(coord))
}

/// Parse a pair of values that describe the absolutely positioned curve control point.
fn parse_control_point_abs(
    iter: &mut Peekable<Cloned<slice::Iter<u8>>>,
) -> Result<ControlPoint<SVGPathPosition, CSSFloat>, ()> {
    let coord = parse_coord(iter)?;
    Ok(ControlPoint::Relative(RelativeControlPoint {
        coord,
        reference: ControlReference::Origin,
    }))
}

/// Parse a pair of values that describe the relatively positioned curve control point.
fn parse_control_point_rel(
    iter: &mut Peekable<Cloned<slice::Iter<u8>>>,
) -> Result<ControlPoint<SVGPathPosition, CSSFloat>, ()> {
    let coord = parse_coord(iter)?;
    Ok(ControlPoint::Relative(RelativeControlPoint {
        coord,
        reference: ControlReference::Start,
    }))
}

/// Parse a number that describes the absolutely positioned axis end point.
fn parse_axis_end_abs(
    iter: &mut Peekable<Cloned<slice::Iter<u8>>>,
) -> Result<AxisEndPoint<f32>, ()> {
    let value = parse_number(iter)?;
    Ok(AxisEndPoint::ToPosition(AxisPosition::LengthPercent(value)))
}

/// Parse a number that describes the relatively positioned axis end point.
fn parse_axis_end_rel(
    iter: &mut Peekable<Cloned<slice::Iter<u8>>>,
) -> Result<AxisEndPoint<f32>, ()> {
    let value = parse_number(iter)?;
    Ok(AxisEndPoint::ByCoordinate(value))
}

/// Parse a pair of numbers that describes the size of the ellipse that the arc is taken from.
fn parse_arc_radii(iter: &mut Peekable<Cloned<slice::Iter<u8>>>) -> Result<ArcRadii<CSSFloat>, ()> {
    let coord = parse_coord(iter)?;
    Ok(ArcRadii {
        rx: coord.x,
        ry: Some(coord.y).into(),
    })
}

/// This is a special version which parses the number for SVG Path. e.g. "M 0.6.5" should be parsed
/// as MoveTo with a coordinate of ("0.6", ".5"), instead of treating 0.6.5 as a non-valid floating
/// point number. In other words, the logic here is similar with that of
/// tokenizer::consume_numeric, which also consumes the number as many as possible, but here the
/// input is a Peekable and we only accept an integer of a floating point number.
///
/// The "number" syntax in https://www.w3.org/TR/SVG/paths.html#PathDataBNF
fn parse_number(iter: &mut Peekable<Cloned<slice::Iter<u8>>>) -> Result<CSSFloat, ()> {
    // 1. Check optional sign.
    let sign = if iter
        .peek()
        .map_or(false, |&sign| sign == b'+' || sign == b'-')
    {
        if iter.next().unwrap() == b'-' {
            -1.
        } else {
            1.
        }
    } else {
        1.
    };

    // 2. Check integer part.
    let mut integral_part: f64 = 0.;
    let got_dot = if !iter.peek().map_or(false, |&n| n == b'.') {
        // If the first digit in integer part is neither a dot nor a digit, this is not a number.
        if iter.peek().map_or(true, |n| !n.is_ascii_digit()) {
            return Err(());
        }

        while iter.peek().map_or(false, |n| n.is_ascii_digit()) {
            integral_part = integral_part * 10. + (iter.next().unwrap() - b'0') as f64;
        }

        iter.peek().map_or(false, |&n| n == b'.')
    } else {
        true
    };

    // 3. Check fractional part.
    let mut fractional_part: f64 = 0.;
    if got_dot {
        // Consume '.'.
        iter.next();
        // If the first digit in fractional part is not a digit, this is not a number.
        if iter.peek().map_or(true, |n| !n.is_ascii_digit()) {
            return Err(());
        }

        let mut factor = 0.1;
        while iter.peek().map_or(false, |n| n.is_ascii_digit()) {
            fractional_part += (iter.next().unwrap() - b'0') as f64 * factor;
            factor *= 0.1;
        }
    }

    let mut value = sign * (integral_part + fractional_part);

    // 4. Check exp part. The segment name of SVG Path doesn't include 'E' or 'e', so it's ok to
    //    treat the numbers after 'E' or 'e' are in the exponential part.
    if iter.peek().map_or(false, |&exp| exp == b'E' || exp == b'e') {
        // Consume 'E' or 'e'.
        iter.next();
        let exp_sign = if iter
            .peek()
            .map_or(false, |&sign| sign == b'+' || sign == b'-')
        {
            if iter.next().unwrap() == b'-' {
                -1.
            } else {
                1.
            }
        } else {
            1.
        };

        let mut exp: f64 = 0.;
        while iter.peek().map_or(false, |n| n.is_ascii_digit()) {
            exp = exp * 10. + (iter.next().unwrap() - b'0') as f64;
        }

        value *= f64::powf(10., exp * exp_sign);
    }

    if value.is_finite() {
        Ok(value.min(f32::MAX as f64).max(f32::MIN as f64) as CSSFloat)
    } else {
        Err(())
    }
}

/// Skip all svg whitespaces, and return true if |iter| hasn't finished.
#[inline]
fn skip_wsp(iter: &mut Peekable<Cloned<slice::Iter<u8>>>) -> bool {
    // Note: SVG 1.1 defines the whitespaces as \u{9}, \u{20}, \u{A}, \u{D}.
    //       However, SVG 2 has one extra whitespace: \u{C}.
    //       Therefore, we follow the newest spec for the definition of whitespace,
    //       i.e. \u{9}, \u{20}, \u{A}, \u{C}, \u{D}.
    while iter.peek().map_or(false, |c| c.is_ascii_whitespace()) {
        iter.next();
    }
    iter.peek().is_some()
}

/// Skip all svg whitespaces and one comma, and return true if |iter| hasn't finished.
#[inline]
fn skip_comma_wsp(iter: &mut Peekable<Cloned<slice::Iter<u8>>>) -> bool {
    if !skip_wsp(iter) {
        return false;
    }

    if *iter.peek().unwrap() != b',' {
        return true;
    }
    iter.next();

    skip_wsp(iter)
}
