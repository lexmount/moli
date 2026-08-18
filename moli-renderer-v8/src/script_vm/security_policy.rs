use std::pin::pin;

use anyhow::Result;

use super::ScriptVm;
use crate::{
    context_bootstrap::{TrustedTypesCodeGenerationCheck, trusted_types_code_generation_check},
    document_runtime::DocumentContentSecurityPolicyViolation,
    native_bridge::JsContextHost,
    page_task_queue::ContentSecurityPolicyViolationEventTask,
    util::context_host_ptr_from_global_bridge,
};

/// Body-only result for one queued `securitypolicyviolation` task.
///
/// Exact-Document rejection happens before entering V8. Once dispatch is
/// attempted, its `Result` remains separate so a best-effort failure cannot be
/// mistaken for a stale task that owes no task-end checkpoint.
pub(super) enum ContentSecurityPolicyViolationBodyExecution {
    DiscardedStaleDocument,
    DispatchAttempted(Result<()>),
}

impl ScriptVm {
    pub(crate) fn queue_content_security_policy_violation_event_best_effort(
        &mut self,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Err(error) = self.queue_content_security_policy_violation_event(violation) {
            self.record_runtime_warning(format_args!(
                "securitypolicyviolation queueing failed for `{}`: {error}",
                violation.blocked_uri
            ));
        }
    }

    fn queue_content_security_policy_violation_event(
        &mut self,
        violation: &DocumentContentSecurityPolicyViolation,
    ) -> Result<()> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*self._context_host).as_ptr();
                self.document_runtime
                    .queue_content_security_policy_violation_event(scope, host_ptr, violation)?;
                Ok(())
            })
    }

    #[cfg(test)]
    pub(super) fn dispatch_content_security_policy_violation_event_page_task_best_effort(
        &mut self,
        task: &ContentSecurityPolicyViolationEventTask,
    ) {
        if let Err(error) = self.dispatch_content_security_policy_violation_event_page_task(task) {
            self.record_runtime_warning(format_args!(
                "securitypolicyviolation page task failed for `{}`: {error}",
                task.violation().blocked_uri
            ));
        }
    }

    #[cfg(test)]
    fn dispatch_content_security_policy_violation_event_page_task(
        &mut self,
        task: &ContentSecurityPolicyViolationEventTask,
    ) -> Result<()> {
        let ContentSecurityPolicyViolationBodyExecution::DispatchAttempted(result) =
            self.dispatch_content_security_policy_violation_event_body(task)
        else {
            return Ok(());
        };
        result?;
        self.perform_owner_lane_task_microtask_checkpoints()
    }

    /// Dispatch the event body without ending the surrounding Page task.
    pub(super) fn dispatch_content_security_policy_violation_event_body(
        &mut self,
        task: &ContentSecurityPolicyViolationEventTask,
    ) -> ContentSecurityPolicyViolationBodyExecution {
        if self.current_main_document_task_owner() != Some(task.owner()) {
            return ContentSecurityPolicyViolationBodyExecution::DiscardedStaleDocument;
        }
        let result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*self._context_host).as_ptr();
                self.document_runtime
                    .dispatch_content_security_policy_violation_event_page_task(
                        scope, host_ptr, task,
                    )?;
                Ok(())
            });
        ContentSecurityPolicyViolationBodyExecution::DispatchAttempted(result)
    }

    pub(crate) fn set_response_content_security_policies(&mut self, policies: &[String]) {
        self.document_runtime
            .set_response_content_security_policies(policies);
    }

    pub(crate) fn set_bypass_content_security_policy(&mut self, bypass: bool) {
        self.document_runtime
            .set_bypass_content_security_policy(bypass);
    }

    pub(crate) fn set_response_content_security_report_only_policies(
        &mut self,
        policies: &[String],
    ) {
        self.document_runtime
            .set_response_content_security_report_only_policies(policies);
    }

    pub(crate) fn set_response_referrer_policy(&mut self, policy: Option<String>) {
        self.document_runtime.set_response_referrer_policy(policy);
    }

    pub(crate) fn set_cross_origin_embedder_policy(
        &mut self,
        policy: crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
    ) {
        self.document_runtime
            .set_cross_origin_embedder_policy(policy);
    }

    pub(crate) fn set_document_isolation_policy(
        &mut self,
        policy: crate::cross_origin_isolation::DocumentIsolationPolicy,
    ) {
        self.document_runtime.set_document_isolation_policy(policy);
    }

    pub(crate) fn set_content_security_reporting_endpoints(
        &mut self,
        endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    ) {
        self.document_runtime
            .set_content_security_reporting_endpoints(endpoints);
    }
}

pub(super) unsafe extern "C" fn wasm_code_generation_check_callback(
    context: v8::Local<'_, v8::Context>,
    _source: v8::Local<'_, v8::String>,
) -> bool {
    v8::callback_scope!(unsafe scope, context);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    let host = unsafe { &mut *host_ptr };
    host.allows_wasm_code_generation_by_csp(scope)
}

pub(super) unsafe extern "C" fn string_code_generation_check_callback(
    context: v8::Local<'_, v8::Context>,
    source: v8::Local<'_, v8::Value>,
    is_code_like: bool,
    modified_source: *mut *const v8::String,
) -> bool {
    v8::callback_scope!(unsafe scope, context);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    let requires_trusted_types_for_script =
        unsafe { &*host_ptr }.requires_trusted_types_for_script(scope);
    let action = if requires_trusted_types_for_script {
        if unsafe { &*host_ptr }.allows_trusted_types_eval(scope) {
            // The keyword relaxes Trusted Types conversion, but it does not
            // override another CSP policy. The per-policy CSP gate still runs.
            if source.is_string() {
                StringCodeGenerationPolicyAction::CheckCsp {
                    allow_trusted_types_eval: true,
                    modified_source: None,
                }
            } else {
                match trusted_types_code_generation_check(scope, source, is_code_like) {
                    TrustedTypesCodeGenerationCheck::AllowModified(source) => {
                        StringCodeGenerationPolicyAction::CheckCsp {
                            allow_trusted_types_eval: true,
                            modified_source: Some(source),
                        }
                    }
                    TrustedTypesCodeGenerationCheck::AllowOriginal => {
                        StringCodeGenerationPolicyAction::AllowWithoutCsp
                    }
                    TrustedTypesCodeGenerationCheck::Block => {
                        StringCodeGenerationPolicyAction::Block
                    }
                }
            }
        } else {
            match trusted_types_code_generation_check(scope, source, is_code_like) {
                TrustedTypesCodeGenerationCheck::AllowOriginal => {
                    StringCodeGenerationPolicyAction::AllowWithoutCsp
                }
                TrustedTypesCodeGenerationCheck::AllowModified(source) => {
                    StringCodeGenerationPolicyAction::CheckCsp {
                        allow_trusted_types_eval: false,
                        modified_source: Some(source),
                    }
                }
                TrustedTypesCodeGenerationCheck::Block => StringCodeGenerationPolicyAction::Block,
            }
        }
    } else {
        match non_trusted_types_code_generation_source(source, is_code_like) {
            NonTrustedTypesCodeGenerationSource::String => {
                StringCodeGenerationPolicyAction::CheckCsp {
                    allow_trusted_types_eval: false,
                    modified_source: None,
                }
            }
            NonTrustedTypesCodeGenerationSource::CodeLikeObject
            | NonTrustedTypesCodeGenerationSource::PassThroughObject => {
                match trusted_types_code_generation_check(scope, source, is_code_like) {
                    TrustedTypesCodeGenerationCheck::AllowModified(source) => {
                        StringCodeGenerationPolicyAction::CheckCsp {
                            allow_trusted_types_eval: false,
                            modified_source: Some(source),
                        }
                    }
                    TrustedTypesCodeGenerationCheck::AllowOriginal => {
                        StringCodeGenerationPolicyAction::AllowWithoutCsp
                    }
                    TrustedTypesCodeGenerationCheck::Block => {
                        StringCodeGenerationPolicyAction::Block
                    }
                }
            }
        }
    };
    match action {
        StringCodeGenerationPolicyAction::AllowWithoutCsp => true,
        StringCodeGenerationPolicyAction::CheckCsp {
            allow_trusted_types_eval,
            modified_source: replacement,
        } => {
            if !unsafe { &mut *host_ptr }
                .allows_eval_code_generation_by_csp(scope, allow_trusted_types_eval)
            {
                return false;
            }
            if let Some(replacement) = replacement {
                let Some(replacement) = v8::String::new(scope, &replacement) else {
                    return false;
                };
                if !modified_source.is_null() {
                    unsafe {
                        *modified_source = &*replacement;
                    }
                }
            }
            true
        }
        StringCodeGenerationPolicyAction::Block => false,
    }
}

enum StringCodeGenerationPolicyAction {
    AllowWithoutCsp,
    CheckCsp {
        allow_trusted_types_eval: bool,
        modified_source: Option<String>,
    },
    Block,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NonTrustedTypesCodeGenerationSource {
    String,
    CodeLikeObject,
    PassThroughObject,
}

pub(super) fn non_trusted_types_code_generation_source(
    source: v8::Local<'_, v8::Value>,
    is_code_like: bool,
) -> NonTrustedTypesCodeGenerationSource {
    if source.is_string() {
        NonTrustedTypesCodeGenerationSource::String
    } else if is_code_like {
        NonTrustedTypesCodeGenerationSource::CodeLikeObject
    } else {
        NonTrustedTypesCodeGenerationSource::PassThroughObject
    }
}
