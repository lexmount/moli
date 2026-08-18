use super::super::{geometry::observable_element_metrics, html_element_getter_receiver};

#[derive(Clone, Copy)]
enum ImageCoordinateAxis {
    X,
    Y,
}

impl ImageCoordinateAxis {
    const fn member(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }

    const fn select(self, point: moli_layout::LayoutPoint) -> f32 {
        match self {
            Self::X => point.x,
            Self::Y => point.y,
        }
    }
}

fn image_root_coordinate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    axis: ImageCoordinateAxis,
) -> Result<i32, moli_layout::LayoutError> {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, object, "HTMLImageElement", axis.member(), "img")
    else {
        return Ok(0);
    };
    let runtime = unsafe { &*runtime_ptr };

    let origin = observable_element_metrics(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )?
    .map(|metrics| metrics.border_origin_in_viewport_ignoring_css_transforms)
    .unwrap_or(moli_layout::LayoutPoint::ZERO);
    // Blink returns LayoutUnit::ToInt(), whose integer division truncates
    // toward zero. Rust's float-to-integer cast has the same truncation and
    // saturates at the WebIDL `long` bounds.
    Ok(axis.select(origin) as i32)
}

fn return_image_root_coordinate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    axis: ImageCoordinateAxis,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match image_root_coordinate(scope, object, axis) {
        Ok(value) => rv.set_int32(value),
        Err(error) => {
            let member = axis.member();
            let message = format!("Layout failed while reading HTMLImageElement.{member}: {error}");
            if let Some(message) = crate::util::v8_string(scope, &message) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            rv.set_int32(0);
        }
    }
}

pub(in crate::native_bridge) fn image_x_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    return_image_root_coordinate(scope, args.this(), ImageCoordinateAxis::X, rv);
}

pub(in crate::native_bridge) fn image_y_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    return_image_root_coordinate(scope, args.this(), ImageCoordinateAxis::Y, rv);
}
