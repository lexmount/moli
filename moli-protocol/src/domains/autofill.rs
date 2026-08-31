use chromiumoxide_cdp::cdp::browser_protocol::autofill::TriggerParams;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererAutofillAddressField,
    RendererAutofillCreditCard, RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
};

use crate::{
    conn::{CdpConnection, Cmd, CommandOwnerScope},
    domains::{actions::AutofillAction, command_output::CommandOutputPlan},
};

pub(crate) struct PendingAutofillCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedAutofillCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    completed: Result<CompletedPageCommand, String>,
}

pub(crate) enum AutofillCommandTaskStep {
    Pending(PendingAutofillCommandDispatch),
    Complete(CommandOutputPlan),
}

impl PendingAutofillCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedAutofillCommandDispatch {
        CompletedAutofillCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            completed: self.pending.wait().await.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedAutofillCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

pub(crate) fn try_start_autofill_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> AutofillCommandTaskStep {
    match cmd.parse_action::<AutofillAction>() {
        Some(AutofillAction::Trigger) => {}
        None => {
            return AutofillCommandTaskStep::Complete(CommandOutputPlan::error(
                -32601,
                "UnknownMethod",
            ));
        }
    }
    if let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id) {
        return AutofillCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
    }
    let params = match cmd.get_params::<TriggerParams>() {
        Ok(Some(params)) => params,
        _ => {
            return AutofillCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "Invalid parameters",
            ));
        }
    };
    let Ok(field_id) = u32::try_from(*params.field_id.inner()) else {
        return AutofillCommandTaskStep::Complete(CommandOutputPlan::error(
            -32602,
            "Invalid parameters",
        ));
    };
    let top_frame_id = conn
        .target_session_owner_frame_tree_identity(cmd.session_id)
        .map(|(frame_id, _, _, _)| frame_id);
    let frame_id = params
        .frame_id
        .map(String::from)
        .filter(|frame_id| Some(frame_id) != top_frame_id.as_ref());
    let card = params.card.map(|card| RendererAutofillCreditCard {
        number: card.number,
        name: card.name,
        expiry_month: card.expiry_month,
        expiry_year: card.expiry_year,
        cvc: card.cvc,
    });
    let address = params.address.map(|address| {
        address
            .fields
            .into_iter()
            .map(|field| RendererAutofillAddressField {
                name: field.name,
                value: field.value,
            })
            .collect()
    });
    let request = RendererAutofillTriggerRequest {
        frame_id,
        field_id,
        card,
        address,
    };
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    let pending = conn
        .loaded_page_mut_for_protocol_access(cmd.session_id)
        .and_then(|page| {
            page.start_autofill_trigger(request)
                .map_err(|error| error.to_string())
        });
    match pending {
        Ok(pending) => AutofillCommandTaskStep::Pending(PendingAutofillCommandDispatch {
            command_id: cmd.id,
            owner_scope,
            pending,
        }),
        Err(message) => {
            AutofillCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message))
        }
    }
}

pub(crate) fn complete_pending_autofill_command(
    conn: &mut CdpConnection,
    completed: CompletedAutofillCommandDispatch,
) -> CommandOutputPlan {
    let session_id = completed.owner_scope.session_id();
    let owner_route = completed.owner_scope.session_owner_route();
    let outcome = completed.completed.and_then(|completion| {
        conn.loaded_page_mut_for_protocol_access_for_route(session_id, owner_route)
            .and_then(|page| {
                page.finish_autofill_trigger(completion)
                    .map_err(|error| error.to_string())
            })
    });
    match outcome {
        Ok(RendererAutofillTriggerOutcome::Applied { .. }) => CommandOutputPlan::success(),
        Ok(RendererAutofillTriggerOutcome::FieldNotFound) => {
            CommandOutputPlan::error(-32600, "Field not found")
        }
        Ok(RendererAutofillTriggerOutcome::FrameNotFound) => {
            CommandOutputPlan::error(-32000, "Frame not found")
        }
        Ok(RendererAutofillTriggerOutcome::CardAndAddressProvided) => {
            CommandOutputPlan::error(-32600, "Card and address cannot both be provided")
        }
        Ok(RendererAutofillTriggerOutcome::MissingCardOrAddress) => {
            CommandOutputPlan::error(-32600, "Either card or address must be provided")
        }
        Ok(RendererAutofillTriggerOutcome::AddressNotSupported) => {
            CommandOutputPlan::error(-32000, "Address autofill is not supported")
        }
        Err(message) => CommandOutputPlan::error(-32000, message),
    }
}

#[cfg(test)]
mod tests {
    use crate::{conn::BrowserContext, testing::TestContext};
    use serde_json::{Value, json};

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut browser_context = BrowserContext::new("BID-1".into());
        browser_context.set_target_url("data:text/html,autofill-test".to_owned());
        browser_context.set_active_target_id("TID-1".to_owned());
        browser_context.attach_active_session("SID-1".to_owned());
        ctx.conn.browser_context = Some(browser_context);
        ctx.install_navigation_fixture_for_session_owner(
            &format!("data:text/html,{html}"),
            Some("SID-1"),
        )
        .await;
        ctx.sent.clear();
    }

    async fn backend_node_id_for_expression(
        ctx: &mut TestContext,
        evaluate_id: u64,
        describe_id: u64,
        expression: &str,
    ) -> u32 {
        ctx.process_async(json!({
            "id": evaluate_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": { "expression": expression }
        }))
        .await;
        let object_id = ctx.take_response_by_id(evaluate_id)["result"]["result"]["objectId"]
            .as_str()
            .expect("Runtime.evaluate should return a node object id")
            .to_owned();
        ctx.process_async(json!({
            "id": describe_id,
            "method": "DOM.describeNode",
            "sessionId": "SID-1",
            "params": { "objectId": object_id }
        }))
        .await;
        u32::try_from(
            ctx.take_response_by_id(describe_id)["result"]["node"]["backendNodeId"]
                .as_u64()
                .expect("DOM.describeNode should return a backend node id"),
        )
        .expect("backend node id should fit u32")
    }

    async fn evaluate_json(ctx: &mut TestContext, id: u64, expression: &str) -> Value {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": { "expression": format!("JSON.stringify({expression})") }
        }))
        .await;
        let response = ctx.take_response_by_id(id);
        serde_json::from_str(
            response["result"]["result"]["value"]
                .as_str()
                .expect("Runtime.evaluate should return a JSON string"),
        )
        .expect("Runtime.evaluate JSON payload should parse")
    }

    fn card_payload() -> Value {
        json!({
            "number": "4444444444444448",
            "name": "T2B Tester",
            "expiryMonth": "12",
            "expiryYear": "2030",
            "cvc": "123"
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trigger_fills_live_card_controls_with_chromium_events_and_state() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            r#"<!doctype html><form>
                <input id=number autocomplete=cc-number>
                <input id=name autocomplete=cc-name>
                <input id=month autocomplete=cc-exp-month>
                <input id=year autocomplete=cc-exp-year>
                <input id=cvc autocomplete=cc-csc>
              </form><script>
                globalThis.autofillEvents = [];
                for (const control of document.querySelectorAll('input')) {
                  for (const type of ['beforeinput', 'input', 'change']) {
                    control.addEventListener(type, event => autofillEvents.push({
                      type, id: control.id, trusted: event.isTrusted,
                      bubbles: event.bubbles, composed: event.composed
                    }));
                  }
                }
              </script>"#,
        )
        .await;
        let field_id =
            backend_node_id_for_expression(&mut ctx, 1, 2, "document.getElementById('number')")
                .await;

        ctx.process_async(json!({
            "id": 3,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": { "fieldId": field_id, "card": card_payload() }
        }))
        .await;
        ctx.expect_result(3, json!({}), Some("SID-1"));

        let payload = evaluate_json(
            &mut ctx,
            4,
            r#"({
              values: Object.fromEntries([...document.querySelectorAll('input')].map(e => [e.id, e.value])),
              attributes: Object.fromEntries([...document.querySelectorAll('input')].map(e => [e.id, e.getAttribute('value')])),
              autofilled: [...document.querySelectorAll('input')].map(e => [e.id, e.matches(':autofill')]),
              active: document.activeElement && document.activeElement.id,
              events: autofillEvents
            })"#,
        )
        .await;
        assert_eq!(
            payload["values"],
            json!({
                "number": "4444444444444448",
                "name": "T2B Tester",
                "month": "12",
                "year": "2030",
                "cvc": "123"
            })
        );
        assert_eq!(
            payload["attributes"],
            json!({ "number": null, "name": null, "month": null, "year": null, "cvc": null })
        );
        assert_eq!(
            payload["autofilled"],
            json!([
                ["number", true],
                ["name", true],
                ["month", true],
                ["year", true],
                ["cvc", true]
            ])
        );
        assert_eq!(payload["active"], json!(""));
        assert_eq!(
            payload["events"],
            json!([
                { "type": "input", "id": "number", "trusted": true, "bubbles": true, "composed": true },
                { "type": "change", "id": "number", "trusted": true, "bubbles": true, "composed": false },
                { "type": "input", "id": "name", "trusted": true, "bubbles": true, "composed": true },
                { "type": "change", "id": "name", "trusted": true, "bubbles": true, "composed": false },
                { "type": "input", "id": "month", "trusted": true, "bubbles": true, "composed": true },
                { "type": "change", "id": "month", "trusted": true, "bubbles": true, "composed": false },
                { "type": "input", "id": "year", "trusted": true, "bubbles": true, "composed": true },
                { "type": "change", "id": "year", "trusted": true, "bubbles": true, "composed": false },
                { "type": "input", "id": "cvc", "trusted": true, "bubbles": true, "composed": true },
                { "type": "change", "id": "cvc", "trusted": true, "bubbles": true, "composed": false }
            ])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trigger_preserves_chromium_validation_order_and_ordinary_field_noop() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><input id=field><div id=notfield></div>",
        )
        .await;
        let field_id =
            backend_node_id_for_expression(&mut ctx, 10, 11, "document.getElementById('field')")
                .await;
        let non_field_id =
            backend_node_id_for_expression(&mut ctx, 12, 13, "document.getElementById('notfield')")
                .await;

        ctx.process_async(json!({
            "id": 14,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": { "fieldId": field_id, "card": card_payload() }
        }))
        .await;
        ctx.expect_result(14, json!({}), Some("SID-1"));
        assert_eq!(
            evaluate_json(
                &mut ctx,
                15,
                "({ value: document.getElementById('field').value, autofilled: document.getElementById('field').matches(':autofill') })",
            )
            .await,
            json!({ "value": "", "autofilled": false })
        );

        ctx.process_async(json!({
            "id": 16,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": {
                "fieldId": non_field_id,
                "card": card_payload(),
                "address": { "fields": [] }
            }
        }))
        .await;
        ctx.expect_error(16, -32600, "Field not found");

        ctx.process_async(json!({
            "id": 17,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": {
                "fieldId": field_id,
                "card": card_payload(),
                "address": { "fields": [] }
            }
        }))
        .await;
        ctx.expect_error(17, -32600, "Card and address cannot both be provided");

        ctx.process_async(json!({
            "id": 18,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": { "fieldId": field_id }
        }))
        .await;
        ctx.expect_error(18, -32600, "Either card or address must be provided");

        ctx.process_async(json!({
            "id": 19,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": {
                "fieldId": field_id,
                "frameId": "missing-frame",
                "card": card_payload()
            }
        }))
        .await;
        ctx.expect_error(19, -32000, "Frame not found");

        ctx.process_async(json!({
            "id": 20,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": {
                "fieldId": field_id,
                "address": { "fields": [] }
            }
        }))
        .await;
        ctx.expect_error(20, -32000, "Address autofill is not supported");

        ctx.process_async(json!({
            "id": 21,
            "method": "Autofill.trigger",
            "sessionId": "SID-1",
            "params": {
                "fieldId": 999_999,
                "card": card_payload()
            }
        }))
        .await;
        ctx.expect_error(21, -32600, "Field not found");
    }
}
