use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScreenshotClip {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScreenshotParams {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    quality: Option<i32>,
    #[serde(default)]
    clip: Option<ScreenshotClip>,
    #[serde(default)]
    from_surface: Option<bool>,
    #[serde(default)]
    capture_beyond_viewport: Option<bool>,
    #[serde(default)]
    optimize_for_speed: Option<bool>,
}

pub(super) fn unsupported_cdp_screenshot_option(option: &str) -> CommandOutputPlan {
    CommandOutputPlan::error(
        -32000,
        format!("Page.captureScreenshot option '{option}' is not supported."),
    )
}

pub(super) fn build_cdp_capture_screenshot_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsCaptureScreenshotCommand, CommandOutputPlan> {
    let params: ScreenshotParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        Ok(None) => ScreenshotParams {
            format: None,
            quality: None,
            clip: None,
            from_surface: None,
            capture_beyond_viewport: None,
            optimize_for_speed: None,
        },
        Err(e) => return Err(CommandOutputPlan::error(-32602, e)),
    };
    match params.format.as_deref() {
        None | Some("png" | "jpeg") => {}
        Some("webp") => {
            return Err(unsupported_cdp_screenshot_option("format"));
        }
        Some(_) => {
            return Err(CommandOutputPlan::error(-32602, "Invalid image format"));
        }
    }
    let quality = params
        .quality
        .map(|quality| {
            u8::try_from(quality)
                .ok()
                .filter(|quality| *quality <= 100)
                .ok_or_else(|| {
                    CommandOutputPlan::error(
                        -32602,
                        "Page.captureScreenshot quality must be between 0 and 100.",
                    )
                })
        })
        .transpose()?;
    if let Some(clip) = params.clip.as_ref()
        && (!clip.x.is_finite()
            || !clip.y.is_finite()
            || !clip.width.is_finite()
            || !clip.height.is_finite()
            || !clip.scale.is_finite()
            || clip.width <= 0.0
            || clip.height <= 0.0
            || clip.scale <= 0.0)
    {
        return Err(CommandOutputPlan::error(
            -32602,
            "Page.captureScreenshot clip must have a finite origin and positive finite width, height, and scale.",
        ));
    }
    if params.from_surface == Some(false) {
        return Err(unsupported_cdp_screenshot_option("fromSurface"));
    }
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsCaptureScreenshotCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        format: params.format,
        quality,
        clip: params.clip.map(|clip| {
            DevToolsCaptureScreenshotClip::Box(DevToolsScreenshotClip {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
                scale: clip.scale,
            })
        }),
        capture_beyond_viewport: params.capture_beyond_viewport.unwrap_or(false),
        optimize_for_speed: params.optimize_for_speed.unwrap_or(false),
    })
}

pub(super) fn devtools_capture_screenshot_error(
    command: &DevToolsCaptureScreenshotCommand,
) -> DevToolsError {
    match command.format.as_deref() {
        None | Some("png") => DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            CAPTURE_SCREENSHOT_UNSUPPORTED_MESSAGE,
        ),
        _ => DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "unsupported screenshot format.",
        ),
    }
}

pub(super) fn try_start_page_capture_screenshot_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = match build_cdp_capture_screenshot_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return PageCommandTaskStep::Complete(plan),
    };
    start_devtools_page_command(conn, cmd.id, AutomationCommand::CaptureScreenshot(command))
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CapturePageSnapshotParams {
    #[serde(default)]
    format: Option<String>,
}

pub(super) fn try_start_page_capture_snapshot_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: CapturePageSnapshotParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => CapturePageSnapshotParams::default(),
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32602, error));
        }
    };
    if !matches!(params.format.as_deref(), None | Some("mhtml")) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "unsupported snapshot format.",
        ));
    }
    let page = match conn.loaded_page_mut_for_protocol_access(cmd.session_id) {
        Ok(page) => page,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    match page.start_serialize_html() {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id: cmd.id,
            owner_scope: CommandOwnerScope::capture(conn, cmd.session_id),
            kind: Box::new(PendingPageCommandKind::CaptureSnapshot { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to serialize page snapshot: {error}"),
        )),
    }
}

pub(super) fn build_mhtml_snapshot(url: &str, html: &str) -> String {
    let boundary = "----MultipartBoundary--moli";
    let content_location = sanitize_mhtml_header_value(url);
    let encoded_html = BASE64_STANDARD.encode(html.as_bytes());
    format!(
        concat!(
            "Snapshot-Content-Location: {content_location}\r\n",
            "Subject: \r\n",
            "Date: \r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/related;\r\n",
            "\ttype=\"text/html\";\r\n",
            "\tboundary=\"{boundary}\"\r\n",
            "\r\n",
            "--{boundary}\r\n",
            "Content-Type: text/html\r\n",
            "Content-ID: <frame-1@mhtml.moli>\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "Content-Location: {content_location}\r\n",
            "\r\n",
            "{encoded_html}\r\n",
            "--{boundary}--\r\n"
        ),
        boundary = boundary,
        content_location = content_location,
        encoded_html = encoded_html,
    )
}

pub(super) fn sanitize_mhtml_header_value(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '\r' | '\n'))
        .collect()
}

pub(super) fn default_print_to_pdf_params() -> PrintToPdfParams {
    PrintToPdfParams {
        landscape: None,
        display_header_footer: None,
        print_background: None,
        scale: None,
        paper_width: None,
        paper_height: None,
        margin_top: None,
        margin_bottom: None,
        margin_left: None,
        margin_right: None,
        page_ranges: None,
        header_template: None,
        footer_template: None,
        prefer_css_page_size: None,
        transfer_mode: None,
        generate_tagged_pdf: None,
        generate_document_outline: None,
    }
}

pub(super) fn build_cdp_print_to_pdf_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsPrintToPdfCommand, CommandOutputPlan> {
    let params: PrintToPdfParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => default_print_to_pdf_params(),
        Err(error) => return Err(CommandOutputPlan::error(-32602, error)),
    };
    if params.display_header_footer.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option("displayHeaderFooter"));
    }
    if params.prefer_css_page_size.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option("preferCSSPageSize"));
    }
    if params.generate_tagged_pdf.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option("generateTaggedPDF"));
    }
    if params.generate_document_outline.unwrap_or(false) {
        return Err(unsupported_cdp_print_to_pdf_option(
            "generateDocumentOutline",
        ));
    }
    let transfer_mode = match params.transfer_mode {
        Some(PrintToPdfTransferMode::ReturnAsBase64) => {
            Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64)
        }
        Some(PrintToPdfTransferMode::ReturnAsStream) => {
            Some(DevToolsPrintToPdfTransferMode::ReturnAsStream)
        }
        None => None,
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsPrintToPdfCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        landscape: params.landscape,
        print_background: params.print_background,
        scale: params.scale,
        paper_width: params.paper_width,
        paper_height: params.paper_height,
        margin_top: params.margin_top,
        margin_bottom: params.margin_bottom,
        margin_left: params.margin_left,
        margin_right: params.margin_right,
        page_ranges: params.page_ranges,
        shrink_to_fit: None,
        transfer_mode,
    })
}

pub(super) fn unsupported_cdp_print_to_pdf_option(option: &str) -> CommandOutputPlan {
    CommandOutputPlan::error(
        -32000,
        format!("Page.printToPDF option '{option}' is not supported."),
    )
}

pub(super) fn devtools_print_to_pdf_error(command: &DevToolsPrintToPdfCommand) -> DevToolsError {
    if let Some(message) = print_to_pdf_page_size_error(command) {
        return DevToolsError::new(DevToolsErrorKind::Unsupported, message);
    }
    DevToolsError::new(
        DevToolsErrorKind::Unsupported,
        PRINT_TO_PDF_UNSUPPORTED_MESSAGE,
    )
}

pub(super) fn complete_devtools_print_to_pdf_command(
    conn: &mut CdpConnection,
    command: DevToolsPrintToPdfCommand,
) -> CommandOutputPlan {
    if let Err(error) = validate_page_capture_target_context(conn, &command.context) {
        return CommandOutputPlan::from_devtools_error(error);
    }
    CommandOutputPlan::from_devtools_error(devtools_print_to_pdf_error(&command))
}

pub(super) fn print_to_pdf_page_size_error(
    command: &DevToolsPrintToPdfCommand,
) -> Option<&'static str> {
    let paper_width = command
        .paper_width
        .unwrap_or(DEFAULT_PRINT_PAGE_WIDTH_INCHES);
    let paper_height = command
        .paper_height
        .unwrap_or(DEFAULT_PRINT_PAGE_HEIGHT_INCHES);
    let margin_left = command.margin_left.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    let margin_right = command.margin_right.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    let margin_top = command.margin_top.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    let margin_bottom = command.margin_bottom.unwrap_or(DEFAULT_PRINT_MARGIN_INCHES);
    if ![
        paper_width,
        paper_height,
        margin_left,
        margin_right,
        margin_top,
        margin_bottom,
    ]
    .into_iter()
    .all(|value| value.is_finite() && value >= 0.0)
    {
        return Some("invalid printToPDF page size or margin");
    }
    if paper_width <= margin_left + margin_right || paper_height <= margin_top + margin_bottom {
        return Some("printToPDF paper size is too small for margins");
    }
    None
}

pub(super) async fn execute_devtools_get_layout_metrics_command(
    conn: &mut CdpConnection,
    command: DevToolsGetLayoutMetricsCommand,
) -> Result<AutomationResult, DevToolsError> {
    let owner = page_command_owner(conn, &command.context)?;
    let result =
        execute_devtools_get_layout_metrics_for_current_owner(conn, &owner, command.publish_layout)
            .await;
    result.map(AutomationResult::LayoutMetrics)
}

pub(super) async fn execute_devtools_get_layout_metrics_for_current_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    publish_layout: bool,
) -> Result<DevToolsLayoutMetricsResult, DevToolsError> {
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Err(devtools_layout_metrics_error("NoDocumentLoaded"));
    };
    let pending = page
        .start_layout_metrics_with_publication(publish_layout)
        .map_err(|error| {
            devtools_layout_metrics_error(format!("Failed to start layout metrics: {error}"))
        })?;
    let completed = pending.wait().await.map_err(|error| {
        devtools_layout_metrics_error(format!("Failed to produce layout metrics: {error}"))
    })?;
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_owner(owner)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Err(devtools_layout_metrics_error("NoDocumentLoaded"));
    };
    page.finish_layout_metrics(completed)
        .map(layout_metrics_result_from_renderer)
        .map_err(|error| {
            devtools_layout_metrics_error(format!("Failed to finish layout metrics: {error}"))
        })
}

pub(super) fn layout_metrics_result_from_renderer(
    metrics: RendererLayoutMetrics,
) -> DevToolsLayoutMetricsResult {
    DevToolsLayoutMetricsResult {
        layout_viewport_width: metrics.viewport_width,
        layout_viewport_height: metrics.viewport_height,
        page_x: metrics.page_x,
        page_y: metrics.page_y,
        content_width: metrics.content_width,
        content_height: metrics.content_height,
        device_pixel_ratio: metrics.device_pixel_ratio,
    }
}

pub(super) fn devtools_layout_metrics_error(message: impl Into<String>) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, message)
}

pub(super) fn renderer_screenshot_request(
    command: &DevToolsCaptureScreenshotCommand,
    base_background_color: [u8; 4],
) -> Result<RendererCaptureScreenshotRequest, DevToolsError> {
    let format = match command.format.as_deref() {
        None | Some("png") => RendererScreenshotFormat::Png,
        Some("jpeg") => RendererScreenshotFormat::Jpeg,
        Some(_) => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "Invalid image format",
            ));
        }
    };
    let region = match command.clip.as_ref() {
        Some(DevToolsCaptureScreenshotClip::Box(clip)) => {
            let clip = RendererScreenshotClip {
                x: clip.x,
                y: clip.y,
                width: clip.width,
                height: clip.height,
                scale: clip.scale,
            };
            if command.capture_beyond_viewport {
                RendererScreenshotRegion::PageClip(clip)
            } else {
                RendererScreenshotRegion::ViewportClip(clip)
            }
        }
        Some(DevToolsCaptureScreenshotClip::Element(_)) => {
            return Err(devtools_capture_screenshot_error(command));
        }
        None if command.capture_beyond_viewport => RendererScreenshotRegion::FullDocument,
        None => RendererScreenshotRegion::Viewport,
    };
    Ok(RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Screenshot,
        base_background_color,
        format,
        quality: command.quality.unwrap_or(80),
        region,
        optimize_for_speed: command.optimize_for_speed,
        max_width: None,
        max_height: None,
    })
}

pub(super) async fn execute_devtools_capture_screenshot_command(
    conn: &mut CdpConnection,
    command: DevToolsCaptureScreenshotCommand,
) -> (
    Result<AutomationResult, DevToolsError>,
    Option<moli_core::RendererOutputFence>,
) {
    let mut predecessor = None;
    let result = async {
        validate_page_capture_target_context(conn, &command.context)?;
        // Child-context and element-clip captures retain their unsupported boundary.
        if command.context.target_id.as_ref().is_some_and(|target| {
            conn.target_session_route_for_target_id(target.as_str())
                .is_none()
        }) || !conn.layout_policy().uses_real_layout()
        {
            return Err(devtools_capture_screenshot_error(&command));
        }
        let owner = page_command_owner(conn, &command.context)?;
        let request =
            renderer_screenshot_request(&command, conn.default_background_color_for_owner(&owner))?;
        let capture_error =
            |message| DevToolsError::new(DevToolsErrorKind::UnableToCaptureScreen, message);
        let page = conn
            .loaded_page_mut_for_protocol_access_for_owner(&owner)
            .map_err(capture_error)?;
        let pending = page
            .start_capture_screenshot_with_request(request)
            .map_err(|error| capture_error(error.to_string()))?;
        let completion = pending
            .wait()
            .await
            .map_err(|error| capture_error(error.to_string()))?;
        predecessor = completion.renderer_output_predecessor();
        let page = conn
            .loaded_page_mut_for_protocol_access_for_owner(&owner)
            .map_err(capture_error)?;
        let reply = page
            .finish_capture_screenshot(completion)
            .map_err(|error| capture_error(error.to_string()))?;
        match reply {
            RendererCaptureScreenshotReply::Captured(image) => Ok(
                AutomationResult::CaptureScreenshot(DevToolsCaptureScreenshotResult {
                    mime_type: image.mime_type,
                    width: image.width,
                    height: image.height,
                    bytes: image.bytes,
                }),
            ),
            RendererCaptureScreenshotReply::LayoutDisabled => Err(capture_error(
                CAPTURE_SCREENSHOT_LAYOUT_DISABLED_MESSAGE.to_owned(),
            )),
            RendererCaptureScreenshotReply::NoDocument => {
                Err(capture_error("NoDocumentLoaded".to_owned()))
            }
        }
    }
    .await;
    (result, predecessor)
}

pub(super) fn execute_devtools_print_to_pdf_command(
    conn: &mut CdpConnection,
    command: DevToolsPrintToPdfCommand,
) -> Result<AutomationResult, DevToolsError> {
    validate_page_capture_target_context(conn, &command.context)?;
    Err(devtools_print_to_pdf_error(&command))
}

pub(super) fn validate_page_capture_target_context(
    conn: &CdpConnection,
    context: &AutomationContext,
) -> Result<(), DevToolsError> {
    if let Some(target_id) = context.target_id.as_ref() {
        page_capture_route_for_context_id(conn, target_id.as_str())?;
    }
    Ok(())
}

pub(super) fn page_capture_route_for_context_id(
    conn: &CdpConnection,
    context_id: &str,
) -> Result<CdpSessionRoute, DevToolsError> {
    page_route_for_context_id(conn, context_id)
}

pub(super) fn start_devtools_capture_screenshot_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsCaptureScreenshotCommand,
) -> PageCommandTaskStep {
    if let Err(error) = validate_page_capture_target_context(conn, &command.context) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
    }
    if command.context.protocol != FrontendProtocol::Cdp {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
            devtools_capture_screenshot_error(&command),
        ));
    }
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            CAPTURE_SCREENSHOT_LAYOUT_DISABLED_MESSAGE,
        ));
    }

    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let base_background_color = conn.default_background_color_for_owner(&owner_scope);
    let page = match conn.loaded_page_mut_for_protocol_access(session_id) {
        Ok(page) => page,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let request = match renderer_screenshot_request(&command, base_background_color) {
        Ok(request) => request,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
        }
    };
    match page.start_capture_screenshot_with_request(request) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::CaptureScreenshot { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to start page screenshot: {error}"),
        )),
    }
}

pub(super) fn start_devtools_print_to_pdf_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: DevToolsPrintToPdfCommand,
) -> PageCommandTaskStep {
    if let Err(error) = validate_page_capture_target_context(conn, &command.context) {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
    }
    if command.context.protocol != FrontendProtocol::Cdp {
        return PageCommandTaskStep::Complete(complete_devtools_print_to_pdf_command(
            conn, command,
        ));
    }
    if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            PRINT_TO_PDF_LAYOUT_DISABLED_MESSAGE,
        ));
    }
    let options = match pdf::RasterPdfOptions::from_command(&command) {
        Ok(options) => options,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                error.code(),
                error.message(),
            ));
        }
    };
    let transfer_mode = command
        .transfer_mode
        .unwrap_or(DevToolsPrintToPdfTransferMode::ReturnAsBase64);
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let owner_scope = CommandOwnerScope::capture(conn, session_id);
    let page = match conn.loaded_page_mut_for_protocol_access(session_id) {
        Ok(page) => page,
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    let request = RendererCaptureScreenshotRequest {
        base_background_color: [255; 4],
        purpose: RendererScreenshotPurpose::Print {
            print_background: command.print_background.unwrap_or(false),
        },
        format: RendererScreenshotFormat::Jpeg,
        quality: 90,
        region: RendererScreenshotRegion::FullDocument,
        optimize_for_speed: false,
        max_width: None,
        max_height: None,
    };
    match page.start_capture_screenshot_with_request(request) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::PrintToPdf {
                pending,
                options,
                transfer_mode,
            }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to start PDF capture: {error}"),
        )),
    }
}

pub(super) fn build_cdp_get_layout_metrics_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> crate::automation::DevToolsGetLayoutMetricsCommand {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    crate::automation::DevToolsGetLayoutMetricsCommand {
        context: cmd.automation_context(target_id.as_deref(), browser_context_id.as_deref()),
        publish_layout: cmd
            .params
            .and_then(|params| params.get("publishLayout"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    }
}

pub(super) fn try_start_page_get_layout_metrics_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = build_cdp_get_layout_metrics_command(conn, cmd);
    start_devtools_page_command(conn, cmd.id, AutomationCommand::GetLayoutMetrics(command))
}

pub(super) fn start_devtools_get_layout_metrics_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command: crate::automation::DevToolsGetLayoutMetricsCommand,
) -> PageCommandTaskStep {
    let command_session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let owner_scope = CommandOwnerScope::capture(conn, command_session_id);
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
            devtools_layout_metrics_error("NoDocumentLoaded"),
        ));
    };
    match page.start_layout_metrics_with_publication(command.publish_layout) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope,
            kind: Box::new(PendingPageCommandKind::GetLayoutMetrics { pending }),
        }),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to start layout metrics: {error}"),
        )),
    }
}

pub(super) fn try_start_page_print_to_pdf_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let command = match build_cdp_print_to_pdf_command(conn, cmd) {
        Ok(command) => command,
        Err(plan) => return PageCommandTaskStep::Complete(plan),
    };
    start_devtools_page_command(conn, cmd.id, AutomationCommand::PrintToPdf(command))
}
