use chromiumoxide_cdp::cdp::browser_protocol::dom::{
    DiscardSearchResultsParams, GetSearchResultsParams, PerformSearchParams,
};
use serde_json::json;

use super::resolve::{
    DomCommandOutput, DomCommandTaskStep, PendingDomCommandDispatch, PendingDomCommandKind,
    PendingDomCommandStartError, PendingDomCommandWork,
};
use super::*;
use crate::devtools_runtime::{
    DevToolsDiscardSearchResultsCommand, DevToolsGetSearchResultsCommand,
    DevToolsPerformSearchCommand,
};
use moli_core::page::{CompletedPageCommand, RendererDomSearchResultsResolution};

pub(super) fn build_cdp_perform_search_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsPerformSearchCommand, PendingDomCommandStartError> {
    let params: PerformSearchParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsPerformSearchCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        query: params.query,
        include_user_agent_shadow_dom: params.include_user_agent_shadow_dom.unwrap_or(false),
    })
}

pub(super) fn build_cdp_get_search_results_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Result<DevToolsGetSearchResultsCommand, PendingDomCommandStartError> {
    let params: GetSearchResultsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return Err(PendingDomCommandStartError::invalid_params()),
    };
    let (Ok(from_index), Ok(to_index)) = (
        usize::try_from(params.from_index),
        usize::try_from(params.to_index),
    ) else {
        return Err(PendingDomCommandStartError {
            code: -32000,
            message: "Invalid search result range".to_owned(),
        });
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Ok(DevToolsGetSearchResultsCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        search_id: params.search_id,
        from_index,
        to_index,
    })
}

pub(super) fn build_cdp_discard_search_results_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<DevToolsDiscardSearchResultsCommand> {
    let params: DiscardSearchResultsParams = cmd.get_params().ok().flatten()?;
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    Some(DevToolsDiscardSearchResultsCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        search_id: params.search_id,
    })
}

pub(super) fn start_devtools_perform_search_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsPerformSearchCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let include_whitespace = super::dom_agent_includes_whitespace_for_route(
        conn,
        owner.session_id(),
        owner.session_owner_route(),
    );
    let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
        return Ok(None);
    };
    let pending = page
        .start_document_perform_search(
            renderer_inspector_session_id,
            command.query,
            command.include_user_agent_shadow_dom,
            include_whitespace,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;

    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::PerformSearchLive,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) fn start_devtools_get_search_results_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsGetSearchResultsCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
        return Ok(None);
    };
    let pending = page
        .start_document_search_results(
            renderer_inspector_session_id,
            command.search_id,
            command.from_index,
            command.to_index,
        )
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::GetSearchResultsLive,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) fn start_devtools_discard_search_results_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    command: DevToolsDiscardSearchResultsCommand,
) -> Result<Option<PendingDomCommandDispatch>, PendingDomCommandStartError> {
    let renderer_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
        return Ok(None);
    };
    let pending = page
        .start_discard_document_search_results(renderer_inspector_session_id, command.search_id)
        .map_err(PendingDomCommandStartError::renderer_error)?;
    Ok(Some(PendingDomCommandDispatch {
        command_id,
        owner_scope: owner.clone(),
        kind: PendingDomCommandKind::DiscardSearchResultsLive,
        pending: PendingDomCommandWork::Page(pending),
    }))
}

pub(super) fn complete_non_pending_perform_search_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    out: &mut DomCommandOutput,
) -> bool {
    let command = match build_cdp_perform_search_command(conn, cmd) {
        Ok(command) => command,
        Err(error) => {
            out.push_error(error.code, error.message);
            return true;
        }
    };
    let session_id = command.context.session_id.as_ref().map(|id| id.as_str());
    let owner = CommandOwnerScope::capture(conn, session_id);
    if super::loaded_page_mut_for_owner(conn, &owner).is_some() {
        out.push_error(-32000, "MissingDomCommand");
        return true;
    }
    out.push_result(json!({ "searchId": "0", "resultCount": 0 }));
    true
}

pub(super) fn complete_non_pending_discard_search_results_command(
    out: &mut DomCommandOutput,
) -> bool {
    out.push_success();
    true
}

pub(super) fn complete_perform_search_live(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let search = {
        let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
            out.push_error(-32000, "NoDocumentLoaded");
            return DomCommandTaskStep::Complete;
        };
        match page.finish_document_perform_search(completion) {
            Ok(search) => search,
            Err(error) => {
                out.push_error(-32000, format!("Could not perform DOM search: {error}"));
                return DomCommandTaskStep::Complete;
            }
        }
    };
    out.push_result(json!({
        "searchId": search.search_id,
        "resultCount": search.result_count
    }));
    DomCommandTaskStep::Complete
}

pub(super) fn complete_get_search_results_live(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let resolution = {
        let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
            out.push_error(-32000, "NoDocumentLoaded");
            return DomCommandTaskStep::Complete;
        };
        match page.finish_document_search_results(completion) {
            Ok(resolution) => resolution,
            Err(error) => {
                out.push_error(-32000, format!("Could not get DOM search results: {error}"));
                return DomCommandTaskStep::Complete;
            }
        }
    };
    match resolution {
        RendererDomSearchResultsResolution::Found(nodes) => {
            let node_ids = nodes
                .into_iter()
                .map(|node| node.frontend_node_id)
                .collect::<Vec<_>>();
            out.push_result(json!({ "nodeIds": node_ids }));
        }
        RendererDomSearchResultsResolution::SearchResultNotFound => {
            out.push_error(-32000, "No search session with given id found");
        }
        RendererDomSearchResultsResolution::BadIndices
        | RendererDomSearchResultsResolution::BadFromIndex
        | RendererDomSearchResultsResolution::BadToIndex => {
            out.push_error(-32000, "Invalid search result range");
        }
    }
    DomCommandTaskStep::Complete
}

pub(super) fn complete_discard_search_results_live(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    completion: CompletedPageCommand,
    out: &mut DomCommandOutput,
) -> DomCommandTaskStep {
    let Some(page) = super::loaded_page_mut_for_owner(conn, owner) else {
        out.push_success();
        return DomCommandTaskStep::Complete;
    };
    if let Err(error) = page.finish_discard_document_search_results(completion) {
        out.push_error(
            -32000,
            format!("Could not discard DOM search results: {error}"),
        );
        return DomCommandTaskStep::Complete;
    }
    out.push_success();
    DomCommandTaskStep::Complete
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::DevToolsProtocol;
    use serde_json::{Value, json};

    use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};

    #[test]
    fn cdp_perform_search_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "query": "article.result",
            "includeUserAgentShadowDOM": true
        });
        let cmd = Cmd::for_test(
            Some(92),
            "DOM.performSearch",
            &params,
            Some("SID-dom"),
            r#"{"id":92,"method":"DOM.performSearch"}"#,
        );

        let command = super::build_cdp_perform_search_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid performSearch command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(command.query, "article.result");
        assert!(command.include_user_agent_shadow_dom);
    }

    #[test]
    fn devtools_dom_entry_keeps_perform_search_without_loaded_page_on_sync_empty_path() {
        let mut conn = CdpConnection::new();
        let params = json!({
            "query": ".hit"
        });
        let cmd = Cmd::for_test(
            Some(93),
            "DOM.performSearch",
            &params,
            Some("SID-dom"),
            r#"{"id":93,"method":"DOM.performSearch"}"#,
        );
        let command = super::build_cdp_perform_search_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid performSearch command");
        };

        let owner = CommandOwnerScope::for_session("SID-dom");
        let result =
            super::start_devtools_perform_search_command(&mut conn, cmd.id, &owner, command);

        let Ok(None) = result else {
            panic!("performSearch without a loaded page should stay on sync empty-result path");
        };
    }

    #[test]
    fn cdp_get_search_results_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "searchId": "search-1",
            "fromIndex": 2,
            "toIndex": 5
        });
        let cmd = Cmd::for_test(
            Some(110),
            "DOM.getSearchResults",
            &params,
            Some("SID-dom"),
            r#"{"id":110,"method":"DOM.getSearchResults"}"#,
        );

        let command = super::build_cdp_get_search_results_command(&conn, &cmd);
        let Ok(command) = command else {
            panic!("valid getSearchResults command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(command.search_id, "search-1");
        assert_eq!(command.from_index, 2);
        assert_eq!(command.to_index, 5);
    }

    #[test]
    fn cdp_discard_search_results_builds_protocol_neutral_command() {
        let conn = CdpConnection::new();
        let params = json!({
            "searchId": "search-2"
        });
        let cmd = Cmd::for_test(
            Some(111),
            "DOM.discardSearchResults",
            &params,
            Some("SID-dom"),
            r#"{"id":111,"method":"DOM.discardSearchResults"}"#,
        );

        let command = super::build_cdp_discard_search_results_command(&conn, &cmd);
        let Some(command) = command else {
            panic!("valid discardSearchResults command");
        };

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-dom")
        );
        assert_eq!(command.search_id, "search-2");
    }

    #[test]
    fn cdp_discard_search_results_keeps_invalid_params_as_noop_success_path() {
        let conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(112),
            "DOM.discardSearchResults",
            &params,
            Some("SID-dom"),
            r#"{"id":112,"method":"DOM.discardSearchResults"}"#,
        );

        let command = super::build_cdp_discard_search_results_command(&conn, &cmd);

        assert!(command.is_none());
    }
}
