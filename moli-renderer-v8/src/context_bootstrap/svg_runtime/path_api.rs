//! Structured access to the base `d` attribute, independent of CSS and animation.

use kurbo::{Arc, Point, SvgArc, Vec2};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::fmt::Write;
use style::values::generics::basic_shape::ControlPoint;
use style::values::specified::svg_path::{CoordPair, PathCommand, SVGPathData, SVGPathPosition};

use crate::{
    native_bridge::node_runtime_and_handle_from_object_or_detached, util::v8_string,
    web_api_interfaces, webidl,
};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SVGPathSegment")]
struct PathSegment {
    #[webidl(required)]
    r#type: String,
    #[webidl(required, sequence, converter = "unrestricted_float")]
    values: Vec<f32>,
}

impl PathSegment {
    fn new(command: char, values: impl Into<Vec<f32>>) -> Self {
        Self {
            r#type: command.to_string(),
            values: values.into(),
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
struct PathDataSettings {
    #[webidl(default = false)]
    normalize: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGPathElement.getPathData")]
struct GetArgs {
    #[webidl(dictionary)]
    settings: PathDataSettings,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGPathElement.setPathData")]
struct SetArgs {
    #[webidl(required, converter = "raw")]
    path_data: webidl::Sequence<webidl::Dictionary<PathSegment>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct SegmentDictionary<'s> {
    r#type: v8::Local<'s, v8::String>,
    values: v8::Local<'s, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGPathElement, enumerable, receiver)]
struct PathMethods {
    #[webapi(method = "getPathData", length = 0, callback = get_path_data)]
    get_path_data: (),
    #[webapi(method = "setPathData", length = 1, callback = set_path_data)]
    set_path_data: (),
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    PathMethods::initialize_prototype_template(scope, prototype);
}

fn get_path_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<GetArgs>(scope, &args) else {
        return;
    };
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGPathElement receiver check validates native identity");
    let raw = super::builders::svg_owner_attribute_value(scope, receiver, "d").unwrap_or_default();
    let segments: Vec<_> = moli_svg::parse_path_segments(&raw)
        .map_while(|segment| {
            let values: Vec<f32> = segment
                .values
                .into_iter()
                .map(|value| value as f32)
                .collect();
            values
                .iter()
                .all(|value| value.is_finite())
                .then(|| PathSegment::new(segment.command, values))
        })
        .collect();
    let segments = if parsed.settings.normalize {
        // Only validated binary32 commands reach the CSS normalizer: its parser
        // clamps larger finite doubles, which SVG DOM error recovery must reject.
        let raw = serialize_valid_prefix(segments);
        let (path, _) = SVGPathData::parse_bytes(raw.as_bytes());
        normalized_segments(&path)
    } else {
        segments
    };
    let dictionaries: Vec<v8::Local<v8::Value>> = segments
        .into_iter()
        .map(|segment| {
            let values: Vec<_> = segment
                .values
                .into_iter()
                .map(|value| v8::Number::new(scope, f64::from(value)).into())
                .collect();
            let values = v8::Array::new_with_elements(scope, &values);
            let command =
                v8_string(scope, &segment.r#type).expect("an SVG command string should allocate");
            SegmentDictionary::new(command, values)
                .bind(scope)
                .expect("path segment dictionary should bind")
                .into()
        })
        .collect();
    rv.set(v8::Array::new_with_elements(scope, &dictionaries).into());
}

fn set_path_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Convert the entire iterable before semantic validation or mutation. Even
    // a getter after an invalid command can throw and must leave `d` untouched.
    let Some(parsed) = webidl::parse_args::<SetArgs>(scope, &args) else {
        return;
    };
    let segments = parsed.path_data.0.into_iter().map(|segment| segment.0);
    let value = serialize_valid_prefix(segments);
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGPathElement receiver check validates native identity");
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .expect("native SVG path has a node handle");
    let runtime = unsafe { &mut *runtime_ptr };
    if value.is_empty() {
        let _ = runtime.remove_attribute(scope, runtime_ptr, handle, "d");
    } else {
        let _ = runtime.set_attribute(scope, runtime_ptr, handle, "d", &value);
    }
}

fn serialize_valid_prefix(segments: impl IntoIterator<Item = PathSegment>) -> String {
    let mut output = String::new();
    for segment in segments {
        let [original] = segment.r#type.as_bytes() else {
            break;
        };
        let command = original.to_ascii_uppercase();
        let arity = match command {
            b'M' | b'L' | b'T' => 2,
            b'H' | b'V' => 1,
            b'C' => 6,
            b'S' | b'Q' => 4,
            b'A' => 7,
            b'Z' => 0,
            _ => break,
        };
        if segment.values.len() != arity
            || segment.values.iter().any(|value| !value.is_finite())
            || (output.is_empty() && command != b'M')
        {
            break;
        }
        if !output.is_empty() {
            output.push(' ');
        }
        output.push(if command == b'Z' {
            'Z'
        } else {
            *original as char
        });
        for (index, value) in segment.values.into_iter().enumerate() {
            let value = if command == b'A' && matches!(index, 3 | 4) {
                if value == 0.0 { 0.0 } else { 1.0 }
            } else {
                value
            };
            // Preserve the converted binary32 coordinate when writing the base
            // attribute; display-oriented CSS serialization loses precision.
            write!(output, " {}", f64::from(value))
                .expect("writing a path attribute to a String is infallible");
        }
    }
    output
}

fn endpoint(point: impl Into<CoordPair>) -> [f32; 2] {
    let point = point.into();
    [point.x, point.y]
}

fn control(point: ControlPoint<SVGPathPosition, f32>) -> [f32; 2] {
    match point {
        ControlPoint::Absolute(point) => [point.horizontal, point.vertical],
        ControlPoint::Relative(point) => [point.coord.x, point.coord.y],
    }
}

fn segment_from_command(command: &PathCommand) -> PathSegment {
    let letter = |absolute: bool, command: char| {
        if absolute {
            command
        } else {
            command.to_ascii_lowercase()
        }
    };
    match *command {
        PathCommand::Move { point } => {
            PathSegment::new(letter(point.is_abs(), 'M'), endpoint(point))
        }
        PathCommand::Line { point } => {
            PathSegment::new(letter(point.is_abs(), 'L'), endpoint(point))
        }
        PathCommand::HLine { x } => PathSegment::new(letter(x.is_abs(), 'H'), [f32::from(x)]),
        PathCommand::VLine { y } => PathSegment::new(letter(y.is_abs(), 'V'), [f32::from(y)]),
        PathCommand::CubicCurve {
            point,
            control1,
            control2,
        } => {
            let [x, y] = endpoint(point);
            let [x1, y1] = control(control1);
            let [x2, y2] = control(control2);
            PathSegment::new(letter(point.is_abs(), 'C'), [x1, y1, x2, y2, x, y])
        }
        PathCommand::SmoothCubic { point, control2 } => {
            let [x, y] = endpoint(point);
            let [x2, y2] = control(control2);
            PathSegment::new(letter(point.is_abs(), 'S'), [x2, y2, x, y])
        }
        PathCommand::QuadCurve { point, control1 } => {
            let [x, y] = endpoint(point);
            let [x1, y1] = control(control1);
            PathSegment::new(letter(point.is_abs(), 'Q'), [x1, y1, x, y])
        }
        PathCommand::SmoothQuad { point } => {
            PathSegment::new(letter(point.is_abs(), 'T'), endpoint(point))
        }
        PathCommand::Arc {
            point,
            radii,
            arc_sweep,
            arc_size,
            rotate,
        } => {
            let [x, y] = endpoint(point);
            PathSegment::new(
                letter(point.is_abs(), 'A'),
                [
                    radii.rx,
                    radii.ry.as_ref().copied().unwrap_or(radii.rx),
                    rotate,
                    arc_size as u8 as f32,
                    arc_sweep as u8 as f32,
                    x,
                    y,
                ],
            )
        }
        PathCommand::Close => PathSegment::new('Z', []),
    }
}

fn normalized_segments(path: &SVGPathData) -> Vec<PathSegment> {
    let path = path.normalize(true);
    let mut segments = Vec::new();
    let mut position = Point::ZERO;
    let mut start = Point::ZERO;
    for command in path.commands() {
        if let PathCommand::Arc {
            point,
            radii,
            arc_sweep,
            arc_size,
            rotate,
        } = *command
        {
            let [x, y] = endpoint(point);
            let end = Point::new(f64::from(x), f64::from(y));
            let svg_arc = SvgArc {
                from: position,
                to: end,
                radii: Vec2::new(
                    f64::from(radii.rx),
                    f64::from(radii.ry.as_ref().copied().unwrap_or(radii.rx)),
                ),
                x_rotation: f64::from(rotate).to_radians(),
                large_arc: arc_size as u8 != 0,
                sweep: arc_sweep as u8 != 0,
            };
            if let Some(arc) = Arc::from_svg_arc(&svg_arc) {
                // Kurbo's minimum subdivision is one cubic per quadrant. A
                // radius-relative tolerance retains that canonical reduction.
                arc.to_cubic_beziers(arc.radii.x.max(arc.radii.y), |p1, p2, p3| {
                    segments.push(PathSegment::new(
                        'C',
                        [
                            p1.x as f32,
                            p1.y as f32,
                            p2.x as f32,
                            p2.y as f32,
                            p3.x as f32,
                            p3.y as f32,
                        ],
                    ));
                });
            } else {
                segments.push(PathSegment::new('L', [x, y]));
            }
            position = end;
            continue;
        }
        let segment = segment_from_command(command);
        match segment.r#type.as_str() {
            "M" => {
                position = Point::new(f64::from(segment.values[0]), f64::from(segment.values[1]));
                start = position;
            }
            "L" | "C" => {
                let values = &segment.values;
                position = Point::new(
                    f64::from(values[values.len() - 2]),
                    f64::from(values[values.len() - 1]),
                );
            }
            "Z" => position = start,
            _ => unreachable!("Stylo reduction produces only M/L/C/A/Z"),
        }
        segments.push(segment);
    }
    segments
}
