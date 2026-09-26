use moli_protocol::devtools_runtime::{
    DevToolsActivateTargetCommand, DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand,
    DevToolsCloseTargetCommand, DevToolsCommand, DevToolsCreateTargetCommand,
    DevToolsDevicePixelRatioSetting, DevToolsGetLayoutMetricsCommand, DevToolsGetTargetsCommand,
    DevToolsGetTargetsResult, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
    DevToolsRemoteHandleId, DevToolsScreenshotElementClip, DevToolsSetViewportCommand,
    DevToolsSetWindowStateCommand, DevToolsTargetId, DevToolsTargetKind, DevToolsViewportSetting,
    DevToolsWindowState,
};
use serde_json::{Value, json};

use crate::{ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode};

use super::parsing::required_string;

const CENTIMETERS_PER_INCH: f64 = 2.54;
const MIN_PRINT_PAGE_SIZE_CM: f64 = 0.035278;
pub const CLASSIC_HEADLESS_SCREEN_WIDTH: u32 = 1920;
pub const CLASSIC_HEADLESS_SCREEN_HEIGHT: u32 = 1080;
pub const CLASSIC_HEADLESS_AVAILABLE_HEIGHT: u32 = 1040;

pub fn create_initial_target_command(context: &ClassicDevToolsCommandContext) -> DevToolsCommand {
    DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
        context: context.command_context(),
        url: "about:blank".to_owned(),
        browser_context_id: None,
        activate: false,
    })
}

pub fn layout_metrics_command(context: &ClassicDevToolsCommandContext) -> DevToolsCommand {
    DevToolsCommand::GetLayoutMetrics(DevToolsGetLayoutMetricsCommand {
        context: context.command_context(),
        publish_layout: false,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassicWindowPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassicWindowRectUpdate {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicWindowState {
    Maximized,
    Minimized,
    Fullscreen,
}

impl ClassicWindowRect {
    pub fn position(self) -> ClassicWindowPosition {
        ClassicWindowPosition {
            x: self.x,
            y: self.y,
        }
    }

    pub fn with_update(self, update: ClassicWindowRectUpdate) -> Self {
        Self {
            x: update.x.unwrap_or(self.x),
            y: update.y.unwrap_or(self.y),
            width: update.width.unwrap_or(self.width),
            height: update.height.unwrap_or(self.height),
        }
    }

    pub fn value(self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "width": self.width,
            "height": self.height,
        })
    }
}

pub fn set_window_rect_update(params: &Value) -> Result<ClassicWindowRectUpdate, ClassicError> {
    let Some(params) = params.as_object() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "params must be an object",
        ));
    };
    Ok(ClassicWindowRectUpdate {
        x: optional_window_position(params, "x")?,
        y: optional_window_position(params, "y")?,
        width: optional_window_dimension(params, "width")?,
        height: optional_window_dimension(params, "height")?,
    })
}

pub fn set_window_rect_command(
    context: &ClassicDevToolsCommandContext,
    width: u32,
    height: u32,
) -> DevToolsCommand {
    set_window_rect_command_with_screen(context, width, height, None, None)
}

pub fn set_window_rect_command_with_screen(
    context: &ClassicDevToolsCommandContext,
    width: u32,
    height: u32,
    screen_width: Option<u32>,
    screen_height: Option<u32>,
) -> DevToolsCommand {
    DevToolsCommand::SetViewport(DevToolsSetViewportCommand {
        context: context.command_context(),
        browser_context_ids: Vec::new(),
        viewport: DevToolsViewportSetting::Dimensions { width, height },
        device_pixel_ratio: DevToolsDevicePixelRatioSetting::Unchanged,
        screen_width,
        screen_height,
    })
}

pub fn classic_window_rect_for_state(
    current: ClassicWindowRect,
    state: ClassicWindowState,
) -> ClassicWindowRect {
    match state {
        ClassicWindowState::Maximized => ClassicWindowRect {
            x: 0,
            y: 0,
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_AVAILABLE_HEIGHT,
        },
        ClassicWindowState::Minimized => current,
        ClassicWindowState::Fullscreen => ClassicWindowRect {
            x: 0,
            y: 0,
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_SCREEN_HEIGHT,
        },
    }
}

pub fn set_window_state_command(
    context: &ClassicDevToolsCommandContext,
    state: ClassicWindowState,
) -> Option<DevToolsCommand> {
    let rect = classic_window_rect_for_state(
        ClassicWindowRect {
            x: 0,
            y: 0,
            width: CLASSIC_HEADLESS_SCREEN_WIDTH,
            height: CLASSIC_HEADLESS_SCREEN_HEIGHT,
        },
        state,
    );
    match state {
        ClassicWindowState::Minimized => None,
        ClassicWindowState::Maximized | ClassicWindowState::Fullscreen => {
            Some(set_window_rect_command_with_screen(
                context,
                rect.width,
                rect.height,
                Some(CLASSIC_HEADLESS_SCREEN_WIDTH),
                Some(CLASSIC_HEADLESS_SCREEN_HEIGHT),
            ))
        }
    }
}

pub fn set_window_surface_state_command(
    context: &ClassicDevToolsCommandContext,
    state: ClassicWindowState,
) -> DevToolsCommand {
    set_window_surface_state_command_from_devtools(
        context,
        match state {
            ClassicWindowState::Maximized => DevToolsWindowState::Maximized,
            ClassicWindowState::Minimized => DevToolsWindowState::Minimized,
            ClassicWindowState::Fullscreen => DevToolsWindowState::Fullscreen,
        },
    )
}

pub fn set_window_normal_surface_state_command(
    context: &ClassicDevToolsCommandContext,
) -> DevToolsCommand {
    set_window_surface_state_command_from_devtools(context, DevToolsWindowState::Normal)
}

fn set_window_surface_state_command_from_devtools(
    context: &ClassicDevToolsCommandContext,
    state: DevToolsWindowState,
) -> DevToolsCommand {
    DevToolsCommand::SetWindowState(DevToolsSetWindowStateCommand {
        context: context.command_context(),
        state,
    })
}

fn optional_window_position(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<i32>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_f64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("window rect {field} must be a number or null"),
        ));
    };
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("window rect {field} must fit in a signed 32-bit integer"),
        ));
    }
    Ok(Some(value.trunc() as i32))
}

fn optional_window_dimension(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<u32>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_f64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("window rect {field} must be a number or null"),
        ));
    };
    let value = value.trunc();
    if !value.is_finite() || value < 1.0 || value > u32::MAX as f64 {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("window rect {field} must be a positive viewport dimension"),
        ));
    }
    Ok(Some(value as u32))
}

pub fn screenshot_command(context: &ClassicDevToolsCommandContext) -> DevToolsCommand {
    DevToolsCommand::CaptureScreenshot(DevToolsCaptureScreenshotCommand {
        context: context.command_context(),
        format: Some("png".to_owned()),
        quality: None,
        clip: None,
        capture_beyond_viewport: false,
        optimize_for_speed: false,
    })
}

pub fn element_screenshot_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CaptureScreenshot(DevToolsCaptureScreenshotCommand {
        context: context.command_context(),
        format: Some("png".to_owned()),
        quality: None,
        clip: Some(DevToolsCaptureScreenshotClip::Element(
            DevToolsScreenshotElementClip {
                shared_id: DevToolsRemoteHandleId::from(object_id.into()),
            },
        )),
        capture_beyond_viewport: true,
        optimize_for_speed: false,
    })
}

pub fn print_page_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
) -> Result<DevToolsCommand, ClassicError> {
    let margin = classic_print_margin(params.get("margin"))?;
    let page = classic_print_page(params.get("page"))?;
    Ok(DevToolsCommand::PrintToPdf(DevToolsPrintToPdfCommand {
        context: context.command_context(),
        landscape: classic_print_orientation(params.get("orientation"))?,
        print_background: optional_bool(params, "background")?,
        scale: classic_print_scale(params.get("scale"))?,
        paper_width: page.width_inches,
        paper_height: page.height_inches,
        margin_top: margin.top_inches,
        margin_bottom: margin.bottom_inches,
        margin_left: margin.left_inches,
        margin_right: margin.right_inches,
        page_ranges: classic_print_page_ranges(params.get("pageRanges"))?,
        shrink_to_fit: optional_bool(params, "shrinkToFit")?,
        transfer_mode: Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64),
    }))
}

#[derive(Default)]
struct ClassicPrintMargin {
    top_inches: Option<f64>,
    bottom_inches: Option<f64>,
    left_inches: Option<f64>,
    right_inches: Option<f64>,
}

fn classic_print_margin(margin: Option<&Value>) -> Result<ClassicPrintMargin, ClassicError> {
    let Some(margin) = margin else {
        return Ok(ClassicPrintMargin::default());
    };
    let Some(margin) = margin.as_object() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print margin must be an object",
        ));
    };
    Ok(ClassicPrintMargin {
        top_inches: optional_cm_to_inches(margin, "top", "print margin top")?,
        bottom_inches: optional_cm_to_inches(margin, "bottom", "print margin bottom")?,
        left_inches: optional_cm_to_inches(margin, "left", "print margin left")?,
        right_inches: optional_cm_to_inches(margin, "right", "print margin right")?,
    })
}

#[derive(Default)]
struct ClassicPrintPage {
    width_inches: Option<f64>,
    height_inches: Option<f64>,
}

fn classic_print_page(page: Option<&Value>) -> Result<ClassicPrintPage, ClassicError> {
    let Some(page) = page else {
        return Ok(ClassicPrintPage::default());
    };
    let Some(page) = page.as_object() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print page must be an object",
        ));
    };
    Ok(ClassicPrintPage {
        width_inches: optional_print_page_cm_to_inches(page, "width", "print page width")?,
        height_inches: optional_print_page_cm_to_inches(page, "height", "print page height")?,
    })
}

fn optional_cm_to_inches(
    params: &serde_json::Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<f64>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_f64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{label} must be a number"),
        ));
    };
    if !value.is_finite() || value < 0.0 {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{label} must be non-negative"),
        ));
    }
    Ok(Some(value / CENTIMETERS_PER_INCH))
}

fn optional_print_page_cm_to_inches(
    params: &serde_json::Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Option<f64>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    let Some(value) = value.as_f64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{label} must be a number"),
        ));
    };
    if !value.is_finite() || value < MIN_PRINT_PAGE_SIZE_CM {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{label} must be at least {MIN_PRINT_PAGE_SIZE_CM:.6} cm"),
        ));
    }
    Ok(Some(value / CENTIMETERS_PER_INCH))
}

fn classic_print_orientation(orientation: Option<&Value>) -> Result<Option<bool>, ClassicError> {
    let Some(orientation) = orientation else {
        return Ok(None);
    };
    let Some(orientation) = orientation.as_str() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print orientation must be portrait or landscape",
        ));
    };
    match orientation {
        "portrait" => Ok(Some(false)),
        "landscape" => Ok(Some(true)),
        _ => Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print orientation must be portrait or landscape",
        )),
    }
}

fn classic_print_scale(scale: Option<&Value>) -> Result<Option<f64>, ClassicError> {
    let Some(scale) = scale else {
        return Ok(None);
    };
    let Some(scale) = scale.as_f64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print scale must be a number",
        ));
    };
    if !scale.is_finite() || !(0.1..=2.0).contains(&scale) {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print scale must be between 0.1 and 2.0",
        ));
    }
    Ok(Some(scale))
}

fn optional_bool(params: &Value, field: &str) -> Result<Option<bool>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a boolean"),
        )
    })
}

fn classic_print_page_ranges(page_ranges: Option<&Value>) -> Result<Option<String>, ClassicError> {
    let Some(page_ranges) = page_ranges else {
        return Ok(None);
    };
    let Some(page_ranges) = page_ranges.as_array() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "print pageRanges must be an array",
        ));
    };
    let mut ranges = Vec::with_capacity(page_ranges.len());
    for range in page_ranges {
        if let Some(page) = range.as_u64() {
            if page == 0 {
                return Err(ClassicError::new(
                    ClassicErrorCode::InvalidArgument,
                    "print pageRanges entries must be positive",
                ));
            }
            ranges.push(page.to_string());
            continue;
        }
        let Some(range) = range.as_str() else {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "print pageRanges entries must be strings or positive integers",
            ));
        };
        if !valid_classic_print_page_range(range) {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "print pageRanges entry is invalid",
            ));
        }
        ranges.push(range.to_owned());
    }
    Ok(Some(ranges.join(",")))
}

fn valid_classic_print_page_range(range: &str) -> bool {
    if range.is_empty() {
        return false;
    }
    let Some((start, end)) = range.split_once('-') else {
        return range.parse::<u64>().is_ok_and(|page| page > 0);
    };
    if start.is_empty() || end.is_empty() {
        return false;
    }
    let Ok(start) = start.parse::<u64>() else {
        return false;
    };
    let Ok(end) = end.parse::<u64>() else {
        return false;
    };
    start > 0 && end >= start
}

pub fn window_handles_command(context: &ClassicDevToolsCommandContext) -> DevToolsCommand {
    DevToolsCommand::GetTargets(DevToolsGetTargetsCommand {
        context: context.command_context(),
        root: None,
        max_depth: None,
        filter: None,
    })
}

pub fn new_window_type(params: &Value) -> Result<String, ClassicError> {
    let Some(value) = params.get("type") else {
        return Ok("tab".to_owned());
    };
    if value.is_null() {
        return Ok("tab".to_owned());
    }
    let Some(type_name) = value.as_str() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "type must be a string",
        ));
    };
    Ok(match type_name {
        "tab" | "window" => type_name,
        _ => "tab",
    }
    .to_owned())
}

pub fn new_window_command(context: &ClassicDevToolsCommandContext) -> DevToolsCommand {
    DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
        context: context.command_context(),
        url: "about:blank".to_owned(),
        browser_context_id: None,
        activate: false,
    })
}

pub fn switch_window_command(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
) -> Result<DevToolsCommand, ClassicError> {
    let handle = required_string(params, "handle")?;
    Ok(DevToolsCommand::ActivateTarget(
        DevToolsActivateTargetCommand {
            context: ClassicDevToolsCommandContext::with_target_id(&context.session_id, handle)
                .command_context(),
            target_id: DevToolsTargetId::from(handle),
        },
    ))
}

pub fn activate_window_command(
    context: &ClassicDevToolsCommandContext,
    target_id: impl AsRef<str>,
) -> DevToolsCommand {
    let target_id = target_id.as_ref();
    DevToolsCommand::ActivateTarget(DevToolsActivateTargetCommand {
        context: ClassicDevToolsCommandContext::with_target_id(&context.session_id, target_id)
            .command_context(),
        target_id: DevToolsTargetId::from(target_id),
    })
}

pub fn close_window_command(
    context: &ClassicDevToolsCommandContext,
) -> Result<DevToolsCommand, ClassicError> {
    let Some(target_id) = context.target_id.as_deref() else {
        return Err(ClassicError::new(
            ClassicErrorCode::NoSuchWindow,
            "current window not found",
        ));
    };
    Ok(DevToolsCommand::CloseTarget(DevToolsCloseTargetCommand {
        context: context.command_context(),
        target_id: DevToolsTargetId::from(target_id),
    }))
}

pub fn window_handles_from_targets(result: DevToolsGetTargetsResult) -> Vec<String> {
    result
        .targets
        .into_iter()
        .filter(|target| target.kind == DevToolsTargetKind::Page)
        .filter_map(|target| target.target_id.map(DevToolsTargetId::into_string))
        .collect()
}
