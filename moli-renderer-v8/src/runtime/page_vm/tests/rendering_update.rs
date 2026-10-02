use super::*;

mod print;
mod transform_precision;

use base64::Engine as _;

use super::super::main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task;

use crate::page_task_queue::{
    PageRenderingUpdateTargetEffect, RendererPageRenderingUpdateTaskKind,
};
use crate::script_vm::MainDocumentLifecycleBody;

fn viewport_screencast_request(
    known_visual_state: Option<crate::runtime::RendererVisualStateToken>,
) -> crate::runtime::RendererCaptureScreencastFrameRequest {
    crate::runtime::RendererCaptureScreencastFrameRequest {
        base_background_color: [255; 4],
        vision_deficiency: Default::default(),
        format: crate::runtime::RendererScreenshotFormat::Png,
        quality: 100,
        optimize_for_speed: true,
        max_width: None,
        max_height: None,
        known_visual_state,
    }
}

async fn dispatch_main_document_domcontentloaded_for_rendering_test(
    page_vm: &mut PageVm,
) -> anyhow::Result<crate::frame_owner_model::FrameDocumentTaskOwner> {
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("main Document owner");
    let interactive = page_vm
        .vm_mut()
        .finish_current_main_document_parsing(owner)
        .expect("parser completion should prepare the interactive transition");
    execute_main_document_lifecycle_on_owner_local_task(
        page_vm,
        MainDocumentLifecycleBody::Interactive(interactive),
    )
    .await?;
    execute_main_document_lifecycle_on_owner_local_task(
        page_vm,
        MainDocumentLifecycleBody::DomContentLoaded { owner },
    )
    .await?;
    Ok(owner)
}

mod layout_geometry;
mod painting_tables;
mod text_styles;
mod update_lifecycle;
mod visual_output;
