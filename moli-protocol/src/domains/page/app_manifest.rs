use chromiumoxide_cdp::cdp::browser_protocol::page::GetAppManifestParams;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererAppManifest, RendererAppManifestDisplayMode,
    RendererAppManifestLoadOutcome, RendererAppManifestLoadPreparation,
    RendererAppManifestOrientation, RendererAppManifestQueryResult,
    RendererPreparedAppManifestLoad,
};
use serde_json::{Map, Value, json};

use super::{PageCommandTaskStep, PendingPageCommandDispatch, PendingPageCommandKind};
use crate::{
    conn::CommandDispatchContext,
    conn::{CdpConnection, Cmd, CommandOwnerScope},
    domains::{
        command_output::CommandOutputPlan,
        network::{
            NetworkBacklogProjectionContext,
            emit_pending_network_backlog_activity_background_events,
        },
    },
};

enum PendingGetAppManifestWork {
    Prepare(PendingPageCommand),
    Fetch(Box<RendererPreparedAppManifestLoad>),
    Publish {
        pending: PendingPageCommand,
        result: Box<RendererAppManifestQueryResult>,
    },
}

enum CompletedGetAppManifestWork {
    Prepare(Result<CompletedPageCommand, String>),
    Fetch(Box<RendererAppManifestLoadOutcome>),
    Publish {
        completion: Result<CompletedPageCommand, String>,
        result: Box<RendererAppManifestQueryResult>,
    },
}

pub(super) struct PendingGetAppManifestCommand {
    manifest_id: Option<String>,
    work: PendingGetAppManifestWork,
}

pub(super) struct CompletedGetAppManifestCommand {
    manifest_id: Option<String>,
    work: CompletedGetAppManifestWork,
}

impl CompletedGetAppManifestCommand {
    pub(super) fn renderer_output_predecessor(&self) -> Option<moli_core::RendererOutputFence> {
        match &self.work {
            CompletedGetAppManifestWork::Prepare(Ok(completion))
            | CompletedGetAppManifestWork::Publish {
                completion: Ok(completion),
                ..
            } => completion.renderer_output_predecessor(),
            CompletedGetAppManifestWork::Prepare(Err(_))
            | CompletedGetAppManifestWork::Fetch(_)
            | CompletedGetAppManifestWork::Publish {
                completion: Err(_), ..
            } => None,
        }
    }
}

impl PendingGetAppManifestCommand {
    pub(super) async fn wait(self) -> CompletedGetAppManifestCommand {
        let work = match self.work {
            PendingGetAppManifestWork::Prepare(pending) => CompletedGetAppManifestWork::Prepare(
                pending.wait().await.map_err(|error| error.to_string()),
            ),
            PendingGetAppManifestWork::Fetch(pending) => {
                CompletedGetAppManifestWork::Fetch(Box::new((*pending).execute().await))
            }
            PendingGetAppManifestWork::Publish { pending, result } => {
                CompletedGetAppManifestWork::Publish {
                    completion: pending.wait().await.map_err(|error| error.to_string()),
                    result,
                }
            }
        };
        CompletedGetAppManifestCommand {
            manifest_id: self.manifest_id,
            work,
        }
    }
}

pub(super) fn try_start_get_app_manifest_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: GetAppManifestParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        Ok(None) => GetAppManifestParams::default(),
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32602, error));
        }
    };
    let pending = match conn.loaded_page_mut_for_protocol_access(cmd.session_id) {
        Ok(page) => page.start_prepare_app_manifest_load(),
        Err(message) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
        }
    };
    match pending {
        Ok(pending) => pending_step(
            cmd.id,
            CommandOwnerScope::capture(conn, cmd.session_id),
            PendingGetAppManifestCommand {
                manifest_id: params.manifest_id,
                work: PendingGetAppManifestWork::Prepare(pending),
            },
        ),
        Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            format!("Failed to inspect app manifest: {error}"),
        )),
    }
}

pub(super) fn complete_get_app_manifest_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completed: CompletedGetAppManifestCommand,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    match completed.work {
        CompletedGetAppManifestWork::Prepare(completion) => {
            let completion = match completion {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to inspect app manifest: {message}"),
                    ));
                }
            };
            let preparation = match conn
                .loaded_page_mut_for_protocol_access_for_owner(owner)
                .and_then(|page| {
                    page.finish_prepare_app_manifest_load(completion)
                        .map_err(|error| error.to_string())
                }) {
                Ok(preparation) => preparation,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to inspect app manifest: {message}"),
                    ));
                }
            };
            match preparation {
                RendererAppManifestLoadPreparation::Complete(result) => {
                    PageCommandTaskStep::Complete(result_plan(
                        completed.manifest_id.as_deref(),
                        *result,
                    ))
                }
                RendererAppManifestLoadPreparation::Ready(pending) => pending_step(
                    command_id,
                    owner.clone(),
                    PendingGetAppManifestCommand {
                        manifest_id: completed.manifest_id,
                        work: PendingGetAppManifestWork::Fetch(pending),
                    },
                ),
            }
        }
        CompletedGetAppManifestWork::Fetch(outcome) => {
            let (result, publication) = (*outcome).into_parts();
            let pending = match conn.loaded_page_mut_for_protocol_access_for_owner(owner) {
                Ok(page) => page.start_publish_app_manifest_load(publication),
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            match pending {
                Ok(pending) => pending_step(
                    command_id,
                    owner.clone(),
                    PendingGetAppManifestCommand {
                        manifest_id: completed.manifest_id,
                        work: PendingGetAppManifestWork::Publish {
                            pending,
                            result: Box::new(result),
                        },
                    },
                ),
                Err(error) => PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000,
                    format!("Failed to publish app manifest result: {error}"),
                )),
            }
        }
        CompletedGetAppManifestWork::Publish { completion, result } => {
            let completion = match completion {
                Ok(completion) => completion,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        format!("Failed to publish app manifest result: {message}"),
                    ));
                }
            };
            let output = match conn
                .loaded_page_mut_for_protocol_access_for_owner(owner)
                .and_then(|page| {
                    page.finish_publish_app_manifest_load(completion)
                        .map_err(|error| error.to_string())
                }) {
                Ok(output) => output,
                Err(message) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
            };
            let mut ordered_events = Vec::new();
            command_context.consume_renderer_command_turn_output(output);
            conn.ingest_runtime_session_owner_output_updates_for_owner(owner);
            emit_pending_network_backlog_activity_background_events(
                conn,
                &mut ordered_events,
                NetworkBacklogProjectionContext::for_owner(owner),
            );
            let mut plan = CommandOutputPlan::default();
            plan.extend_background_events(ordered_events);
            plan.extend(result_plan(completed.manifest_id.as_deref(), *result));
            PageCommandTaskStep::Complete(plan)
        }
    }
}

fn pending_step(
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    pending: PendingGetAppManifestCommand,
) -> PageCommandTaskStep {
    PageCommandTaskStep::Pending(PendingPageCommandDispatch {
        command_id,
        owner_scope,
        kind: Box::new(PendingPageCommandKind::GetAppManifest(pending)),
    })
}

fn result_plan(
    manifest_id: Option<&str>,
    result: RendererAppManifestQueryResult,
) -> CommandOutputPlan {
    if manifest_id.is_some_and(|expected| expected != result.manifest.id) {
        return CommandOutputPlan::error(
            -32602,
            format!(
                "Page manifest id {} does not match the input {}",
                result.manifest.id,
                manifest_id.unwrap_or_default()
            ),
        );
    }
    let RendererAppManifestQueryResult {
        url,
        errors,
        data,
        manifest,
    } = result;
    let scope = manifest.scope.clone();
    let mut response = Map::new();
    response.insert("url".to_owned(), json!(url));
    response.insert(
        "errors".to_owned(),
        json!(
            errors
                .into_iter()
                .map(|error| json!({
                    "message": error.message,
                    "critical": error.critical,
                    "line": error.line,
                    "column": error.column,
                }))
                .collect::<Vec<_>>()
        ),
    );
    if let Some(data) = data {
        response.insert("data".to_owned(), json!(data));
    }
    response.insert("parsed".to_owned(), json!({ "scope": scope }));
    response.insert("manifest".to_owned(), manifest_json(manifest));
    CommandOutputPlan::result(Value::Object(response))
}

fn manifest_json(manifest: RendererAppManifest) -> Value {
    let mut output = Map::new();
    insert_optional(&mut output, "backgroundColor", manifest.background_color);
    insert_optional(&mut output, "description", manifest.description);
    output.insert(
        "display".to_owned(),
        json!(display_mode_label(manifest.display)),
    );
    if !manifest.display_overrides.is_empty() {
        output.insert(
            "displayOverrides".to_owned(),
            json!(
                manifest
                    .display_overrides
                    .into_iter()
                    .map(display_mode_label)
                    .collect::<Vec<_>>()
            ),
        );
    }
    if !manifest.icons.is_empty() {
        output.insert(
            "icons".to_owned(),
            json!(
                manifest
                    .icons
                    .into_iter()
                    .map(|icon| json!({
                        "url": icon.url,
                        "sizes": icon.sizes,
                        "type": icon.mime_type,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    output.insert("id".to_owned(), json!(manifest.id));
    insert_optional(&mut output, "name", manifest.name);
    output.insert(
        "orientation".to_owned(),
        json!(orientation_label(manifest.orientation)),
    );
    output.insert(
        "preferRelatedApplications".to_owned(),
        json!(manifest.prefer_related_applications),
    );
    if !manifest.protocol_handlers.is_empty() {
        output.insert(
            "protocolHandlers".to_owned(),
            json!(
                manifest
                    .protocol_handlers
                    .into_iter()
                    .map(|handler| json!({
                        "protocol": handler.protocol,
                        "url": handler.url,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    if !manifest.related_applications.is_empty() {
        output.insert(
            "relatedApplications".to_owned(),
            json!(
                manifest
                    .related_applications
                    .into_iter()
                    .map(|application| {
                        let mut value = Map::new();
                        if let Some(id) = application.id {
                            value.insert("id".to_owned(), json!(id));
                        }
                        value.insert("url".to_owned(), json!(application.url));
                        Value::Object(value)
                    })
                    .collect::<Vec<_>>()
            ),
        );
    }
    output.insert("scope".to_owned(), json!(manifest.scope));
    if !manifest.shortcuts.is_empty() {
        output.insert(
            "shortcuts".to_owned(),
            json!(
                manifest
                    .shortcuts
                    .into_iter()
                    .map(|shortcut| json!({ "name": shortcut.name, "url": shortcut.url }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    output.insert("startUrl".to_owned(), json!(manifest.start_url));
    insert_optional(&mut output, "themeColor", manifest.theme_color);
    Value::Object(output)
}

fn insert_optional(output: &mut Map<String, Value>, name: &str, value: Option<String>) {
    if let Some(value) = value {
        output.insert(name.to_owned(), json!(value));
    }
}

fn display_mode_label(mode: RendererAppManifestDisplayMode) -> &'static str {
    match mode {
        RendererAppManifestDisplayMode::Undefined => "kUndefined",
        RendererAppManifestDisplayMode::Browser => "kBrowser",
        RendererAppManifestDisplayMode::MinimalUi => "kMinimalUi",
        RendererAppManifestDisplayMode::Standalone => "kStandalone",
        RendererAppManifestDisplayMode::Fullscreen => "kFullscreen",
        RendererAppManifestDisplayMode::WindowControlsOverlay => "kWindowControlsOverlay",
        RendererAppManifestDisplayMode::Tabbed => "kTabbed",
        RendererAppManifestDisplayMode::Borderless => "kBorderless",
        RendererAppManifestDisplayMode::PictureInPicture => "kPictureInPicture",
    }
}

fn orientation_label(orientation: RendererAppManifestOrientation) -> &'static str {
    match orientation {
        RendererAppManifestOrientation::Default => "DEFAULT",
        RendererAppManifestOrientation::Any => "ANY",
        RendererAppManifestOrientation::Natural => "NATURAL",
        RendererAppManifestOrientation::Landscape => "LANDSCAPE",
        RendererAppManifestOrientation::LandscapePrimary => "LANDSCAPE_PRIMARY",
        RendererAppManifestOrientation::LandscapeSecondary => "LANDSCAPE_SECONDARY",
        RendererAppManifestOrientation::Portrait => "PORTRAIT",
        RendererAppManifestOrientation::PortraitPrimary => "PORTRAIT_PRIMARY",
        RendererAppManifestOrientation::PortraitSecondary => "PORTRAIT_SECONDARY",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> RendererAppManifestQueryResult {
        RendererAppManifestQueryResult {
            url: String::new(),
            errors: Vec::new(),
            data: Some(String::new()),
            manifest: RendererAppManifest {
                background_color: None,
                description: None,
                display: RendererAppManifestDisplayMode::Undefined,
                display_overrides: Vec::new(),
                icons: Vec::new(),
                id: "https://example.test/app/page".to_owned(),
                name: None,
                orientation: RendererAppManifestOrientation::Default,
                prefer_related_applications: false,
                protocol_handlers: Vec::new(),
                related_applications: Vec::new(),
                scope: "https://example.test/app/".to_owned(),
                shortcuts: Vec::new(),
                start_url: "https://example.test/app/page".to_owned(),
                theme_color: None,
            },
        }
    }

    #[test]
    fn default_manifest_serializes_chromium_shape() {
        let plan = result_plan(None, result());
        let mut messages = Vec::new();
        plan.emit_into(&mut messages, Some(41), None);
        assert_eq!(
            messages,
            vec![json!({
                "id": 41,
                "result": {
                    "url": "",
                    "errors": [],
                    "data": "",
                    "parsed": {"scope": "https://example.test/app/"},
                    "manifest": {
                        "display": "kUndefined",
                        "id": "https://example.test/app/page",
                        "orientation": "DEFAULT",
                        "preferRelatedApplications": false,
                        "scope": "https://example.test/app/",
                        "startUrl": "https://example.test/app/page",
                    },
                },
            })]
        );
    }

    #[test]
    fn manifest_id_mismatch_uses_invalid_params_error() {
        let plan = result_plan(Some("https://example.test/wrong"), result());
        let mut messages = Vec::new();
        plan.emit_into(&mut messages, Some(42), None);
        assert_eq!(messages[0]["error"]["code"], json!(-32602));
        assert_eq!(
            messages[0]["error"]["message"],
            json!(
                "Page manifest id https://example.test/app/page does not match the input https://example.test/wrong"
            )
        );
    }
}
