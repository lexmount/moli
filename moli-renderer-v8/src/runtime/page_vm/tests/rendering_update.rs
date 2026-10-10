use super::*;

mod print;
mod transform_precision;

use base64::Engine as _;

use super::super::main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task;

use crate::page_task_queue::{
    PageRenderingUpdateTargetEffect, RendererPageRenderingUpdateTaskKind,
};
use crate::script_vm::MainDocumentLifecycleBody;

#[tokio::test(flavor = "current_thread")]
async fn interactive_geometry_updates_only_after_visual_publication() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/dynamic-button.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page_vm.vm_mut().eval(
            "document.body.innerHTML = '<button id=first style=\"width:40px;height:20px\">first</button>'; 'ready'",
        )?;
        let first_width: f64 = page_vm
            .vm_mut()
            .eval("document.getElementById('first').getBoundingClientRect().width")?
            .parse()?;
        assert!(first_width > 0.0);
        let first_publish = page_vm.vm().layout_snapshot_cache_observability_for_test().2;

        page_vm.vm_mut().eval(
            "document.body.insertAdjacentHTML('beforeend', '<button id=added style=\"width:60px;height:20px\">add</button>'); 'inserted'",
        )?;
        let added_width_before: f64 = page_vm
            .vm_mut()
            .eval("document.getElementById('added').getBoundingClientRect().width")?
            .parse()?;
        assert_eq!(added_width_before, 0.0);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, first_publish);
        page_vm.vm_mut().publish_layout_for_test()?;
        let dom_publish = page_vm.vm().layout_snapshot_cache_observability_for_test().2;
        assert_eq!(dom_publish, first_publish + 1);
        let added_width: f64 = page_vm
            .vm_mut()
            .eval("document.getElementById('added').getBoundingClientRect().width")?
            .parse()?;
        assert!(added_width > 0.0);
        page_vm
            .vm_mut()
            .eval("document.getElementById('added').getBoundingClientRect().width")?;
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, dom_publish);

        page_vm.vm_mut().eval(
            "document.body.insertAdjacentHTML('beforeend', '<button id=offset style=\"width:50px;height:20px\">offset</button>'); 'inserted'",
        )?;
        let offset_width_before: i32 = page_vm
            .vm_mut()
            .eval("document.getElementById('offset').offsetWidth")?
            .parse()?;
        assert_eq!(offset_width_before, 0);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, dom_publish);
        page_vm.vm_mut().publish_layout_for_test()?;
        let offset_width: i32 = page_vm
            .vm_mut()
            .eval("document.getElementById('offset').offsetWidth")?
            .parse()?;
        assert!(offset_width > 0);
        let metrics_publish = page_vm.vm().layout_snapshot_cache_observability_for_test().2;
        assert_eq!(metrics_publish, dom_publish + 1);

        page_vm.vm_mut().eval(
            "document.body.insertAdjacentHTML('beforeend', '<button id=cdp style=\"width:70px;height:20px\">cdp</button>'); 'inserted'",
        )?;
        let handle = page_vm
            .vm()
            .element_handle_by_id_for_test("cdp")
            .expect("new CDP target");
        assert!(page_vm.vm_mut().client_rect_for_live_node_handle(handle)?.is_none());
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, metrics_publish);
        page_vm.vm_mut().publish_layout_for_test()?;
        let rect = page_vm
            .vm_mut()
            .client_rect_for_live_node_handle(handle)?
            .expect("new CDP target should have geometry");
        assert!(rect.width > 0.0);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, metrics_publish + 1);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("interactive geometry should update only after visual publication");
}

fn viewport_screencast_request(
    known_visual_state: Option<crate::runtime::RendererVisualStateToken>,
) -> crate::runtime::RendererCaptureScreencastFrameRequest {
    crate::runtime::RendererCaptureScreencastFrameRequest {
        base_background_color: [255; 4],
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
mod scrollbars;
mod text_styles;
mod update_lifecycle;
mod visual_output;
