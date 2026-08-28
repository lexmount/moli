use anyhow::{Result, anyhow};

use super::{
    ScriptVm,
    eval_exec::{SourceTextScriptCompletion, SourceTextScriptCompletionMode},
};
#[cfg(test)]
use crate::util::{v8_string, v8str};
use crate::{
    frame_owner_model::{FrameRealmId, FrameScriptJob, FrameScriptSource},
    native_bridge::PendingWindowMessageEndpoint,
};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum FrameScriptCompletionValue {
    String(String),
    NonString,
}

enum FrameScriptJobSourceTextCompletionMode {
    Ignore,
    ValueTypeAware,
}

enum FrameScriptJobExecutionBoundary {
    CompleteRuntimeTurn,
    SelectedPageTaskBody,
}

impl ScriptVm {
    fn current_realm_id_for_frame_script_job(&self, job: &FrameScriptJob) -> Result<FrameRealmId> {
        self._context_host
            .borrow()
            .frame_owner_current_realm_id_for_script_job(job)
            .ok_or_else(|| {
                anyhow!(
                    "frame script job for {:?}/{:?} is stale or has no materialized current FrameRealmRecord",
                    job.local_window_id,
                    job.document_id
                )
            })
    }

    fn frame_script_job_window_message_source(
        &self,
        job: &FrameScriptJob,
    ) -> Option<PendingWindowMessageEndpoint> {
        self._context_host
            .borrow()
            .frame_owner_child_handle_for_script_job(job)
            .map(PendingWindowMessageEndpoint::ChildWindow)
    }

    #[cfg(test)]
    pub(crate) fn eval_in_child_default_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<String> {
        let realm_id =
            self.child_frame_owner_realm_id_for_execution_context_id(execution_context_id)?;
        self.eval_in_frame_realm(realm_id, source)
    }

    #[cfg(test)]
    pub(super) fn eval_in_frame_realm(
        &mut self,
        realm_id: FrameRealmId,
        source: &str,
    ) -> Result<String> {
        let context_ptr = self.frame_realm_context_ptr(realm_id)?;
        self.eval_string_in_context_ptr_runtime_turn(context_ptr, source, true)
    }

    pub(super) fn execute_frame_script_job_value_type_completion_selected_task_body(
        &mut self,
        job: FrameScriptJob,
    ) -> Result<FrameScriptCompletionValue> {
        self.execute_source_text_frame_script_job(
            job,
            FrameScriptJobSourceTextCompletionMode::ValueTypeAware,
            FrameScriptJobExecutionBoundary::SelectedPageTaskBody,
        )
        .and_then(|completion| match completion {
            SourceTextScriptCompletion::String(completion) => {
                Ok(FrameScriptCompletionValue::String(completion))
            }
            SourceTextScriptCompletion::NonString => Ok(FrameScriptCompletionValue::NonString),
            SourceTextScriptCompletion::Ignored => Err(anyhow!(
                "source-text frame script job unexpectedly ignored completion"
            )),
        })
    }

    pub(super) fn execute_frame_script_job_selected_task_body(
        &mut self,
        job: FrameScriptJob,
    ) -> Result<()> {
        self.execute_source_text_frame_script_job(
            job,
            FrameScriptJobSourceTextCompletionMode::Ignore,
            FrameScriptJobExecutionBoundary::SelectedPageTaskBody,
        )
        .map(|_completion| ())
    }

    pub(super) fn exec_frame_script_job(&mut self, job: FrameScriptJob) -> Result<()> {
        self.execute_source_text_frame_script_job(
            job,
            FrameScriptJobSourceTextCompletionMode::Ignore,
            FrameScriptJobExecutionBoundary::CompleteRuntimeTurn,
        )
        .map(|_completion| ())
    }

    fn execute_source_text_frame_script_job(
        &mut self,
        job: FrameScriptJob,
        completion_mode: FrameScriptJobSourceTextCompletionMode,
        execution_boundary: FrameScriptJobExecutionBoundary,
    ) -> Result<SourceTextScriptCompletion> {
        let Some(job) = self.prepare_frame_script_job_for_execution(job)? else {
            return Ok(SourceTextScriptCompletion::Ignored);
        };
        let realm_id = self.current_realm_id_for_frame_script_job(&job)?;
        let is_source_text = matches!(&job.source, FrameScriptSource::SourceText(_));
        let context_ptr = is_source_text
            .then(|| self.frame_realm_context_ptr(realm_id))
            .transpose()?;
        let current_script_token = if is_source_text {
            self._context_host
                .borrow_mut()
                .push_frame_script_job_current_script(&job)
        } else {
            None
        };
        let message_source = self.frame_script_job_window_message_source(&job);
        let previous_message_source = message_source.map(|source| {
            self._context_host
                .borrow_mut()
                .enter_window_message_source_scope(source)
        });
        let FrameScriptJob {
            source,
            script_url,
            base_url,
            script_nonce,
            ..
        } = job;
        let result = match source {
            FrameScriptSource::SourceText(source) => {
                let context_ptr =
                    context_ptr.expect("source-text frame script job has context ptr");
                let source_completion_mode = match completion_mode {
                    FrameScriptJobSourceTextCompletionMode::Ignore => {
                        SourceTextScriptCompletionMode::Ignore
                    }
                    FrameScriptJobSourceTextCompletionMode::ValueTypeAware => {
                        SourceTextScriptCompletionMode::ValueTypeAware
                    }
                };
                let result = match execution_boundary {
                    FrameScriptJobExecutionBoundary::CompleteRuntimeTurn => self
                        .execute_source_text_in_context_ptr_runtime_turn_with_base_url_and_current_window_error_report(
                            context_ptr,
                            &source,
                            Some(&script_url),
                            Some(&base_url),
                            0,
                            script_nonce.as_deref(),
                            true,
                            true,
                            source_completion_mode,
                        ),
                    FrameScriptJobExecutionBoundary::SelectedPageTaskBody => self
                        .execute_source_text_in_context_ptr_selected_page_task_body(
                            context_ptr,
                            &source,
                            Some(&script_url),
                            Some(&base_url),
                            0,
                            script_nonce.as_deref(),
                            source_completion_mode,
                        ),
                };
                if let Some(token) = current_script_token {
                    self._context_host
                        .borrow_mut()
                        .pop_child_current_script(token);
                }
                result
            }
            #[cfg(test)]
            FrameScriptSource::FunctionConstructor(_) => Err(anyhow!(
                "function-constructor frame script job is not a source-text execution"
            )),
        };
        if let Some(previous_message_source) = previous_message_source {
            self._context_host
                .borrow_mut()
                .restore_window_message_source_scope(previous_message_source);
        }
        result
    }

    fn prepare_frame_script_job_for_execution(
        &mut self,
        mut job: FrameScriptJob,
    ) -> Result<Option<FrameScriptJob>> {
        if job.kind == crate::frame_owner_model::FrameScriptJobKind::JavascriptUrl {
            let realm_id = self.current_realm_id_for_frame_script_job(&job)?;
            let url = job.script_url.clone();
            let allowed = self.with_frame_realm_scope(realm_id, |scope, host_ptr| {
                let host = unsafe { &mut *host_ptr };
                let Some(owner) = host.current_realm_owner_dispatch_scope(scope) else {
                    return Ok(false);
                };
                let csp_source = crate::native_bridge::javascript_url_csp_source(&url);
                if !host.allows_inline_javascript_navigation_by_csp(scope, owner, &csp_source) {
                    return Ok(false);
                }
                #[cfg(test)]
                let source = match &mut job.source {
                    FrameScriptSource::SourceText(source) => source,
                    FrameScriptSource::FunctionConstructor(_) => return Ok(false),
                };
                #[cfg(not(test))]
                let FrameScriptSource::SourceText(source) = &mut job.source;
                let requirements = host.trusted_types_for_script_requirements(scope);
                let Some(rewritten) =
                    crate::context_bootstrap::trusted_script_string_for_javascript_navigation(
                        scope,
                        source,
                        requirements,
                    )
                else {
                    return Ok(false);
                };
                *source = rewritten;
                Ok(true)
            })?;
            return Ok(allowed.then_some(job));
        }
        if !job.needs_inline_classic_element_preparation() {
            // Non-inline jobs do not use script-element preparation. Synthetic
            // inline jobs without a backing element are not DOM insertions;
            // production parser/dynamic jobs retain the element handle and
            // therefore always take the owner-scoped gate.
            return Ok(Some(job));
        }
        let realm_id = self.current_realm_id_for_frame_script_job(&job)?;
        let allowed = self.with_frame_realm_scope(realm_id, |scope, host_ptr| {
            crate::native_bridge::element::prepare_inline_classic_frame_script_job_for_execution(
                scope, host_ptr, &mut job,
            )
        })?;
        Ok(allowed.then_some(job))
    }

    // This is the ScriptVm-owned half of `contentWindow.Function` object
    // construction. The native WindowProxy binding callback must not call back into
    // ScriptVm directly; the bridge that hands this result back synchronously is
    // still being built, and the test target locks this owner boundary meanwhile.
    #[cfg(test)]
    pub(super) fn function_from_frame_script_job(
        &mut self,
        job: FrameScriptJob,
    ) -> Result<v8::Global<v8::Function>> {
        let realm_id = self.current_realm_id_for_frame_script_job(&job)?;
        match job.source {
            FrameScriptSource::FunctionConstructor(source) => {
                self.construct_function_in_frame_realm(realm_id, &source)
            }
            FrameScriptSource::SourceText(_) => Err(anyhow!(
                "frame script job kind {:?} is not a function constructor evaluation",
                job.kind
            )),
        }
    }

    #[cfg(test)]
    fn construct_function_in_frame_realm(
        &mut self,
        realm_id: FrameRealmId,
        source: &crate::frame_owner_model::FrameFunctionConstructorSource,
    ) -> Result<v8::Global<v8::Function>> {
        self.with_frame_realm_scope(realm_id, |scope, _host_ptr| {
            let global = scope.get_current_context().global(scope);
            let function_constructor = global
                .get(scope, v8str(scope, "Function").into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .ok_or_else(|| anyhow!("FrameRealm has no Function constructor"))?;
            let mut args = Vec::with_capacity(source.parameters.len() + 1);
            for parameter in &source.parameters {
                let parameter = v8_string(scope, parameter)
                    .ok_or_else(|| anyhow!("failed to allocate Function parameter string"))?;
                args.push(parameter.into());
            }
            let body = v8_string(scope, &source.body)
                .ok_or_else(|| anyhow!("failed to allocate Function body string"))?;
            args.push(body.into());
            let function = function_constructor
                .new_instance(scope, &args)
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .ok_or_else(|| anyhow!("Function constructor did not return a function object"))?;
            Ok(v8::Global::new(scope, function))
        })
    }

    #[cfg(test)]
    pub(super) fn call_frame_function_for_test(
        &mut self,
        realm_id: FrameRealmId,
        function: &v8::Global<v8::Function>,
    ) -> Result<String> {
        self.with_frame_realm_scope_and_checkpoint_for_test(realm_id, |scope, _host_ptr| {
            let function = v8::Local::new(scope, function);
            let receiver = v8::undefined(scope).into();
            let result = function
                .call(scope, receiver, &[])
                .ok_or_else(|| anyhow!("frame function call failed"))?;
            Ok(result
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default())
        })
    }
}
