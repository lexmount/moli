//! Native PDF export through the shared renderer print lifecycle.

use anyhow::{Context, Result, bail};
use moli_core::page::{Page, RendererPrintToPdfReply, RendererPrintToPdfRequest};

pub(super) async fn render_pdf(page: &mut Page) -> Result<Vec<u8>> {
    let pending = page
        .start_print_to_pdf(RendererPrintToPdfRequest {
            // CLI exports preserve page artwork, including white-on-color controls.
            print_background: true,
            ..Default::default()
        })
        .context("failed to start --dump pdf capture")?;
    let completion = pending
        .wait()
        .await
        .context("failed to complete --dump pdf capture")?;
    match page.finish_print_to_pdf(completion)? {
        RendererPrintToPdfReply::Printed(bytes) => Ok(bytes),
        RendererPrintToPdfReply::Failed(error) => bail!("{error}"),
        RendererPrintToPdfReply::LayoutDisabled => {
            bail!(
                "--dump pdf requires --lazy or --realtime (MOLI_LAYOUT=true or MOLI_REALTIME=true)"
            )
        }
        RendererPrintToPdfReply::NoDocument => {
            bail!("--dump pdf requires a loaded HTML document")
        }
    }
}
