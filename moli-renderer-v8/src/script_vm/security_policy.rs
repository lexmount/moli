use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use super::ScriptVm;
use crate::{
    context_bootstrap::{TrustedTypesCodeGenerationCheck, trusted_types_code_generation_check},
    document_runtime::DocumentContentSecurityPolicyViolation,
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
    pub(crate) fn module_fetch_csp_context_for_child_owner(
        &self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm: crate::frame_owner_model::FrameRealmId,
        request_url: &url::Url,
        kind: crate::module_runtime::ModuleKind,
        metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> Option<crate::content_security_policy::ModuleFetchCspContext> {
        let host = self._context_host.borrow();
        let task_owner = host.current_child_module_route_task_owner(owner, realm)?;
        let target = host.current_child_module_fetch_target_for_realm(task_owner, realm)?;
        host.module_fetch_csp_context_for_owner(
            crate::native_bridge::OwnerDispatchScope::Child(target.child_handle()),
            request_url,
            kind,
            metadata,
        )
    }

    pub(in crate::script_vm) fn dispatch_captured_module_csp_violation(
        &mut self,
        owner: crate::native_bridge::OwnerDispatchScope,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        let result = self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .dispatch_content_security_policy_violation_event_for_owner_best_effort(
                    scope, host_ptr, owner, violation,
                );
            Ok(())
        });
        if let Err(error) = result {
            self.record_runtime_warning(format_args!("module CSP report dispatch failed: {error}"));
        }
    }

    pub(in crate::script_vm) fn queue_captured_module_csp_violation(
        &mut self,
        owner: crate::native_bridge::OwnerDispatchScope,
        violation: DocumentContentSecurityPolicyViolation,
    ) {
        let target = {
            let host = self._context_host.borrow();
            host.current_main_document_task_owner()
                .zip(host.current_window_document_task_target_for_dispatch_scope(owner))
        };
        if let Some((root_owner, target)) = target {
            self.document_runtime
                .enqueue_content_security_policy_violation_event_task(
                    ContentSecurityPolicyViolationEventTask::for_window(
                        root_owner, target, violation,
                    ),
                );
        }
    }
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
        self.with_main_world_document_scope(|scope, host_ptr, document_runtime| {
            document_runtime
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
        if let Some(target) = task.window_target() {
            if !self
                ._context_host
                .borrow()
                .window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
            {
                return ContentSecurityPolicyViolationBodyExecution::DiscardedStaleDocument;
            }
            let result = self.with_default_context_scope(|scope, host_ptr| {
                unsafe { &mut *host_ptr }
                    .dispatch_content_security_policy_violation_event_for_owner_best_effort(
                        scope,
                        host_ptr,
                        target.dispatch_scope(),
                        task.violation(),
                    );
                Ok(())
            });
            return ContentSecurityPolicyViolationBodyExecution::DispatchAttempted(result);
        }
        let result = self.with_main_world_document_scope(|scope, host_ptr, document_runtime| {
            document_runtime.dispatch_content_security_policy_violation_event_page_task(
                scope, host_ptr, task,
            )?;
            Ok(())
        });
        ContentSecurityPolicyViolationBodyExecution::DispatchAttempted(result)
    }

    #[cfg(test)]
    pub(crate) fn set_response_content_security_policies(&mut self, policies: &[String]) {
        self.document_runtime
            .set_response_content_security_policies(policies);
    }

    pub(crate) fn set_bypass_content_security_policy(&mut self, bypass: bool) {
        self.document_runtime
            .set_bypass_content_security_policy(bypass);
    }

    #[cfg(test)]
    pub(crate) fn set_response_content_security_report_only_policies(
        &mut self,
        policies: &[String],
    ) {
        self.document_runtime
            .set_response_content_security_report_only_policies(policies);
    }

    #[cfg(test)]
    pub(crate) fn set_response_referrer_policy(&mut self, policy: Option<String>) {
        self.document_runtime.set_response_referrer_policy(policy);
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

pub(crate) fn string_code_generation_check_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Value>,
    is_code_like: bool,
) -> v8::ModifyCodeGenerationFromStringsResult<'s> {
    let original_source = source;
    let worker_trusted_types_requirements =
        crate::worker::worker_trusted_types_for_script_requirements(scope);
    let (trusted_types_requirements, allow_trusted_types_eval, host_ptr) =
        if let Some(requirements) = worker_trusted_types_requirements {
            (
                requirements,
                crate::worker::worker_allows_trusted_types_eval(scope).unwrap_or(false),
                None,
            )
        } else {
            let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                return allowed_code_generation_result(scope, source, None);
            };
            if crate::context_bootstrap::consume_internal_javascript_url_eval(scope) {
                return code_generation_result(true, None);
            }
            (
                unsafe { &*host_ptr }.trusted_types_for_script_requirements(scope),
                unsafe { &*host_ptr }.allows_trusted_types_eval(scope),
                Some(host_ptr),
            )
        };
    let action = if trusted_types_requirements.requires_conversion() {
        if allow_trusted_types_eval {
            // The keyword relaxes Trusted Types conversion, but it does not
            // override another CSP policy. The per-policy CSP gate still runs.
            if source.is_string() {
                StringCodeGenerationPolicyAction::CheckCsp {
                    allow_trusted_types_eval: true,
                    source: string_code_generation_source(scope, source),
                    modified_source: None,
                }
            } else {
                match trusted_types_code_generation_check(
                    scope,
                    source,
                    is_code_like,
                    trusted_types_requirements,
                ) {
                    TrustedTypesCodeGenerationCheck::AllowModified(source) => {
                        StringCodeGenerationPolicyAction::CheckCsp {
                            allow_trusted_types_eval: true,
                            source: Some(source.clone()),
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
            let check = trusted_types_code_generation_check(
                scope,
                source,
                is_code_like,
                trusted_types_requirements,
            );
            code_generation_action_from_trusted_types_check(check)
        }
    } else if is_code_like || !source.is_string() {
        let check = trusted_types_code_generation_check(
            scope,
            source,
            is_code_like,
            trusted_types_requirements,
        );
        code_generation_action_from_trusted_types_check(check)
    } else {
        match non_trusted_types_code_generation_source(source, is_code_like) {
            NonTrustedTypesCodeGenerationSource::String => {
                StringCodeGenerationPolicyAction::CheckCsp {
                    allow_trusted_types_eval: false,
                    source: string_code_generation_source(scope, source),
                    modified_source: None,
                }
            }
            NonTrustedTypesCodeGenerationSource::CodeLikeObject
            | NonTrustedTypesCodeGenerationSource::PassThroughObject => {
                match trusted_types_code_generation_check(
                    scope,
                    source,
                    is_code_like,
                    trusted_types_requirements,
                ) {
                    TrustedTypesCodeGenerationCheck::AllowModified(source) => {
                        StringCodeGenerationPolicyAction::CheckCsp {
                            allow_trusted_types_eval: false,
                            source: Some(source.clone()),
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
    let codegen_allowed = match action {
        StringCodeGenerationPolicyAction::AllowWithoutCsp => true,
        StringCodeGenerationPolicyAction::CheckCsp {
            allow_trusted_types_eval,
            source,
            modified_source: replacement,
        } => {
            let allowed = match host_ptr {
                Some(host_ptr) => unsafe { &mut *host_ptr }.allows_eval_code_generation_by_csp(
                    scope,
                    allow_trusted_types_eval,
                    source.as_deref(),
                ),
                None => crate::worker::worker_allows_eval_code_generation_by_csp(
                    scope,
                    allow_trusted_types_eval,
                    source.as_deref(),
                )
                .unwrap_or(false),
            };
            if !allowed {
                return code_generation_result(false, None);
            }
            let replacement = match replacement {
                Some(replacement) => {
                    let Some(replacement) = v8::String::new(scope, &replacement) else {
                        return code_generation_result(false, None);
                    };
                    Some(replacement)
                }
                None => None,
            };
            return allowed_code_generation_result(scope, original_source, replacement);
        }
        StringCodeGenerationPolicyAction::Block => false,
    };
    if codegen_allowed {
        allowed_code_generation_result(scope, original_source, None)
    } else {
        code_generation_result(false, None)
    }
}

fn allowed_code_generation_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original_source: v8::Local<'s, v8::Value>,
    modified_source: Option<v8::Local<'s, v8::String>>,
) -> v8::ModifyCodeGenerationFromStringsResult<'s> {
    let original_string = v8::Local::<v8::String>::try_from(original_source).ok();
    let Some(source) = modified_source.or(original_string) else {
        return code_generation_result(true, modified_source);
    };
    let is_dynamic_function = original_string
        .is_some_and(|source| is_dynamic_function_source(scope, source))
        || is_dynamic_function_source(scope, source);
    if !is_dynamic_function {
        return code_generation_result(true, modified_source);
    }

    // V8's Function-constructor cache keys include the native-context closure
    // but omit the active Script/Module. A cached function can consequently
    // keep another script's dynamic-import base URL and fetch options.
    let script_id = v8::StackTrace::current_stack_trace(scope, 1)
        // V8's GetFrame does not bounds-check an empty trace, including when
        // a host call or cross-origin boundary hides all JavaScript frames.
        .filter(|trace| trace.get_frame_count() != 0)
        .and_then(|trace| trace.get_frame(scope, 0))
        .filter(|frame| frame.is_user_javascript())
        .map(|frame| frame.get_script_id())
        .filter(|id| *id != 0);
    let origin = if let Some(script_id) = script_id {
        format!("script-{script_id}")
    } else {
        // A host call or a security boundary can leave no visible JS frame.
        // Give such compilations distinct keys rather than sharing provenance.
        static NEXT_UNATTRIBUTED_COMPILATION: AtomicU64 = AtomicU64::new(1);
        let Ok(id) = NEXT_UNATTRIBUTED_COMPILATION.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |id| id.checked_add(1),
        ) else {
            return code_generation_result(false, None);
        };
        format!("host-{id}")
    };
    // Keep the key outside the FunctionExpression: neither Function#toString
    // nor parameter/body source positions change. Concat preserves UTF-16,
    // including lone surrogates, without a lossy Rust string conversion.
    // A line comment cannot close an invalid, unterminated block comment in
    // a Trusted Types policy's replacement source.
    let Some(suffix) = v8::String::new(scope, &format!("\n// moli-function-origin:{origin}"))
    else {
        return code_generation_result(false, None);
    };
    let Some(source) = v8::String::concat(scope, source, suffix) else {
        return code_generation_result(false, None);
    };
    code_generation_result(true, Some(source))
}

fn is_dynamic_function_source(
    scope: &v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::String>,
) -> bool {
    if source.length() < 3 {
        return false;
    }
    let mut prefix = [0_u16; 32];
    source.write_v2(scope, 0, &mut prefix, v8::WriteFlags::empty());
    let matches_prefix = [
        "(function anonymous(",
        "(function* anonymous(",
        "(async function anonymous(",
        "(async function* anonymous(",
    ]
    .into_iter()
    .any(|candidate| {
        prefix[..candidate.len()]
            .iter()
            .copied()
            .eq(candidate.encode_utf16())
    });
    if !matches_prefix {
        return false;
    }
    let mut suffix = [0_u16; 3];
    source.write_v2(
        scope,
        (source.length() - suffix.len()) as u32,
        &mut suffix,
        v8::WriteFlags::empty(),
    );
    suffix == [u16::from(b'\n'), u16::from(b'}'), u16::from(b')')]
}

fn code_generation_result<'s>(
    codegen_allowed: bool,
    modified_source: Option<v8::Local<'s, v8::String>>,
) -> v8::ModifyCodeGenerationFromStringsResult<'s> {
    v8::ModifyCodeGenerationFromStringsResult {
        codegen_allowed,
        modified_source,
    }
}

enum StringCodeGenerationPolicyAction {
    AllowWithoutCsp,
    CheckCsp {
        allow_trusted_types_eval: bool,
        source: Option<String>,
        modified_source: Option<String>,
    },
    Block,
}

fn code_generation_action_from_trusted_types_check(
    check: TrustedTypesCodeGenerationCheck,
) -> StringCodeGenerationPolicyAction {
    match check {
        TrustedTypesCodeGenerationCheck::AllowOriginal => {
            StringCodeGenerationPolicyAction::AllowWithoutCsp
        }
        TrustedTypesCodeGenerationCheck::AllowModified(source) => {
            StringCodeGenerationPolicyAction::CheckCsp {
                allow_trusted_types_eval: false,
                source: Some(source.clone()),
                modified_source: Some(source),
            }
        }
        TrustedTypesCodeGenerationCheck::Block => StringCodeGenerationPolicyAction::Block,
    }
}

fn string_code_generation_source(
    scope: &mut v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::Value>,
) -> Option<String> {
    v8::Local::<v8::String>::try_from(source)
        .ok()
        .map(|source| source.to_rust_string_lossy(scope))
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
