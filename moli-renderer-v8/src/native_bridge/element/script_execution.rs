use crate::{
    content_security_policy::ContentSecurityPolicyScriptElementRequest,
    document_runtime::DomHandle,
    frame_owner_model::{FrameScriptJob, FrameScriptSource},
    native_bridge::JsContextHost,
};

use super::trusted_types::prepare_trusted_script_text;

pub(crate) fn prepare_inline_classic_frame_script_job_for_execution(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    job: &mut FrameScriptJob,
) -> anyhow::Result<bool> {
    if !job.needs_inline_classic_element_preparation() {
        return Ok(true);
    }
    let script = job
        .current_script
        .expect("inline classic element preparation requires a backing script element");
    let parser_inserted = job
        .inline_classic_parser_inserted()
        .expect("inline classic element preparation requires an inline classic job");
    #[cfg(not(test))]
    let FrameScriptSource::SourceText(source) = &job.source;
    #[cfg(test)]
    let source = match &job.source {
        FrameScriptSource::SourceText(source) => source,
        FrameScriptSource::FunctionConstructor(_) => {
            return Err(anyhow::anyhow!(
                "inline classic frame script job does not carry source text"
            ));
        }
    };
    let Some(source) = inline_script_source_for_execution(
        scope,
        host_ptr,
        script,
        source,
        ContentSecurityPolicyScriptElementRequest {
            nonce: job.script_nonce.as_deref(),
            integrity: job.script_integrity.as_deref(),
            parser_inserted,
        },
    ) else {
        return Ok(false);
    };
    job.source = FrameScriptSource::SourceText(source);
    Ok(true)
}

pub(crate) fn inline_script_source_for_execution(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    script: DomHandle,
    source: &str,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> Option<String> {
    let source = prepare_trusted_script_text(scope, host_ptr, script, source)?;
    let host = unsafe { &mut *host_ptr };
    let Some(owner) = host.owner_dispatch_scope_for_node(script) else {
        // A script with no live owner has no document policy container to
        // enforce. As with the other owner-scoped CSP checks, fail open rather
        // than applying the top document's policy to the wrong context.
        return Some(source);
    };
    host.allows_inline_script_element_by_csp(scope, owner, script, &source, request)
        .then_some(source)
}
