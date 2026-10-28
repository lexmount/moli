use crate::conn::{CommandWorkflowContext, PreparedRuntimePageQuery, ProtocolWorkflowDriver};

use super::*;

pub(crate) struct PreparedAutomationPageQuery(PageQuery);

enum PageQuery {
    FrameTree(Box<resource_tree::PreparedAutomationFrameTree>),
    FrameTrees(Vec<resource_tree::PreparedAutomationFrameTree>),
    Layout(Box<PreparedRuntimePageQuery<RendererLayoutMetrics>>),
    Screenshot(Box<PreparedRuntimePageQuery<RendererCaptureScreenshotReply>>),
}

impl PreparedAutomationPageQuery {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        command: AutomationCommand,
    ) -> Result<Self, DevToolsError> {
        let query = match command {
            AutomationCommand::GetFrameTree(command) => PageQuery::FrameTree(Box::new(
                resource_tree::PreparedAutomationFrameTree::prepare(conn, command)?,
            )),
            AutomationCommand::GetFrameTrees(command) => {
                let mut queries = Vec::new();
                for info in resource_tree::devtools_browsing_context_target_infos(conn) {
                    let Some(target) = info.target_id else {
                        continue;
                    };
                    queries.push(resource_tree::PreparedAutomationFrameTree::prepare(
                        conn,
                        DevToolsGetFrameTreeCommand {
                            context: AutomationContext {
                                target_id: Some(target),
                                ..command.context.clone()
                            },
                            max_depth: command.max_depth,
                        },
                    )?);
                }
                PageQuery::FrameTrees(queries)
            }
            AutomationCommand::GetLayoutMetrics(command) => {
                let owner = page_command_owner(conn, &command.context)?;
                PageQuery::Layout(Box::new(
                    PreparedRuntimePageQuery::prepare_resident(
                        conn,
                        &owner,
                        "layout metrics query failed",
                        |page| page.start_layout_metrics(),
                        Page::finish_layout_metrics,
                    )
                    .map_err(devtools_layout_metrics_error)?,
                ))
            }
            AutomationCommand::CaptureScreenshot(command) => {
                validate_page_capture_target_context(conn, &command.context)?;
                if command.context.target_id.as_ref().is_some_and(|target| {
                    conn.target_session_route_for_target_id(target.as_str())
                        .is_none()
                }) || !conn.layout_policy().uses_real_layout()
                {
                    return Err(devtools_capture_screenshot_error(&command));
                }
                let owner = page_command_owner(conn, &command.context)?;
                let request = renderer_screenshot_request(
                    &command,
                    conn.default_background_color_for_owner(&owner),
                )?;
                PageQuery::Screenshot(Box::new(
                    PreparedRuntimePageQuery::prepare(
                        conn,
                        &owner,
                        "page screenshot failed",
                        move |page| page.start_capture_screenshot_with_request(request),
                        Page::finish_capture_screenshot,
                    )
                    .map_err(capture_error)?,
                ))
            }
            _ => unreachable!("only admitted Page queries use this preparation"),
        };
        Ok(Self(query))
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        match self.0 {
            PageQuery::FrameTree(query) => {
                query.run(driver).await.map(AutomationResult::GetFrameTree)
            }
            PageQuery::FrameTrees(queries) => {
                let mut frame_trees = Vec::with_capacity(queries.len());
                for query in queries {
                    frame_trees.push(query.run(driver).await?);
                }
                Ok(AutomationResult::GetFrameTrees(
                    DevToolsGetFrameTreesResult { frame_trees },
                ))
            }
            PageQuery::Layout(query) => {
                // Read-only inventories preserve their individual Page FIFO;
                // unrelated Page cursors cannot become one command fence.
                query
                    .run(driver, &mut None)
                    .await
                    .map(layout_metrics_result_from_renderer)
                    .map(AutomationResult::LayoutMetrics)
                    .map_err(devtools_layout_metrics_error)
            }
            PageQuery::Screenshot(query) => {
                let image = query
                    .run(
                        driver,
                        context.context_mut().renderer_output_predecessor_mut(),
                    )
                    .await
                    .map_err(capture_error)?;
                match image {
                    RendererCaptureScreenshotReply::Captured(image) => Ok(
                        AutomationResult::CaptureScreenshot(DevToolsCaptureScreenshotResult {
                            mime_type: image.mime_type,
                            width: image.width,
                            height: image.height,
                            bytes: image.bytes,
                        }),
                    ),
                    RendererCaptureScreenshotReply::LayoutDisabled => {
                        Err(capture_error(ScreenshotLayoutDisabled))
                    }
                    RendererCaptureScreenshotReply::NoDocument => {
                        Err(capture_error(crate::conn::NoDocumentLoaded))
                    }
                }
            }
        }
    }
}

fn capture_error(error: impl Into<anyhow::Error>) -> DevToolsError {
    let error = error.into();
    if error.is::<crate::conn::NoDocumentLoaded>() {
        DevToolsError::new(
            DevToolsErrorKind::UnableToCaptureScreen,
            crate::conn::NoDocumentLoaded,
        )
    } else {
        DevToolsError::new(DevToolsErrorKind::UnableToCaptureScreen, error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{CAPTURE_SCREENSHOT_LAYOUT_DISABLED_MESSAGE}")]
struct ScreenshotLayoutDisabled;
