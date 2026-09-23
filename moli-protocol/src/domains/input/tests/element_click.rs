use super::*;
use crate::devtools_runtime::{
    DevToolsElementClickCommand, DevToolsElementClickOperation, DevToolsProtocol,
};
use moli_core::page::{
    RendererElementClickError, RendererElementClickTarget, RendererPreparedPointerClick,
};

async fn command(
    ctx: &mut TestContext,
    operation: DevToolsElementClickOperation,
) -> DevToolsCommandResult {
    ctx.conn
        .execute_devtools_command(DevToolsCommand::ElementClick(DevToolsElementClickCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverClassic,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            operation,
        }))
        .await
        .into_parts()
        .0
        .expect("native element click command")
}

async fn prepare(ctx: &mut TestContext) -> RendererPreparedPointerClick {
    let object_id = resolve_selector_object_id(ctx, "#target", 501).await;
    match command(
        ctx,
        DevToolsElementClickOperation::Prepare {
            object_id: object_id.into(),
        },
    )
    .await
    {
        DevToolsCommandResult::ElementClickPreparation(Ok(
            RendererElementClickTarget::Pointer(click),
        )) => click,
        result => panic!("expected prepared pointer click: {result:?}"),
    }
}

async fn fixture() -> TestContext {
    let mut ctx = TestContext::new();
    with_loaded_document(&mut ctx, r#"<html><body>
        <button id='target' style='position:absolute;left:40px;top:40px;width:120px;height:70px'>go</button>
        <script>
        window.clicks=0; window.events=[];
        document.getElementById('target').onclick=()=>clicks++;
        for(const type of ['mousemove','mousedown','mouseup','click'])
            document.addEventListener(type,e=>events.push([type,e.target.id]));
        </script></body></html>"#).await;
    ctx
}

#[tokio::test(flavor = "multi_thread")]
async fn native_element_click_rejects_stale_preparation_before_observable_input() {
    for mutation in [
        "target.remove()",
        "document.open();document.write('<button id=target>replacement</button>');document.close()",
    ] {
        let mut ctx = fixture().await;
        let click = prepare(&mut ctx).await;
        assert!(evaluate_bool(&mut ctx, &format!("(()=>{{const target=document.getElementById('target');{mutation};return true}})()")).await);
        let result = command(&mut ctx, DevToolsElementClickOperation::Dispatch(click)).await;
        let DevToolsCommandResult::ElementClickDispatch(result) = result else {
            panic!("unexpected dispatch result: {result:?}")
        };
        assert_eq!(
            result,
            Err(RendererElementClickError::StaleNode),
            "{mutation}"
        );
        let observed = evaluate_string(&mut ctx, "JSON.stringify([clicks,events])").await;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&observed).unwrap(),
            json!([0, []]),
            "{mutation}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_element_click_stops_after_document_replacement_in_pointer_handler() {
    for event in ["mousemove", "mousedown"] {
        let mut ctx = fixture().await;
        assert!(evaluate_bool(&mut ctx, &format!(r#"(() => {{
            document.getElementById('target').addEventListener('{event}', () => {{
                document.open();document.write('<button id="replacement" style="position:absolute;left:40px;top:40px;width:120px;height:70px">new</button>');document.close();
                window.successorEvents=[];
                for(const type of ['mousemove','mousedown','mouseup','click'])
                    document.getElementById('replacement').addEventListener(type,()=>successorEvents.push(type));
            }});
            return true;
        }})()"#)).await);
        let click = prepare(&mut ctx).await;
        assert!(matches!(
            command(&mut ctx, DevToolsElementClickOperation::Dispatch(click)).await,
            DevToolsCommandResult::ElementClickDispatch(Ok(()))
        ));
        assert_eq!(
            evaluate_string(&mut ctx, "JSON.stringify(successorEvents)").await,
            "[]",
            "{event}"
        );
    }
}
