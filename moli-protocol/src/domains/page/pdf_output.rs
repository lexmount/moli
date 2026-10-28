use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_core::page::{CompletedPageCommand, RendererPrintToPdfReply};
use serde_json::json;

use crate::automation::DevToolsPrintToPdfTransferMode;
use crate::conn::{
    CapturedBody, CdpConnection, CommandOwnerScope, NoDocumentLoaded, ProtocolWorkflowDriver,
    RuntimePageScope,
};
use crate::domains::command_output::CommandOutputPlan;

pub(crate) struct PreparedPdfOutput {
    owner: RuntimePageScope,
    bytes: Vec<u8>,
    transfer_mode: DevToolsPrintToPdfTransferMode,
}

enum PdfOutput {
    Base64(String),
    Stream(CapturedBody),
}

impl PreparedPdfOutput {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        completed: Result<CompletedPageCommand>,
        transfer_mode: DevToolsPrintToPdfTransferMode,
    ) -> Result<Self, CommandOutputPlan> {
        let completion = completed.map_err(capture_error)?;
        let scope = RuntimePageScope::capture(conn, &owner)
            .map_err(|error| CommandOutputPlan::error(-32000, error))?;
        let page = conn
            .loaded_page_mut_for_protocol_access_for_owner(scope.owner())
            .map_err(|error| CommandOutputPlan::error(-32000, error))?;
        let bytes = match page
            .finish_print_to_pdf(completion)
            .map_err(capture_error)?
        {
            RendererPrintToPdfReply::Printed(bytes) => bytes,
            RendererPrintToPdfReply::Failed(message) => {
                return Err(CommandOutputPlan::error(-32000, message));
            }
            RendererPrintToPdfReply::LayoutDisabled => {
                return Err(CommandOutputPlan::error(
                    -32000,
                    super::PRINT_TO_PDF_LAYOUT_DISABLED_MESSAGE,
                ));
            }
            RendererPrintToPdfReply::NoDocument => {
                return Err(CommandOutputPlan::error(-32000, NoDocumentLoaded));
            }
        };
        Ok(Self {
            owner: scope,
            bytes,
            transfer_mode,
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
    ) -> Result<CommandOutputPlan> {
        let Self {
            owner,
            bytes,
            transfer_mode,
        } = self;
        let output = crate::conn::run_body_storage_work(move || {
            Ok(match transfer_mode {
                DevToolsPrintToPdfTransferMode::ReturnAsBase64 => {
                    PdfOutput::Base64(BASE64_STANDARD.encode(bytes))
                }
                DevToolsPrintToPdfTransferMode::ReturnAsStream => {
                    PdfOutput::Stream(CapturedBody::from_bytes_spooled(bytes))
                }
            })
        })
        .await?;
        match output {
            PdfOutput::Base64(data) => Ok(CommandOutputPlan::result(json!({ "data": data }))),
            PdfOutput::Stream(body) => {
                let result =
                    driver.with_state(move |state| state.open_io_stream_for_page(owner, body))?;
                Ok(match result {
                    Ok(stream) => {
                        CommandOutputPlan::result(json!({ "data": "", "stream": stream }))
                    }
                    Err(error) => CommandOutputPlan::error(-32000, error),
                })
            }
        }
    }
}

fn capture_error(error: impl std::fmt::Display) -> CommandOutputPlan {
    CommandOutputPlan::error(-32000, format!("Failed to capture PDF content: {error}"))
}
