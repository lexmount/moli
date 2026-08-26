use super::graph::TabTarget;
use crate::devtools_runtime::{DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind};

pub(crate) fn tab_target_info_from_page_target_info(
    target: &TabTarget,
    page_target_info: DevToolsTargetInfo,
) -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: Some(DevToolsTargetId::from(target.id())),
        kind: DevToolsTargetKind::Tab,
        title: page_target_info.title,
        url: page_target_info.url,
        attached: target.has_session(),
        // Chromium's tab DevToolsAgentHost delegates opener identity and
        // access to its primary frame host. Preserve the same relationship
        // when projecting our page target into a tab target.
        opener_id: page_target_info.opener_id,
        opener_frame_id: page_target_info.opener_frame_id,
        can_access_opener: page_target_info.can_access_opener,
        browser_context_id: page_target_info.browser_context_id,
        moli_popup_id: None,
    }
}

pub(crate) fn project_page_tab_target_infos_for_destruction(
    target: Option<&TabTarget>,
    target_info: DevToolsTargetInfo,
) -> Vec<DevToolsTargetInfo> {
    let mut target_infos = vec![target_info.clone()];
    if target_info.kind == DevToolsTargetKind::Page
        && let Some(target) = target
    {
        target_infos.push(tab_target_info_from_page_target_info(target, target_info));
    }
    target_infos
}

#[cfg(test)]
mod tests {
    use crate::devtools_runtime::{
        DevToolsBrowserContextId, DevToolsFrameId, DevToolsTargetId, DevToolsTargetInfo,
        DevToolsTargetKind,
    };

    use super::super::graph::TargetGraph;
    use super::tab_target_info_from_page_target_info;

    #[test]
    fn tab_projection_preserves_noopener_creator_identity_and_access_policy() {
        let mut graph = TargetGraph::default();
        graph.register_tab("TID-tab".to_owned(), "TID-page".to_owned());
        let target = graph
            .tab_for_page_target_id("TID-page")
            .expect("registered tab target");
        let tab = tab_target_info_from_page_target_info(
            target,
            DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("TID-page")),
                kind: DevToolsTargetKind::Page,
                title: String::new(),
                url: "about:blank".to_owned(),
                attached: true,
                opener_id: Some(DevToolsTargetId::from("TID-opener")),
                opener_frame_id: Some(DevToolsFrameId::from("FRAME-opener")),
                can_access_opener: false,
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-1")),
                moli_popup_id: None,
            },
        );

        assert_eq!(tab.opener_id.unwrap().as_str(), "TID-opener");
        assert_eq!(tab.opener_frame_id.unwrap().as_str(), "FRAME-opener");
        assert!(!tab.can_access_opener);
    }
}
