use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::pin;
use std::rc::Rc;

use crate::exception_reporting::{V8ExceptionReport, build_event_handler_exception_report};
use crate::module_runtime::{
    ModuleAttributesKey, ModuleImportPhase, WasmDependencyModuleMessages, WasmImportRecord,
    WasmModuleRecord, ensure_wasm_dependency_module_namespace_ready,
    evaluate_wasm_synthetic_module, preserve_current_v8_module_exception, throw_wasm_link_error,
    throw_wasm_synthetic_module_error, wasm_dependency_export_value,
};
use crate::util::{get_private_value, v8_string, v8str};
use crate::wasm_module_support::{
    prepare_wasm_module_record, v8_exception_message_or, wasm_evaluation_import_modules,
};
use moli_fetch::{BrowserRequestMetadata, RequestCredentialsMode};
use moli_webapi_declare::WebApiObject;
use tokio::sync::mpsc;
use url::Url;

use super::global_scope::worker_current_script_url;
use super::handle::{WorkerParentErrorEventKind, WorkerScriptResource};
use crate::content_security_policy::ContentSecurityPolicyUrlViolation;

pub(super) type WorkerBootstrapError = (
    V8ExceptionReport,
    Option<v8::Global<v8::Value>>,
    WorkerParentErrorEventKind,
);

pub(super) type WorkerModuleGraphFetchId = u64;
pub(super) type WorkerModuleEvaluationId = u64;

const WORKER_MODULE_EVALUATION_REACTION_ID_SLOT: &str = "workerModuleEvaluationReactionId";

#[derive(WebApiObject)]
#[webapi(record)]
struct WorkerModuleEvaluationReactionDataDeclaration {
    #[webapi(slot = WORKER_MODULE_EVALUATION_REACTION_ID_SLOT)]
    evaluation_id: f64,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct WorkerImportMetaDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    url: v8::Local<'scope, v8::String>,

    #[webapi(
        method,
        enumerable,
        callback = worker_import_meta_resolve_callback,
        data = self.url,
        length = 1
    )]
    resolve: (),
}

pub(super) struct WorkerModuleGraphFetchCompletion {
    fetch_id: WorkerModuleGraphFetchId,
    result: Result<WorkerModuleFetchedSource, String>,
    csp_violation: Option<ContentSecurityPolicyUrlViolation>,
    csp_report_only_violation: Option<ContentSecurityPolicyUrlViolation>,
}

pub(super) struct WorkerModuleFetchedSource {
    final_url: Url,
    source: WorkerModuleSource,
    resource: Option<WorkerScriptResource>,
    response_referrer_policy: Option<String>,
}

#[derive(Clone)]
pub(super) enum WorkerModuleSource {
    Text(String),
    Binary(Vec<u8>),
}

pub(super) struct WorkerModuleEvaluationCompletion {
    evaluation_id: WorkerModuleEvaluationId,
    result: Result<(), String>,
}

pub(super) enum WorkerDynamicModuleImportAdvance {
    Complete,
    NeedFetches(WorkerModuleGraphFetchBatch),
    WaitingFetches,
    WaitingEvaluation {
        evaluation_id: WorkerModuleEvaluationId,
    },
}

#[derive(Clone, Copy)]
enum WorkerDynamicModuleImportErrorKind {
    Type,
    Syntax,
}

struct WorkerDynamicModuleImportError {
    message: String,
    kind: WorkerDynamicModuleImportErrorKind,
    rejection: Option<v8::Global<v8::Value>>,
    stage: WorkerDynamicModuleImportErrorStage,
}

type WorkerDynamicModuleImportResult<T> = Result<T, WorkerDynamicModuleImportError>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkerDynamicModuleImportErrorStage {
    Graph,
    Instantiate,
    Evaluate,
}

impl WorkerDynamicModuleImportError {
    fn type_error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: WorkerDynamicModuleImportErrorKind::Type,
            rejection: None,
            stage: WorkerDynamicModuleImportErrorStage::Graph,
        }
    }

    fn syntax_error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: WorkerDynamicModuleImportErrorKind::Syntax,
            rejection: None,
            stage: WorkerDynamicModuleImportErrorStage::Graph,
        }
    }

    fn caught_instantiate_exception(
        scope: &mut v8::PinScope<'_, '_>,
        exception: Option<v8::Local<'_, v8::Value>>,
        fallback: impl Into<String>,
    ) -> Self {
        Self {
            message: fallback.into(),
            kind: WorkerDynamicModuleImportErrorKind::Type,
            rejection: exception.map(|exception| v8::Global::new(scope, exception)),
            stage: WorkerDynamicModuleImportErrorStage::Instantiate,
        }
    }

    fn caught_evaluation_exception(
        scope: &mut v8::PinScope<'_, '_>,
        exception: Option<v8::Local<'_, v8::Value>>,
        fallback: impl Into<String>,
    ) -> Self {
        Self {
            message: fallback.into(),
            kind: WorkerDynamicModuleImportErrorKind::Type,
            rejection: exception.map(|exception| v8::Global::new(scope, exception)),
            stage: WorkerDynamicModuleImportErrorStage::Evaluate,
        }
    }

    fn evaluation_rejection_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<'_, v8::Value>,
        fallback: impl Into<String>,
    ) -> Self {
        Self {
            message: fallback.into(),
            kind: WorkerDynamicModuleImportErrorKind::Type,
            rejection: Some(v8::Global::new(scope, value)),
            stage: WorkerDynamicModuleImportErrorStage::Evaluate,
        }
    }

    fn clone_for_scope(&self, scope: &mut v8::PinScope<'_, '_>) -> Self {
        Self {
            message: self.message.clone(),
            kind: self.kind,
            rejection: self
                .rejection
                .as_ref()
                .map(|value| v8::Global::new(scope, v8::Local::new(scope, value))),
            stage: self.stage,
        }
    }
}

impl From<String> for WorkerDynamicModuleImportError {
    fn from(message: String) -> Self {
        Self::type_error(message)
    }
}

impl WorkerModuleGraphFetchCompletion {
    pub(super) fn new(
        fetch_id: WorkerModuleGraphFetchId,
        result: Result<WorkerModuleFetchedSource, String>,
    ) -> Self {
        Self {
            fetch_id,
            result,
            csp_violation: None,
            csp_report_only_violation: None,
        }
    }

    pub(super) fn with_csp_violation(
        mut self,
        violation: ContentSecurityPolicyUrlViolation,
    ) -> Self {
        self.csp_violation = Some(violation);
        self
    }

    pub(super) fn with_csp_report_only_violation(
        mut self,
        violation: ContentSecurityPolicyUrlViolation,
    ) -> Self {
        self.csp_report_only_violation = Some(violation);
        self
    }

    pub(super) fn fetch_id(&self) -> WorkerModuleGraphFetchId {
        self.fetch_id
    }

    pub(super) fn csp_violation(&self) -> Option<&ContentSecurityPolicyUrlViolation> {
        self.csp_violation.as_ref()
    }

    pub(super) fn csp_report_only_violation(&self) -> Option<&ContentSecurityPolicyUrlViolation> {
        self.csp_report_only_violation.as_ref()
    }

    pub(super) fn result(&self) -> Result<&WorkerModuleFetchedSource, &str> {
        self.result.as_ref().map_err(String::as_str)
    }
}

impl WorkerModuleFetchedSource {
    pub(super) fn new(final_url: Url, source: WorkerModuleSource) -> Self {
        Self {
            final_url,
            source,
            resource: None,
            response_referrer_policy: None,
        }
    }

    pub(super) fn with_resource(mut self, resource: WorkerScriptResource) -> Self {
        self.resource = Some(resource);
        self
    }

    pub(super) fn with_response_referrer_policy(
        mut self,
        response_referrer_policy: Option<String>,
    ) -> Self {
        self.response_referrer_policy = response_referrer_policy;
        self
    }

    pub(super) fn final_url(&self) -> &Url {
        &self.final_url
    }

    pub(super) fn source(&self) -> &WorkerModuleSource {
        &self.source
    }

    pub(super) fn resource(&self) -> Option<&WorkerScriptResource> {
        self.resource.as_ref()
    }

    fn effective_referrer_policy(&self, inherited_referrer_policy: Option<&str>) -> Option<String> {
        self.response_referrer_policy
            .clone()
            .or_else(|| inherited_referrer_policy.map(str::to_owned))
    }
}

impl WorkerModuleSource {
    pub(super) fn text(source: String) -> Self {
        Self::Text(source)
    }

    pub(super) fn binary(bytes: Vec<u8>) -> Self {
        Self::Binary(bytes)
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Text(source) => source.len(),
            Self::Binary(bytes) => bytes.len(),
        }
    }

    fn text_source(&self) -> Option<&str> {
        match self {
            Self::Text(source) => Some(source),
            Self::Binary(_) => None,
        }
    }

    fn binary_source(&self) -> Option<&[u8]> {
        match self {
            Self::Text(_) => None,
            Self::Binary(bytes) => Some(bytes),
        }
    }
}

impl WorkerModuleEvaluationCompletion {
    fn new(evaluation_id: WorkerModuleEvaluationId, result: Result<(), String>) -> Self {
        Self {
            evaluation_id,
            result,
        }
    }

    pub(super) fn evaluation_id(&self) -> WorkerModuleEvaluationId {
        self.evaluation_id
    }

    pub(super) fn result(&self) -> Result<(), &str> {
        self.result.as_ref().map(|_| ()).map_err(String::as_str)
    }
}

pub(super) fn evaluate_module_worker_bootstrap_source(
    scope: &mut v8::PinScope<'_, '_>,
    source: WorkerModuleSource,
    script_url: &str,
    static_import_initiator_url: Option<Url>,
    credentials_mode: RequestCredentialsMode,
    referrer_policy: Option<String>,
    evaluation_completion_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
) -> WorkerModuleBootstrapStart {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let root_url = match Url::parse(script_url) {
        Ok(url) => url,
        Err(_) => {
            return WorkerModuleBootstrapStart::Failed(Box::new(worker_bootstrap_error(
                &mut scope,
                script_url,
                "Module worker script URL is invalid",
                WorkerParentErrorEventKind::Event,
            )));
        }
    };
    let static_import_initiator_url =
        static_import_initiator_url.unwrap_or_else(|| root_url.clone());
    let runtime =
        WorkerModuleRuntime::new(credentials_mode, referrer_policy, evaluation_completion_tx);
    runtime.install_context_slot(scope.get_current_context());
    let mut bootstrap = runtime.start_bootstrap(source, root_url, static_import_initiator_url);
    match bootstrap.advance(&mut scope) {
        WorkerModuleAdvance::ReadyToInstantiate { root_entry } => {
            match finish_worker_module_bootstrap(
                &mut scope,
                &bootstrap.runtime,
                root_entry,
                script_url,
            ) {
                Ok(WorkerModuleFinish::Complete) => WorkerModuleBootstrapStart::Complete,
                Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id }) => {
                    WorkerModuleBootstrapStart::Pending(Box::new(
                        WorkerModulePendingBootstrap::new_evaluation(bootstrap, evaluation_id),
                    ))
                }
                Err(error) => WorkerModuleBootstrapStart::Failed(error),
            }
        }
        WorkerModuleAdvance::NeedFetches(requests) => {
            WorkerModuleBootstrapStart::Pending(Box::new(
                WorkerModulePendingBootstrap::new_fetches(bootstrap, requests),
            ))
        }
        WorkerModuleAdvance::Failed(error) => WorkerModuleBootstrapStart::Failed(error),
    }
}

pub(super) enum WorkerModuleBootstrapStart {
    Complete,
    Pending(Box<WorkerModulePendingBootstrap>),
    Failed(Box<WorkerBootstrapError>),
}

pub(super) enum WorkerModuleBootstrapResume {
    Complete,
    NeedFetches(WorkerModuleGraphFetchBatch),
    WaitingFetches,
    WaitingEvaluation,
    Failed(Box<WorkerBootstrapError>),
}

pub(super) struct WorkerModulePendingBootstrap {
    job: WorkerModuleBootstrapJob,
    state: WorkerModulePendingBootstrapState,
}

enum WorkerModulePendingBootstrapState {
    Fetch(WorkerModuleGraphFetchBatch),
    Evaluation {
        evaluation_id: WorkerModuleEvaluationId,
    },
}

impl WorkerModulePendingBootstrap {
    fn new_fetches(
        job: WorkerModuleBootstrapJob,
        pending_requests: WorkerModuleGraphFetchBatch,
    ) -> Self {
        Self {
            job,
            state: WorkerModulePendingBootstrapState::Fetch(pending_requests),
        }
    }

    fn new_evaluation(
        job: WorkerModuleBootstrapJob,
        evaluation_id: WorkerModuleEvaluationId,
    ) -> Self {
        Self {
            job,
            state: WorkerModulePendingBootstrapState::Evaluation { evaluation_id },
        }
    }

    pub(super) fn pending_requests(&self) -> Option<&WorkerModuleGraphFetchBatch> {
        match &self.state {
            WorkerModulePendingBootstrapState::Fetch(requests) => Some(requests),
            WorkerModulePendingBootstrapState::Evaluation { .. } => None,
        }
    }

    pub(super) fn resume(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: WorkerModuleGraphFetchCompletion,
    ) -> WorkerModuleBootstrapResume {
        let fetch_id = completion.fetch_id();
        let WorkerModulePendingBootstrapState::Fetch(pending_requests) = &mut self.state else {
            return WorkerModuleBootstrapResume::Failed(Box::new(worker_bootstrap_error(
                scope,
                self.job.script_url(),
                "Module worker graph fetch completion arrived while waiting for evaluation",
                WorkerParentErrorEventKind::Event,
            )));
        };
        let Some(pending_request) = pending_requests.remove_by_fetch_id(fetch_id) else {
            return WorkerModuleBootstrapResume::Failed(Box::new(worker_bootstrap_error(
                scope,
                self.job.script_url(),
                &format!(
                    "Module worker graph fetch completion id {fetch_id} did not match any pending id"
                ),
                WorkerParentErrorEventKind::Event,
            )));
        };
        let pending_keys = pending_requests.pending_keys();
        match self.job.resume_fetch_with_pending_keys(
            scope,
            &pending_request,
            completion,
            pending_keys,
        ) {
            WorkerModuleAdvance::ReadyToInstantiate { root_entry } => {
                if !pending_requests.is_empty() {
                    return WorkerModuleBootstrapResume::WaitingFetches;
                }
                match finish_worker_module_bootstrap(
                    scope,
                    &self.job.runtime,
                    root_entry,
                    self.job.script_url(),
                ) {
                    Ok(WorkerModuleFinish::Complete) => WorkerModuleBootstrapResume::Complete,
                    Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id }) => {
                        self.state =
                            WorkerModulePendingBootstrapState::Evaluation { evaluation_id };
                        WorkerModuleBootstrapResume::WaitingEvaluation
                    }
                    Err(error) => WorkerModuleBootstrapResume::Failed(error),
                }
            }
            WorkerModuleAdvance::NeedFetches(requests) => {
                let new_requests = requests.clone();
                pending_requests.extend(requests);
                WorkerModuleBootstrapResume::NeedFetches(new_requests)
            }
            WorkerModuleAdvance::Failed(error) => WorkerModuleBootstrapResume::Failed(error),
        }
    }

    pub(super) fn resume_evaluation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: WorkerModuleEvaluationCompletion,
    ) -> WorkerModuleBootstrapResume {
        let WorkerModulePendingBootstrapState::Evaluation { evaluation_id } = self.state else {
            return WorkerModuleBootstrapResume::Failed(Box::new(worker_bootstrap_error(
                scope,
                self.job.script_url(),
                "Module worker evaluation completion arrived while waiting for graph fetch",
                WorkerParentErrorEventKind::Event,
            )));
        };
        if completion.evaluation_id != evaluation_id {
            return WorkerModuleBootstrapResume::Failed(Box::new(worker_bootstrap_error(
                scope,
                self.job.script_url(),
                &format!(
                    "Module worker evaluation completion id {} did not match pending id {evaluation_id}",
                    completion.evaluation_id
                ),
                WorkerParentErrorEventKind::Event,
            )));
        }
        match completion.result {
            Ok(()) => WorkerModuleBootstrapResume::Complete,
            Err(message) => WorkerModuleBootstrapResume::Failed(Box::new(worker_bootstrap_error(
                scope,
                self.job.script_url(),
                &message,
                WorkerParentErrorEventKind::ErrorEvent,
            ))),
        }
    }
}

enum WorkerModuleFinish {
    Complete,
    PendingEvaluation {
        evaluation_id: WorkerModuleEvaluationId,
    },
}

fn finish_worker_module_bootstrap(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &WorkerModuleRuntime,
    root_entry: usize,
    script_url: &str,
) -> WorkerModuleBootstrapResult<WorkerModuleFinish> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let scope = &mut scope;
    let root_module_global = runtime.graph.borrow().module(root_entry).clone();
    let root_module = v8::Local::new(scope, &root_module_global);

    match root_module.instantiate_module2(
        scope,
        worker_resolve_static_module_callback,
        worker_resolve_static_source_callback,
    ) {
        Some(true) => {}
        Some(false) => {
            return Err(Box::new(worker_bootstrap_error(
                scope,
                script_url,
                "v8 reported module worker instantiate failure",
                WorkerParentErrorEventKind::Event,
            )));
        }
        None => {
            let exception = scope.exception();
            let message = scope.message();
            let stack_trace = scope.stack_trace();
            let report =
                build_event_handler_exception_report(scope, exception, message, stack_trace);
            return Err(Box::new((
                report,
                exception.map(|value| v8::Global::new(scope, value)),
                WorkerParentErrorEventKind::Event,
            )));
        }
    }

    let Some(value) = root_module.evaluate(scope) else {
        let exception = scope.exception();
        let message = scope.message();
        let stack_trace = scope.stack_trace();
        let report = build_event_handler_exception_report(scope, exception, message, stack_trace);
        return Err(Box::new((
            report,
            exception.map(|value| v8::Global::new(scope, value)),
            WorkerParentErrorEventKind::ErrorEvent,
        )));
    };
    scope.perform_microtask_checkpoint();
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
    if root_module.get_status() == v8::ModuleStatus::Errored {
        return Err(Box::new(worker_bootstrap_value_error(
            scope,
            script_url,
            root_module.get_exception(),
            WorkerParentErrorEventKind::ErrorEvent,
        )));
    }
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        match promise.state() {
            v8::PromiseState::Fulfilled => return Ok(WorkerModuleFinish::Complete),
            v8::PromiseState::Rejected => {
                let reason = promise.result(scope);
                return Err(Box::new(worker_bootstrap_value_error(
                    scope,
                    script_url,
                    reason,
                    WorkerParentErrorEventKind::ErrorEvent,
                )));
            }
            v8::PromiseState::Pending => {
                let evaluation_id = runtime.reserve_evaluation_id();
                let promise = v8::Global::new(scope, promise);
                attach_worker_module_evaluation_reactions(scope, evaluation_id, promise)?;
                return Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id });
            }
        }
    }

    Ok(WorkerModuleFinish::Complete)
}

struct WorkerModuleRuntime {
    graph: Rc<RefCell<WorkerModuleGraph>>,
    next_fetch_id: Rc<RefCell<WorkerModuleGraphFetchId>>,
    next_evaluation_id: Rc<RefCell<WorkerModuleEvaluationId>>,
    dynamic_imports: Rc<RefCell<WorkerDynamicModuleResolver>>,
    evaluation_completion_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
}

impl WorkerModuleRuntime {
    fn new(
        credentials_mode: RequestCredentialsMode,
        referrer_policy: Option<String>,
        evaluation_completion_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
    ) -> Self {
        Self {
            graph: Rc::new(RefCell::new(WorkerModuleGraph::new(
                credentials_mode,
                referrer_policy,
            ))),
            next_fetch_id: Rc::new(RefCell::new(0)),
            next_evaluation_id: Rc::new(RefCell::new(0)),
            dynamic_imports: Rc::new(RefCell::new(WorkerDynamicModuleResolver::default())),
            evaluation_completion_tx,
        }
    }

    fn install_context_slot(&self, context: v8::Local<'_, v8::Context>) {
        let _previous_graph = context.set_slot(self.graph.clone());
        let _previous_fetch_id = context.set_slot(Rc::new(WorkerModuleRuntimeFetchIdSlot {
            next_fetch_id: Rc::clone(&self.next_fetch_id),
        }));
        let _previous_evaluation = context.set_slot(Rc::new(WorkerModuleRuntimeEvaluationSlot {
            next_evaluation_id: Rc::clone(&self.next_evaluation_id),
            evaluation_completion_tx: self.evaluation_completion_tx.clone(),
        }));
        let _previous_dynamic_imports = context.set_slot(self.dynamic_imports.clone());
    }

    fn start_bootstrap(
        &self,
        source: WorkerModuleSource,
        root_url: Url,
        static_import_initiator_url: Url,
    ) -> WorkerModuleBootstrapJob {
        WorkerModuleBootstrapJob {
            runtime: self.clone(),
            source,
            root_url,
            static_import_initiator_url,
        }
    }

    fn reserve_evaluation_id(&self) -> WorkerModuleEvaluationId {
        let mut next_evaluation_id = self.next_evaluation_id.borrow_mut();
        let evaluation_id = *next_evaluation_id;
        *next_evaluation_id += 1;
        evaluation_id
    }
}

pub(super) fn install_classic_worker_dynamic_module_runtime(
    scope: &mut v8::PinScope<'_, '_>,
    referrer_policy: Option<String>,
    evaluation_completion_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
) {
    WorkerModuleRuntime::new(
        RequestCredentialsMode::SameOrigin,
        referrer_policy,
        evaluation_completion_tx,
    )
    .install_context_slot(scope.get_current_context());
}

pub(super) fn run_next_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<WorkerDynamicModuleImportAdvance> {
    let context = scope.get_current_context();
    let dynamic_imports = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>()?;
    let mut imports = dynamic_imports.borrow_mut();
    let index = imports
        .pending_imports
        .iter()
        .position(|job| matches!(job.state, WorkerDynamicModuleImportJobState::Graph))?;
    let mut job = imports
        .pending_imports
        .remove(index)
        .expect("position came from pending_imports");
    drop(imports);
    match advance_worker_dynamic_module_import(scope, &mut job) {
        Ok(WorkerDynamicModuleImportAdvance::Complete) => {
            resolve_worker_dynamic_module_import(scope, job);
            Some(WorkerDynamicModuleImportAdvance::Complete)
        }
        Ok(WorkerDynamicModuleImportAdvance::NeedFetches(requests)) => {
            job.state = WorkerDynamicModuleImportJobState::Fetch(requests.clone());
            dynamic_imports.borrow_mut().pending_imports.push_back(job);
            Some(WorkerDynamicModuleImportAdvance::NeedFetches(requests))
        }
        Ok(WorkerDynamicModuleImportAdvance::WaitingFetches) => {
            dynamic_imports.borrow_mut().pending_imports.push_back(job);
            Some(WorkerDynamicModuleImportAdvance::WaitingFetches)
        }
        Ok(WorkerDynamicModuleImportAdvance::WaitingEvaluation { evaluation_id }) => {
            job.state = WorkerDynamicModuleImportJobState::Evaluation { evaluation_id };
            dynamic_imports.borrow_mut().pending_imports.push_back(job);
            Some(WorkerDynamicModuleImportAdvance::WaitingEvaluation { evaluation_id })
        }
        Err(error) => {
            finish_failed_worker_dynamic_module_import(scope, job, error);
            Some(WorkerDynamicModuleImportAdvance::Complete)
        }
    }
}

pub(super) fn worker_dynamic_module_import_waits_for_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    fetch_id: WorkerModuleGraphFetchId,
) -> bool {
    let context = scope.get_current_context();
    let Some(dynamic_imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        return false;
    };
    dynamic_imports.borrow().pending_imports.iter().any(|job| {
        matches!(
            &job.state,
            WorkerDynamicModuleImportJobState::Fetch(requests) if requests.contains_fetch_id(fetch_id)
        )
    })
}

pub(super) fn worker_has_pending_dynamic_module_imports(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let context = scope.get_current_context();
    let Some(dynamic_imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        return false;
    };
    !dynamic_imports.borrow().pending_imports.is_empty()
}

pub(super) fn worker_has_runnable_dynamic_module_imports(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let context = scope.get_current_context();
    let Some(dynamic_imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        return false;
    };
    dynamic_imports
        .borrow()
        .pending_imports
        .iter()
        .any(|job| matches!(job.state, WorkerDynamicModuleImportJobState::Graph))
}

pub(super) fn resume_worker_dynamic_module_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    completion: WorkerModuleGraphFetchCompletion,
) -> Option<WorkerDynamicModuleImportAdvance> {
    let context = scope.get_current_context();
    let dynamic_imports = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>()?;
    let mut imports = dynamic_imports.borrow_mut();
    let index = imports.pending_imports.iter().position(|job| {
        matches!(
            &job.state,
            WorkerDynamicModuleImportJobState::Fetch(requests)
                if requests.contains_fetch_id(completion.fetch_id())
        )
    })?;
    let mut job = imports
        .pending_imports
        .remove(index)
        .expect("position came from pending_imports");
    drop(imports);
    let fetch_id = completion.fetch_id();
    let (request, pending_keys) = match &mut job.state {
        WorkerDynamicModuleImportJobState::Fetch(requests) => {
            let request = requests
                .remove_by_fetch_id(fetch_id)
                .expect("position came from matching pending fetch id");
            let pending_keys = requests.pending_keys();
            (request, pending_keys)
        }
        _ => unreachable!(),
    };
    let had_pending_requests = !pending_keys.is_empty();
    match job_resume_fetch_with_pending_keys(scope, &mut job, &request, completion, pending_keys) {
        Ok(WorkerDynamicModuleImportAdvance::Complete) => {
            if had_pending_requests {
                dynamic_imports.borrow_mut().pending_imports.push_back(job);
                return Some(WorkerDynamicModuleImportAdvance::WaitingFetches);
            }
            resolve_worker_dynamic_module_import(scope, job);
            Some(WorkerDynamicModuleImportAdvance::Complete)
        }
        Ok(WorkerDynamicModuleImportAdvance::NeedFetches(requests)) => {
            let new_requests = requests.clone();
            match &mut job.state {
                WorkerDynamicModuleImportJobState::Fetch(pending_requests) => {
                    pending_requests.extend(requests);
                }
                _ => unreachable!(),
            }
            dynamic_imports.borrow_mut().pending_imports.push_back(job);
            Some(WorkerDynamicModuleImportAdvance::NeedFetches(new_requests))
        }
        Ok(WorkerDynamicModuleImportAdvance::WaitingFetches) => {
            dynamic_imports.borrow_mut().pending_imports.push_back(job);
            Some(WorkerDynamicModuleImportAdvance::WaitingFetches)
        }
        Ok(WorkerDynamicModuleImportAdvance::WaitingEvaluation { evaluation_id }) => {
            if had_pending_requests {
                dynamic_imports.borrow_mut().pending_imports.push_back(job);
                return Some(WorkerDynamicModuleImportAdvance::WaitingFetches);
            }
            job.state = WorkerDynamicModuleImportJobState::Evaluation { evaluation_id };
            dynamic_imports.borrow_mut().pending_imports.push_back(job);
            Some(WorkerDynamicModuleImportAdvance::WaitingEvaluation { evaluation_id })
        }
        Err(error) => {
            finish_failed_worker_dynamic_module_import(scope, job, error);
            Some(WorkerDynamicModuleImportAdvance::Complete)
        }
    }
}

pub(super) fn resume_worker_dynamic_module_evaluation(
    scope: &mut v8::PinScope<'_, '_>,
    completion: &WorkerModuleEvaluationCompletion,
) -> bool {
    let context = scope.get_current_context();
    let Some(dynamic_imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        return false;
    };
    let mut imports = dynamic_imports.borrow_mut();
    let Some(index) = imports.pending_imports.iter().position(|job| {
        matches!(
            job.state,
            WorkerDynamicModuleImportJobState::Evaluation { evaluation_id }
                if evaluation_id == completion.evaluation_id
        )
    }) else {
        return false;
    };
    let job = imports
        .pending_imports
        .remove(index)
        .expect("position came from pending_imports");
    drop(imports);
    match completion.result() {
        Ok(()) => resolve_worker_dynamic_module_import(scope, job),
        Err(message) => finish_failed_worker_dynamic_module_import(
            scope,
            job,
            WorkerDynamicModuleImportError {
                message: message.to_owned(),
                kind: WorkerDynamicModuleImportErrorKind::Type,
                rejection: None,
                stage: WorkerDynamicModuleImportErrorStage::Evaluate,
            },
        ),
    }
    true
}

impl Clone for WorkerModuleRuntime {
    fn clone(&self) -> Self {
        Self {
            graph: Rc::clone(&self.graph),
            next_fetch_id: Rc::clone(&self.next_fetch_id),
            next_evaluation_id: Rc::clone(&self.next_evaluation_id),
            dynamic_imports: Rc::clone(&self.dynamic_imports),
            evaluation_completion_tx: self.evaluation_completion_tx.clone(),
        }
    }
}

#[derive(Default)]
struct WorkerDynamicModuleResolver {
    pending_imports: VecDeque<WorkerDynamicModuleImportJob>,
}

impl WorkerDynamicModuleResolver {
    fn queue_import(
        &mut self,
        context: v8::Global<v8::Context>,
        resolver: v8::Global<v8::PromiseResolver>,
        specifier: String,
        base_url: Url,
        fetch_initiator_url: Url,
        attributes: ModuleAttributesKey,
        phase: ModuleImportPhase,
        audio_worklet_bootstrap: bool,
    ) {
        self.pending_imports
            .push_back(WorkerDynamicModuleImportJob::new(
                context,
                resolver,
                specifier,
                base_url,
                fetch_initiator_url,
                attributes,
                phase,
                audio_worklet_bootstrap,
            ));
    }

    fn root_import_in_flight(&self, key: &WorkerModuleKey) -> bool {
        self.pending_imports.iter().any(|job| {
            job.root_key.as_ref() == Some(key)
                && !matches!(
                    job.state,
                    WorkerDynamicModuleImportJobState::JoinedRoot { .. }
                )
        })
    }

    fn take_joined_root_imports(
        &mut self,
        key: &WorkerModuleKey,
    ) -> Vec<WorkerDynamicModuleImportJob> {
        let mut joined = Vec::new();
        let mut remaining = VecDeque::with_capacity(self.pending_imports.len());
        while let Some(job) = self.pending_imports.pop_front() {
            if matches!(
                &job.state,
                WorkerDynamicModuleImportJobState::JoinedRoot { key: joined_key }
                    if joined_key == key
            ) {
                joined.push(job);
            } else {
                remaining.push_back(job);
            }
        }
        self.pending_imports = remaining;
        joined
    }
}

struct WorkerDynamicModuleImportJob {
    context: v8::Global<v8::Context>,
    resolver: v8::Global<v8::PromiseResolver>,
    specifier: String,
    // Referencing module URL used only for specifier resolution.
    base_url: Url,
    // Worker inside-settings URL used for fetch security checks.
    fetch_initiator_url: Url,
    attributes: ModuleAttributesKey,
    phase: ModuleImportPhase,
    audio_worklet_bootstrap: bool,
    root_key: Option<WorkerModuleKey>,
    resolved_entry: Option<usize>,
    state: WorkerDynamicModuleImportJobState,
}

enum WorkerDynamicModuleImportJobState {
    Graph,
    JoinedRoot {
        key: WorkerModuleKey,
    },
    Fetch(WorkerModuleGraphFetchBatch),
    Evaluation {
        evaluation_id: WorkerModuleEvaluationId,
    },
}

impl WorkerDynamicModuleImportJob {
    fn new(
        context: v8::Global<v8::Context>,
        resolver: v8::Global<v8::PromiseResolver>,
        specifier: String,
        base_url: Url,
        fetch_initiator_url: Url,
        attributes: ModuleAttributesKey,
        phase: ModuleImportPhase,
        audio_worklet_bootstrap: bool,
    ) -> Self {
        Self {
            context,
            resolver,
            specifier,
            base_url,
            fetch_initiator_url,
            attributes,
            phase,
            audio_worklet_bootstrap,
            root_key: None,
            resolved_entry: None,
            state: WorkerDynamicModuleImportJobState::Graph,
        }
    }

    fn browser_request_metadata(&self) -> BrowserRequestMetadata {
        if self.audio_worklet_bootstrap {
            BrowserRequestMetadata::AudioWorklet
        } else {
            BrowserRequestMetadata::Fetch
        }
    }
}

fn advance_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: &mut WorkerDynamicModuleImportJob,
) -> WorkerDynamicModuleImportResult<WorkerDynamicModuleImportAdvance> {
    let context = scope.get_current_context();
    let graph = context
        .get_slot::<RefCell<WorkerModuleGraph>>()
        .ok_or_else(|| "worker module graph is not available".to_owned())?;
    let module_url = job
        .base_url
        .join(&job.specifier)
        .or_else(|_| Url::parse(&job.specifier))
        .map_err(|error| {
            format!(
                "Failed to resolve dynamic module worker import `{}`: {error}",
                job.specifier
            )
        })?;
    let module_key = worker_module_key_for_attributes(&module_url, &job.attributes)
        .map_err(|message| format!("{message} for dynamic import `{}`", job.specifier))?;
    job.root_key = Some(module_key.clone());
    if let Some(error) = graph.borrow().source_fetch_failure(&module_key) {
        return Err(WorkerDynamicModuleImportError::type_error(format!(
            "Failed to dynamically import module worker dependency `{module_url}`: {error}"
        )));
    }
    if context
        .get_slot::<RefCell<WorkerDynamicModuleResolver>>()
        .is_some_and(|dynamic_imports| dynamic_imports.borrow().root_import_in_flight(&module_key))
    {
        job.state = WorkerDynamicModuleImportJobState::JoinedRoot { key: module_key };
        return Ok(WorkerDynamicModuleImportAdvance::WaitingFetches);
    }
    if job.phase == ModuleImportPhase::Source && module_key.kind != WorkerModuleKind::WebAssembly {
        return Err(WorkerDynamicModuleImportError::syntax_error(format!(
            "source-phase dynamic import `{}` does not resolve to a WebAssembly module",
            job.specifier
        )));
    }
    let inherited_referrer_policy = graph.borrow().referrer_policy_for_url(&job.base_url);
    let existing_root_entry = graph.borrow().entry_for_key(&module_key);
    let root_entry = match existing_root_entry {
        Some(entry) => entry,
        None => match load_worker_static_module_dependency(&job.base_url, &job.specifier)? {
            WorkerModuleDependencyLoad::Source { url, source } => {
                let key =
                    worker_module_key_for_attributes(&url, &job.attributes).map_err(|message| {
                        format!("{message} for dynamic import `{}`", job.specifier)
                    })?;
                ensure_worker_module_entry(
                    scope,
                    &graph,
                    &source,
                    key,
                    url,
                    inherited_referrer_policy.clone(),
                )
                .map_err(|error| error.0.summary)?
            }
            WorkerModuleDependencyLoad::NeedFetch(_) => {
                let fetch_id = reserve_worker_module_graph_fetch_id(scope);
                return Ok(WorkerDynamicModuleImportAdvance::NeedFetches(
                    WorkerModuleGraphFetchBatch::single(WorkerModuleGraphFetchRequest::new(
                        fetch_id,
                        module_key,
                        job.fetch_initiator_url.clone(),
                        WorkerModuleGraphFetchCspSource::DynamicImportGraph,
                        None,
                        job.specifier.clone(),
                        job.attributes.clone(),
                        graph.borrow().credentials_mode(),
                        inherited_referrer_policy,
                        job.browser_request_metadata(),
                    )),
                ));
            }
        },
    };
    job.resolved_entry = Some(root_entry);
    match continue_worker_module_graph(
        scope,
        &graph,
        &job.fetch_initiator_url,
        WorkerModuleGraphFetchCspSource::DynamicImportGraph,
        job.browser_request_metadata(),
    )
    .map_err(|error| error.0.summary)?
    {
        WorkerModuleGraphBuild::Ready if job.phase == ModuleImportPhase::Source => {
            Ok(WorkerDynamicModuleImportAdvance::Complete)
        }
        WorkerModuleGraphBuild::Ready => {
            finish_worker_dynamic_module_import_evaluation(scope, root_entry)
        }
        WorkerModuleGraphBuild::NeedFetches(requests) => {
            Ok(WorkerDynamicModuleImportAdvance::NeedFetches(requests))
        }
    }
}

fn job_finish_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    job: &mut WorkerDynamicModuleImportJob,
    request: &WorkerModuleGraphFetchRequest,
    completion: WorkerModuleGraphFetchCompletion,
) -> WorkerDynamicModuleImportResult<()> {
    let context = scope.get_current_context();
    let graph = context
        .get_slot::<RefCell<WorkerModuleGraph>>()
        .ok_or_else(|| "worker module graph is not available".to_owned())?;
    let fetched_source = match completion.result {
        Ok(source) => source,
        Err(message) => {
            graph
                .borrow_mut()
                .mark_source_fetch_failed(&request.key, message.clone());
            return Err(format!(
                "Failed to dynamically import module worker dependency `{}`: {message}",
                request.url()
            )
            .into());
        }
    };
    let referrer_policy = fetched_source.effective_referrer_policy(request.referrer_policy());
    let target_entry = match ensure_worker_module_entry(
        scope,
        &graph,
        fetched_source.source(),
        request.key.clone(),
        fetched_source.final_url().clone(),
        referrer_policy,
    ) {
        Ok(entry) => entry,
        Err(error) => {
            let message = error.0.summary.clone();
            graph
                .borrow_mut()
                .mark_source_fetch_failed(&request.key, message.clone());
            return Err(message.into());
        }
    };
    if let Some(parent_entry) = request.parent_entry {
        graph.borrow_mut().add_dependency(
            parent_entry,
            request.specifier.clone(),
            request.attributes.clone(),
            target_entry,
        );
    } else {
        job.resolved_entry = Some(target_entry);
    }
    Ok(())
}

fn job_resume_fetch_with_pending_keys(
    scope: &mut v8::PinScope<'_, '_>,
    job: &mut WorkerDynamicModuleImportJob,
    request: &WorkerModuleGraphFetchRequest,
    completion: WorkerModuleGraphFetchCompletion,
    pending_keys: HashSet<WorkerModuleKey>,
) -> WorkerDynamicModuleImportResult<WorkerDynamicModuleImportAdvance> {
    let has_pending_requests = !pending_keys.is_empty();
    let context = scope.get_current_context();
    let graph = context
        .get_slot::<RefCell<WorkerModuleGraph>>()
        .ok_or_else(|| "worker module graph is not available".to_owned())?;
    job_finish_fetch(scope, job, request, completion)?;
    let root_entry = job
        .resolved_entry
        .ok_or_else(|| "dynamic import root module is not compiled".to_owned())?;
    match continue_worker_module_graph_with_pending_keys(
        scope,
        &graph,
        &request.initiator_url,
        request.csp_source(),
        request.graph_browser_request_metadata(),
        pending_keys,
    )
    .map_err(|error| error.0.summary)?
    {
        WorkerModuleGraphBuild::Ready if job.phase == ModuleImportPhase::Source => {
            if has_pending_requests {
                return Ok(WorkerDynamicModuleImportAdvance::WaitingFetches);
            }
            Ok(WorkerDynamicModuleImportAdvance::Complete)
        }
        WorkerModuleGraphBuild::Ready => {
            if has_pending_requests {
                return Ok(WorkerDynamicModuleImportAdvance::WaitingFetches);
            }
            finish_worker_dynamic_module_import_evaluation(scope, root_entry)
        }
        WorkerModuleGraphBuild::NeedFetches(requests) => {
            Ok(WorkerDynamicModuleImportAdvance::NeedFetches(requests))
        }
    }
}

fn finish_worker_dynamic_module_import_evaluation(
    scope: &mut v8::PinScope<'_, '_>,
    root_entry: usize,
) -> WorkerDynamicModuleImportResult<WorkerDynamicModuleImportAdvance> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let scope = &mut scope;
    let context = scope.get_current_context();
    let graph = context
        .get_slot::<RefCell<WorkerModuleGraph>>()
        .ok_or_else(|| "worker module graph is not available".to_owned())?;
    let root_module_global = graph.borrow().module(root_entry).clone();
    let root_module = v8::Local::new(scope, &root_module_global);
    if root_module.get_status() == v8::ModuleStatus::Uninstantiated {
        match root_module.instantiate_module2(
            scope,
            worker_resolve_static_module_callback,
            worker_resolve_static_source_callback,
        ) {
            Some(true) => {}
            Some(false) => {
                return Err(WorkerDynamicModuleImportError::type_error(
                    "v8 reported dynamic import instantiate failure",
                ));
            }
            None => {
                let exception = scope.exception();
                return Err(
                    WorkerDynamicModuleImportError::caught_instantiate_exception(
                        scope,
                        exception,
                        "dynamic import instantiate threw an exception",
                    ),
                );
            }
        }
    } else if root_module.get_status() == v8::ModuleStatus::Evaluated {
        return Ok(WorkerDynamicModuleImportAdvance::Complete);
    } else if root_module.get_status() == v8::ModuleStatus::Errored {
        return Err(WorkerDynamicModuleImportError::evaluation_rejection_value(
            scope,
            root_module.get_exception(),
            "dynamic import module evaluation failed",
        ));
    }
    let Some(value) = root_module.evaluate(scope) else {
        let exception = scope.exception();
        return Err(WorkerDynamicModuleImportError::caught_evaluation_exception(
            scope,
            exception,
            "dynamic import evaluation threw an exception",
        ));
    };
    scope.perform_microtask_checkpoint();
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
    if root_module.get_status() == v8::ModuleStatus::Errored {
        return Err(WorkerDynamicModuleImportError::evaluation_rejection_value(
            scope,
            root_module.get_exception(),
            "dynamic import module evaluation failed",
        ));
    }
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        match promise.state() {
            v8::PromiseState::Fulfilled => Ok(WorkerDynamicModuleImportAdvance::Complete),
            v8::PromiseState::Rejected => {
                let reason = promise.result(scope);
                Err(WorkerDynamicModuleImportError::evaluation_rejection_value(
                    scope,
                    reason,
                    "dynamic import module evaluation rejected",
                ))
            }
            v8::PromiseState::Pending => {
                let evaluation_id = reserve_worker_module_evaluation_id(scope)?;
                let promise = v8::Global::new(scope, promise);
                attach_worker_module_evaluation_reactions(scope, evaluation_id, promise)
                    .map_err(|error| error.0.summary)?;
                Ok(WorkerDynamicModuleImportAdvance::WaitingEvaluation { evaluation_id })
            }
        }
    } else {
        Ok(WorkerDynamicModuleImportAdvance::Complete)
    }
}

fn resolve_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
) {
    let resolved_entry = job.resolved_entry;
    let joined_imports = take_worker_dynamic_module_imports_joined_to_root(scope, &job);
    resolve_single_worker_dynamic_module_import(scope, job);
    for mut joined_job in joined_imports {
        joined_job.resolved_entry = resolved_entry;
        resolve_single_worker_dynamic_module_import(scope, joined_job);
    }
}

fn resolve_single_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
) {
    let context = v8::Local::new(scope, &job.context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(graph) = context.get_slot::<RefCell<WorkerModuleGraph>>() else {
        reject_worker_dynamic_module_import(
            scope,
            job,
            WorkerDynamicModuleImportError::type_error("worker module graph is not available"),
        );
        return;
    };
    let Some(entry) = job.resolved_entry else {
        reject_worker_dynamic_module_import(
            scope,
            job,
            WorkerDynamicModuleImportError::type_error("dynamic import module is not compiled"),
        );
        return;
    };
    let resolver = v8::Local::new(scope, &job.resolver);
    let resolved_value = {
        let graph = graph.borrow();
        let record = &graph.records[entry];
        match job.phase {
            ModuleImportPhase::Evaluation => {
                let module = v8::Local::new(scope, &record.module);
                Some(module.get_module_namespace())
            }
            ModuleImportPhase::Source => {
                let Some(wasm_record) = record.wasm_module.as_ref() else {
                    reject_worker_dynamic_module_import(
                        scope,
                        job,
                        WorkerDynamicModuleImportError::syntax_error(
                            "source-phase dynamic import did not resolve to a WebAssembly module",
                        ),
                    );
                    return;
                };
                wasm_record
                    .source_module(scope)
                    .map(v8::Local::<v8::Value>::from)
            }
        }
    };
    let Some(resolved_value) = resolved_value else {
        reject_worker_dynamic_module_import(
            scope,
            job,
            WorkerDynamicModuleImportError::type_error(
                "failed to materialize WebAssembly source for dynamic import",
            ),
        );
        return;
    };
    let _ = resolver.resolve(scope, resolved_value);
    scope.perform_microtask_checkpoint();
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
}

fn take_worker_dynamic_module_imports_joined_to_root(
    scope: &mut v8::PinScope<'_, '_>,
    job: &WorkerDynamicModuleImportJob,
) -> Vec<WorkerDynamicModuleImportJob> {
    let Some(root_key) = job.root_key.as_ref() else {
        return Vec::new();
    };
    let context = v8::Local::new(scope, &job.context);
    let Some(dynamic_imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        return Vec::new();
    };
    dynamic_imports
        .borrow_mut()
        .take_joined_root_imports(root_key)
}

fn reject_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
    error: WorkerDynamicModuleImportError,
) {
    let context = v8::Local::new(scope, &job.context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let resolver = v8::Local::new(scope, &job.resolver);
    let rejection = match error.rejection {
        Some(value) => v8::Local::new(scope, value),
        None => v8::String::new(scope, &error.message)
            .map(|message| match error.kind {
                WorkerDynamicModuleImportErrorKind::Type => {
                    v8::Exception::type_error(scope, message)
                }
                WorkerDynamicModuleImportErrorKind::Syntax => {
                    v8::Exception::syntax_error(scope, message)
                }
            })
            .unwrap_or_else(|| v8::undefined(scope).into()),
    };
    let _ = resolver.reject(scope, rejection);
    scope.perform_microtask_checkpoint();
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
}

fn finish_failed_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
    error: WorkerDynamicModuleImportError,
) {
    let resolved_entry = job.resolved_entry;
    let joined_imports = take_worker_dynamic_module_imports_joined_to_root(scope, &job);
    let joined_errors = joined_imports
        .iter()
        .map(|_| error.clone_for_scope(scope))
        .collect::<Vec<_>>();
    finish_failed_single_worker_dynamic_module_import(scope, job, error);
    for (mut joined_job, joined_error) in joined_imports.into_iter().zip(joined_errors) {
        joined_job.resolved_entry = resolved_entry;
        finish_failed_single_worker_dynamic_module_import(scope, joined_job, joined_error);
    }
}

fn finish_failed_single_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
    error: WorkerDynamicModuleImportError,
) {
    if job.audio_worklet_bootstrap && error.stage == WorkerDynamicModuleImportErrorStage::Evaluate {
        resolve_worker_dynamic_module_import(scope, job);
    } else {
        reject_worker_dynamic_module_import(scope, job, error);
    }
}

pub(super) struct WorkerModuleBootstrapJob {
    runtime: WorkerModuleRuntime,
    source: WorkerModuleSource,
    root_url: Url,
    static_import_initiator_url: Url,
}

impl WorkerModuleBootstrapJob {
    fn script_url(&self) -> &str {
        self.root_url.as_str()
    }

    fn advance(&mut self, scope: &mut v8::PinScope<'_, '_>) -> WorkerModuleAdvance {
        let root_key = worker_module_root_key_for_source(&self.root_url, &self.source);
        let root_referrer_policy = self
            .runtime
            .graph
            .borrow()
            .root_referrer_policy()
            .map(str::to_owned);
        let root_entry = match ensure_worker_module_entry(
            scope,
            &self.runtime.graph,
            &self.source,
            root_key,
            self.root_url.clone(),
            root_referrer_policy,
        ) {
            Ok(root_entry) => root_entry,
            Err(error) => return WorkerModuleAdvance::Failed(error),
        };
        match continue_worker_module_graph(
            scope,
            &self.runtime.graph,
            &self.static_import_initiator_url,
            WorkerModuleGraphFetchCspSource::StaticModuleGraph,
            BrowserRequestMetadata::Fetch,
        ) {
            Ok(WorkerModuleGraphBuild::Ready) => {
                WorkerModuleAdvance::ReadyToInstantiate { root_entry }
            }
            Ok(WorkerModuleGraphBuild::NeedFetches(requests)) => {
                WorkerModuleAdvance::NeedFetches(requests)
            }
            Err(error) => WorkerModuleAdvance::Failed(error),
        }
    }

    fn finish_fetch(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        request: &WorkerModuleGraphFetchRequest,
        completion: WorkerModuleGraphFetchCompletion,
    ) -> WorkerModuleBootstrapResult<()> {
        let fetched_source = match completion.result {
            Ok(source) => source,
            Err(message) => {
                return Err(Box::new(worker_bootstrap_error(
                    scope,
                    request.initiator_url.as_str(),
                    &message,
                    WorkerParentErrorEventKind::Event,
                )));
            }
        };
        let referrer_policy = fetched_source.effective_referrer_policy(request.referrer_policy());
        let target_entry = ensure_worker_module_entry(
            scope,
            &self.runtime.graph,
            fetched_source.source(),
            request.key.clone(),
            fetched_source.final_url().clone(),
            referrer_policy,
        )?;
        self.runtime.graph.borrow_mut().add_dependency(
            request
                .parent_entry
                .expect("bootstrap graph fetch must have a parent entry"),
            request.specifier.clone(),
            request.attributes.clone(),
            target_entry,
        );
        Ok(())
    }

    fn resume_fetch_with_pending_keys(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        request: &WorkerModuleGraphFetchRequest,
        completion: WorkerModuleGraphFetchCompletion,
        pending_keys: HashSet<WorkerModuleKey>,
    ) -> WorkerModuleAdvance {
        if let Err(error) = self.finish_fetch(scope, request, completion) {
            return WorkerModuleAdvance::Failed(error);
        }
        let root_key = worker_module_root_key_for_source(&self.root_url, &self.source);
        let root_entry = match self.runtime.graph.borrow().entry_for_key(&root_key) {
            Some(entry) => entry,
            None => {
                return WorkerModuleAdvance::Failed(Box::new(worker_bootstrap_error(
                    scope,
                    self.root_url.as_str(),
                    "Module worker root disappeared while resuming graph fetch",
                    WorkerParentErrorEventKind::Event,
                )));
            }
        };
        match continue_worker_module_graph_with_pending_keys(
            scope,
            &self.runtime.graph,
            &self.static_import_initiator_url,
            WorkerModuleGraphFetchCspSource::StaticModuleGraph,
            BrowserRequestMetadata::Fetch,
            pending_keys,
        ) {
            Ok(WorkerModuleGraphBuild::Ready) => {
                WorkerModuleAdvance::ReadyToInstantiate { root_entry }
            }
            Ok(WorkerModuleGraphBuild::NeedFetches(requests)) => {
                WorkerModuleAdvance::NeedFetches(requests)
            }
            Err(error) => WorkerModuleAdvance::Failed(error),
        }
    }
}

fn worker_module_root_key_for_source(url: &Url, source: &WorkerModuleSource) -> WorkerModuleKey {
    match source {
        WorkerModuleSource::Text(_) => WorkerModuleKey::java_script(url.clone()),
        WorkerModuleSource::Binary(_) => WorkerModuleKey::webassembly(url.clone()),
    }
}

enum WorkerModuleAdvance {
    ReadyToInstantiate { root_entry: usize },
    NeedFetches(WorkerModuleGraphFetchBatch),
    Failed(Box<WorkerBootstrapError>),
}

#[derive(Clone)]
pub(super) struct WorkerModuleGraphFetchRequest {
    fetch_id: WorkerModuleGraphFetchId,
    key: WorkerModuleKey,
    initiator_url: Url,
    csp_source: WorkerModuleGraphFetchCspSource,
    parent_entry: Option<usize>,
    specifier: String,
    attributes: ModuleAttributesKey,
    credentials_mode: RequestCredentialsMode,
    referrer_policy: Option<String>,
    browser_request_metadata: BrowserRequestMetadata,
}

#[derive(Clone)]
pub(super) struct WorkerModuleGraphFetchBatch {
    requests: Vec<WorkerModuleGraphFetchRequest>,
}

impl WorkerModuleGraphFetchBatch {
    fn new(requests: Vec<WorkerModuleGraphFetchRequest>) -> Self {
        debug_assert!(
            !requests.is_empty(),
            "worker module graph fetch batch should not be empty"
        );
        Self { requests }
    }

    fn single(request: WorkerModuleGraphFetchRequest) -> Self {
        Self::new(vec![request])
    }

    fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub(super) fn contains_fetch_id(&self, fetch_id: WorkerModuleGraphFetchId) -> bool {
        self.requests
            .iter()
            .any(|request| request.fetch_id == fetch_id)
    }

    fn remove_by_fetch_id(
        &mut self,
        fetch_id: WorkerModuleGraphFetchId,
    ) -> Option<WorkerModuleGraphFetchRequest> {
        let index = self
            .requests
            .iter()
            .position(|request| request.fetch_id == fetch_id)?;
        Some(self.requests.remove(index))
    }

    fn extend(&mut self, other: WorkerModuleGraphFetchBatch) {
        self.requests.extend(other.requests);
    }

    fn pending_keys(&self) -> HashSet<WorkerModuleKey> {
        self.requests
            .iter()
            .map(|request| request.key.clone())
            .collect()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &WorkerModuleGraphFetchRequest> {
        self.requests.iter()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkerModuleGraphFetchCspSource {
    StaticModuleGraph,
    DynamicImportGraph,
}

impl WorkerModuleGraphFetchRequest {
    fn new(
        fetch_id: WorkerModuleGraphFetchId,
        key: WorkerModuleKey,
        initiator_url: Url,
        csp_source: WorkerModuleGraphFetchCspSource,
        parent_entry: Option<usize>,
        specifier: String,
        attributes: ModuleAttributesKey,
        credentials_mode: RequestCredentialsMode,
        referrer_policy: Option<String>,
        browser_request_metadata: BrowserRequestMetadata,
    ) -> Self {
        Self {
            fetch_id,
            key,
            initiator_url,
            csp_source,
            parent_entry,
            specifier,
            attributes,
            credentials_mode,
            referrer_policy,
            browser_request_metadata,
        }
    }

    pub(super) fn fetch_id(&self) -> WorkerModuleGraphFetchId {
        self.fetch_id
    }

    pub(super) fn url(&self) -> &Url {
        &self.key.url
    }

    pub(super) fn initiator_url(&self) -> &Url {
        &self.initiator_url
    }

    pub(super) fn csp_source(&self) -> WorkerModuleGraphFetchCspSource {
        self.csp_source
    }

    pub(super) fn module_type(&self) -> Option<&str> {
        self.attributes.module_type()
    }

    pub(super) fn kind(&self) -> WorkerModuleKind {
        self.key.kind
    }

    pub(super) fn credentials_mode(&self) -> RequestCredentialsMode {
        self.credentials_mode
    }

    pub(super) fn referrer_policy(&self) -> Option<&str> {
        self.referrer_policy.as_deref()
    }

    pub(super) fn browser_request_metadata(&self) -> BrowserRequestMetadata {
        worker_module_browser_request_metadata(self.key.kind, self.browser_request_metadata)
    }

    fn graph_browser_request_metadata(&self) -> BrowserRequestMetadata {
        self.browser_request_metadata
    }
}

fn worker_module_browser_request_metadata(
    kind: WorkerModuleKind,
    graph_metadata: BrowserRequestMetadata,
) -> BrowserRequestMetadata {
    match kind {
        WorkerModuleKind::Json => BrowserRequestMetadata::JsonModule,
        WorkerModuleKind::JavaScript | WorkerModuleKind::WebAssembly => graph_metadata,
    }
}

struct WorkerModuleGraph {
    credentials_mode: RequestCredentialsMode,
    root_referrer_policy: Option<String>,
    failed_source_fetches: HashMap<WorkerModuleKey, String>,
    records: Vec<WorkerModuleRecord>,
    entries_by_key: HashMap<WorkerModuleKey, usize>,
    module_to_entries: HashMap<i32, Vec<usize>>,
}

struct WorkerModuleRecord {
    key: WorkerModuleKey,
    source_url: Url,
    source: WorkerModuleSource,
    module: v8::Global<v8::Module>,
    requests: Vec<WorkerModuleRequest>,
    dependencies: Vec<WorkerModuleDependency>,
    wasm_module: Option<WasmModuleRecord>,
    referrer_policy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WorkerModuleKey {
    url: Url,
    kind: WorkerModuleKind,
    attributes: ModuleAttributesKey,
}

impl WorkerModuleKey {
    fn java_script(url: Url) -> Self {
        Self::java_script_with_attributes(url, ModuleAttributesKey::empty())
    }

    fn java_script_with_attributes(url: Url, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            kind: WorkerModuleKind::JavaScript,
            attributes,
        }
    }

    fn json(url: Url, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            kind: WorkerModuleKind::Json,
            attributes,
        }
    }

    fn webassembly(url: Url) -> Self {
        Self {
            url,
            kind: WorkerModuleKind::WebAssembly,
            attributes: ModuleAttributesKey::empty(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum WorkerModuleKind {
    JavaScript,
    Json,
    WebAssembly,
}

#[derive(Clone)]
struct WorkerModuleRequest {
    specifier: String,
    attributes: ModuleAttributesKey,
    phase: ModuleImportPhase,
}

struct WorkerModuleDependency {
    specifier: String,
    attributes: ModuleAttributesKey,
    target_entry: usize,
}

enum WorkerSyntheticModuleRecord {
    Json(String),
    WebAssembly(WasmModuleRecord),
}

impl WorkerModuleGraph {
    fn new(credentials_mode: RequestCredentialsMode, root_referrer_policy: Option<String>) -> Self {
        Self {
            credentials_mode,
            root_referrer_policy,
            failed_source_fetches: HashMap::new(),
            records: Vec::new(),
            entries_by_key: HashMap::new(),
            module_to_entries: HashMap::new(),
        }
    }

    fn credentials_mode(&self) -> RequestCredentialsMode {
        self.credentials_mode
    }

    fn root_referrer_policy(&self) -> Option<&str> {
        self.root_referrer_policy.as_deref()
    }

    fn referrer_policy(&self, entry: usize) -> Option<&str> {
        self.records[entry].referrer_policy.as_deref()
    }

    fn referrer_policy_for_url(&self, url: &Url) -> Option<String> {
        self.records
            .iter()
            .find(|record| record.key.url == *url || record.source_url == *url)
            .and_then(|record| record.referrer_policy.clone())
            .or_else(|| self.root_referrer_policy.clone())
    }

    fn entry_for_key(&self, key: &WorkerModuleKey) -> Option<usize> {
        self.entries_by_key.get(key).copied()
    }

    fn source_fetch_failure(&self, key: &WorkerModuleKey) -> Option<String> {
        self.failed_source_fetches.get(key).cloned()
    }

    fn mark_source_fetch_failed(&mut self, key: &WorkerModuleKey, message: String) {
        self.failed_source_fetches.insert(key.clone(), message);
    }

    fn insert(
        &mut self,
        key: WorkerModuleKey,
        source_url: Url,
        source: WorkerModuleSource,
        module: v8::Global<v8::Module>,
        identity_hash: i32,
        requests: Vec<WorkerModuleRequest>,
        wasm_module: Option<WasmModuleRecord>,
        referrer_policy: Option<String>,
    ) -> usize {
        let entry = self.records.len();
        self.records.push(WorkerModuleRecord {
            key: key.clone(),
            source_url,
            source,
            module,
            requests,
            dependencies: Vec::new(),
            wasm_module,
            referrer_policy,
        });
        self.entries_by_key.insert(key, entry);
        self.module_to_entries
            .entry(identity_hash)
            .or_default()
            .push(entry);
        entry
    }

    fn module(&self, entry: usize) -> &v8::Global<v8::Module> {
        &self.records[entry].module
    }

    fn requests(&self, entry: usize) -> Vec<WorkerModuleRequest> {
        self.records[entry].requests.clone()
    }

    fn add_dependency(
        &mut self,
        entry: usize,
        specifier: String,
        attributes: ModuleAttributesKey,
        target_entry: usize,
    ) {
        self.records[entry]
            .dependencies
            .push(WorkerModuleDependency {
                specifier,
                attributes,
                target_entry,
            });
    }

    fn len(&self) -> usize {
        self.records.len()
    }

    fn url(&self, entry: usize) -> &Url {
        &self.records[entry].source_url
    }

    fn has_dependency(
        &self,
        entry: usize,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> bool {
        self.records[entry].dependencies.iter().any(|dependency| {
            dependency.specifier == specifier && dependency.attributes == *attributes
        })
    }

    fn module_url_for(&self, module: v8::Local<'_, v8::Module>) -> Option<Url> {
        let entry = self.entry_for_module(module)?;
        Some(self.records[entry].source_url.clone())
    }

    fn wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        for record in &self.records {
            let Some(wasm_record) = record.wasm_module.as_ref() else {
                continue;
            };
            let module = v8::Local::new(scope, &record.module);
            if !matches!(
                module.get_status(),
                v8::ModuleStatus::Instantiated
                    | v8::ModuleStatus::Evaluating
                    | v8::ModuleStatus::Evaluated
            ) {
                continue;
            }
            let candidate = module.get_module_namespace();
            if namespace.strict_equals(candidate) {
                return wasm_record.instance(scope);
            }
        }
        None
    }

    fn synthetic_module_record_for(
        &self,
        module: v8::Local<'_, v8::Module>,
    ) -> Option<WorkerSyntheticModuleRecord> {
        let entry = self.entry_for_module(module)?;
        let record = &self.records[entry];
        match record.key.kind {
            WorkerModuleKind::Json => record
                .source
                .text_source()
                .map(|source| WorkerSyntheticModuleRecord::Json(source.to_owned())),
            WorkerModuleKind::WebAssembly => record
                .wasm_module
                .clone()
                .map(WorkerSyntheticModuleRecord::WebAssembly),
            WorkerModuleKind::JavaScript => None,
        }
    }

    fn resolve_static_dependency_entry(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<usize> {
        let referrer_entry = self.entry_for_module(referrer)?;
        let dependency = self.records[referrer_entry]
            .dependencies
            .iter()
            .find(|dependency| {
                dependency.specifier == specifier && dependency.attributes == *attributes
            })?;
        Some(dependency.target_entry)
    }

    fn resolve_static_dependency_record(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<&WorkerModuleRecord> {
        let dependency_entry =
            self.resolve_static_dependency_entry(referrer, specifier, attributes)?;
        Some(&self.records[dependency_entry])
    }

    fn resolve_static_dependency(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<&v8::Global<v8::Module>> {
        Some(
            &self
                .resolve_static_dependency_record(referrer, specifier, attributes)?
                .module,
        )
    }

    fn entry_for_module(&self, module: v8::Local<'_, v8::Module>) -> Option<usize> {
        let identity_hash = module.get_identity_hash().get();
        self.module_to_entries
            .get(&identity_hash)?
            .iter()
            .copied()
            .find(|entry| module == self.records[*entry].module)
    }
}

type WorkerModuleBootstrapResult<T> = Result<T, Box<WorkerBootstrapError>>;

enum WorkerModuleGraphBuild {
    Ready,
    NeedFetches(WorkerModuleGraphFetchBatch),
}

fn ensure_worker_module_entry(
    scope: &mut v8::PinScope<'_, '_>,
    graph: &Rc<RefCell<WorkerModuleGraph>>,
    source: &WorkerModuleSource,
    key: WorkerModuleKey,
    source_url: Url,
    referrer_policy: Option<String>,
) -> WorkerModuleBootstrapResult<usize> {
    let existing_entry = graph.borrow().entry_for_key(&key);
    if let Some(entry) = existing_entry {
        return Ok(entry);
    }
    let (module, identity_hash, requests, wasm_module) =
        compile_worker_module_record(scope, source, &key, &source_url)?;
    Ok(graph.borrow_mut().insert(
        key,
        source_url,
        source.clone(),
        module,
        identity_hash,
        requests,
        wasm_module,
        referrer_policy,
    ))
}

fn continue_worker_module_graph(
    scope: &mut v8::PinScope<'_, '_>,
    graph: &Rc<RefCell<WorkerModuleGraph>>,
    fetch_initiator_url: &Url,
    csp_source: WorkerModuleGraphFetchCspSource,
    browser_request_metadata: BrowserRequestMetadata,
) -> WorkerModuleBootstrapResult<WorkerModuleGraphBuild> {
    continue_worker_module_graph_with_pending_keys(
        scope,
        graph,
        fetch_initiator_url,
        csp_source,
        browser_request_metadata,
        HashSet::new(),
    )
}

fn continue_worker_module_graph_with_pending_keys(
    scope: &mut v8::PinScope<'_, '_>,
    graph: &Rc<RefCell<WorkerModuleGraph>>,
    fetch_initiator_url: &Url,
    csp_source: WorkerModuleGraphFetchCspSource,
    browser_request_metadata: BrowserRequestMetadata,
    mut pending_keys: HashSet<WorkerModuleKey>,
) -> WorkerModuleBootstrapResult<WorkerModuleGraphBuild> {
    let mut entry = 0;
    let mut pending_requests = Vec::new();
    while entry < graph.borrow().len() {
        let url = graph.borrow().url(entry).clone();
        let requests = graph.borrow().requests(entry);
        for request in requests {
            if graph
                .borrow()
                .has_dependency(entry, &request.specifier, &request.attributes)
            {
                continue;
            }
            match resolve_worker_module_dependency(
                scope,
                graph,
                entry,
                &url,
                fetch_initiator_url,
                csp_source,
                browser_request_metadata,
                request,
                &mut pending_keys,
            )? {
                WorkerModuleGraphBuild::Ready => {}
                WorkerModuleGraphBuild::NeedFetches(requests) => {
                    pending_requests.extend(requests.requests);
                }
            }
        }
        entry += 1;
    }
    if !pending_requests.is_empty() {
        return Ok(WorkerModuleGraphBuild::NeedFetches(
            WorkerModuleGraphFetchBatch::new(pending_requests),
        ));
    }
    Ok(WorkerModuleGraphBuild::Ready)
}

fn resolve_worker_module_dependency(
    scope: &mut v8::PinScope<'_, '_>,
    graph: &Rc<RefCell<WorkerModuleGraph>>,
    entry: usize,
    url: &Url,
    fetch_initiator_url: &Url,
    csp_source: WorkerModuleGraphFetchCspSource,
    browser_request_metadata: BrowserRequestMetadata,
    request: WorkerModuleRequest,
    pending_keys: &mut HashSet<WorkerModuleKey>,
) -> WorkerModuleBootstrapResult<WorkerModuleGraphBuild> {
    if request.phase == ModuleImportPhase::Source {
        let dependency_url = url
            .join(&request.specifier)
            .or_else(|_| Url::parse(&request.specifier))
            .map_err(|error| {
                Box::new(worker_bootstrap_error(
                    scope,
                    url.as_str(),
                    &format!(
                        "Failed to resolve module worker dependency `{}`: {error}",
                        request.specifier
                    ),
                    WorkerParentErrorEventKind::Event,
                ))
            })?;
        let dependency_key = worker_module_key_for_attributes(&dependency_url, &request.attributes)
            .map_err(|message| {
                Box::new(worker_bootstrap_error(
                    scope,
                    url.as_str(),
                    &format!("{message} for import `{}`", request.specifier),
                    WorkerParentErrorEventKind::Event,
                ))
            })?;
        if dependency_key.kind != WorkerModuleKind::WebAssembly {
            return Err(Box::new(worker_bootstrap_error(
                scope,
                url.as_str(),
                &format!(
                    "source-phase import `{}` does not resolve to a WebAssembly module",
                    request.specifier
                ),
                WorkerParentErrorEventKind::Event,
            )));
        }
    }
    let (dependency_key, dependency_source) =
        match load_worker_static_module_dependency(url, &request.specifier).map_err(|message| {
            Box::new(worker_bootstrap_error(
                scope,
                url.as_str(),
                &message,
                WorkerParentErrorEventKind::Event,
            ))
        })? {
            WorkerModuleDependencyLoad::Source { url, source } => {
                let dependency_key = worker_module_key_for_attributes(&url, &request.attributes)
                    .map_err(|message| {
                        Box::new(worker_bootstrap_error(
                            scope,
                            url.as_str(),
                            &format!("{message} for import `{}`", request.specifier),
                            WorkerParentErrorEventKind::Event,
                        ))
                    })?;
                (dependency_key, source)
            }
            WorkerModuleDependencyLoad::NeedFetch(dependency_url) => {
                let dependency_key =
                    worker_module_key_for_attributes(&dependency_url, &request.attributes)
                        .map_err(|message| {
                            Box::new(worker_bootstrap_error(
                                scope,
                                url.as_str(),
                                &format!("{message} for import `{}`", request.specifier),
                                WorkerParentErrorEventKind::Event,
                            ))
                        })?;
                let existing_entry = graph.borrow().entry_for_key(&dependency_key);
                if let Some(target_entry) = existing_entry {
                    graph.borrow_mut().add_dependency(
                        entry,
                        request.specifier,
                        request.attributes,
                        target_entry,
                    );
                    return Ok(WorkerModuleGraphBuild::Ready);
                }
                if !pending_keys.insert(dependency_key.clone()) {
                    return Ok(WorkerModuleGraphBuild::Ready);
                }
                let fetch_id = reserve_worker_module_graph_fetch_id(scope);
                let referrer_policy = graph.borrow().referrer_policy(entry).map(str::to_owned);
                return Ok(WorkerModuleGraphBuild::NeedFetches(
                    WorkerModuleGraphFetchBatch::single(WorkerModuleGraphFetchRequest::new(
                        fetch_id,
                        dependency_key,
                        fetch_initiator_url.clone(),
                        csp_source,
                        Some(entry),
                        request.specifier,
                        request.attributes,
                        graph.borrow().credentials_mode(),
                        referrer_policy,
                        browser_request_metadata,
                    )),
                ));
            }
        };
    let referrer_policy = graph.borrow().referrer_policy(entry).map(str::to_owned);
    let target_entry = ensure_worker_module_entry(
        scope,
        graph,
        &dependency_source,
        dependency_key.clone(),
        dependency_key.url.clone(),
        referrer_policy,
    )?;
    graph
        .borrow_mut()
        .add_dependency(entry, request.specifier, request.attributes, target_entry);
    Ok(WorkerModuleGraphBuild::Ready)
}

fn reserve_worker_module_graph_fetch_id(
    scope: &mut v8::PinScope<'_, '_>,
) -> WorkerModuleGraphFetchId {
    let slot = scope
        .get_current_context()
        .get_slot::<WorkerModuleRuntimeFetchIdSlot>()
        .expect("worker module runtime fetch id slot should be installed");
    let mut next_fetch_id = slot.next_fetch_id.borrow_mut();
    let fetch_id = *next_fetch_id;
    *next_fetch_id = next_fetch_id
        .checked_add(1)
        .expect("worker module graph fetch id space exhausted");
    fetch_id
}

fn reserve_worker_module_evaluation_id(
    scope: &mut v8::PinScope<'_, '_>,
) -> Result<WorkerModuleEvaluationId, String> {
    let slot = scope
        .get_current_context()
        .get_slot::<WorkerModuleRuntimeEvaluationSlot>()
        .ok_or_else(|| "worker module runtime evaluation slot is not installed".to_owned())?;
    let mut next_evaluation_id = slot.next_evaluation_id.borrow_mut();
    let evaluation_id = *next_evaluation_id;
    *next_evaluation_id += 1;
    Ok(evaluation_id)
}

fn attach_worker_module_evaluation_reactions(
    scope: &mut v8::PinScope<'_, '_>,
    evaluation_id: WorkerModuleEvaluationId,
    promise: v8::Global<v8::Promise>,
) -> WorkerModuleBootstrapResult<()> {
    let data = WorkerModuleEvaluationReactionDataDeclaration {
        evaluation_id: evaluation_id as f64,
    }
    .bind(scope)
    .expect("worker module evaluation reaction data declaration should bind");
    let on_fulfilled = v8::Function::builder(worker_module_evaluation_fulfilled_callback)
        .data(data.into())
        .build(scope)
        .ok_or_else(|| {
            Box::new(worker_bootstrap_error(
                scope,
                "worker module",
                "failed to create worker module evaluation success reaction",
                WorkerParentErrorEventKind::Event,
            ))
        })?;
    let on_rejected = v8::Function::builder(worker_module_evaluation_rejected_callback)
        .data(data.into())
        .build(scope)
        .ok_or_else(|| {
            Box::new(worker_bootstrap_error(
                scope,
                "worker module",
                "failed to create worker module evaluation failure reaction",
                WorkerParentErrorEventKind::Event,
            ))
        })?;
    let promise = v8::Local::new(scope, &promise);
    promise
        .then2(scope, on_fulfilled, on_rejected)
        .map(|_| ())
        .ok_or_else(|| {
            Box::new(worker_bootstrap_error(
                scope,
                "worker module",
                "failed to attach worker module evaluation reactions",
                WorkerParentErrorEventKind::Event,
            ))
        })
}

fn compile_worker_module_record(
    scope: &mut v8::PinScope<'_, '_>,
    source: &WorkerModuleSource,
    key: &WorkerModuleKey,
    source_url: &Url,
) -> WorkerModuleBootstrapResult<(
    v8::Global<v8::Module>,
    i32,
    Vec<WorkerModuleRequest>,
    Option<WasmModuleRecord>,
)> {
    if matches!(key.kind, WorkerModuleKind::Json) {
        return compile_worker_synthetic_module_record(scope, source_url);
    }
    if key.kind == WorkerModuleKind::WebAssembly {
        let Some(bytes) = source.binary_source() else {
            return Err(Box::new(worker_bootstrap_error(
                scope,
                source_url.as_str(),
                "WebAssembly module worker source is not binary",
                WorkerParentErrorEventKind::Event,
            )));
        };
        return compile_worker_wasm_module_record(scope, bytes, source_url);
    }
    let Some(source) = source.text_source() else {
        return Err(Box::new(worker_bootstrap_error(
            scope,
            source_url.as_str(),
            "JavaScript module worker source is not text",
            WorkerParentErrorEventKind::Event,
        )));
    };
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let source_str = v8::String::new(&scope, source).expect("v8 string allocation");
    let origin = create_module_script_origin(&mut scope, source_url.as_str());
    let mut compiler_source = v8::script_compiler::Source::new(source_str, Some(&origin));
    let Some(module) = v8::script_compiler::compile_module(&scope, &mut compiler_source) else {
        let exception = scope.exception();
        let message = scope.message();
        let stack_trace = scope.stack_trace();
        let report =
            build_event_handler_exception_report(&mut scope, exception, message, stack_trace);
        return Err(Box::new((
            report,
            exception.map(|value| v8::Global::new(&scope, value)),
            WorkerParentErrorEventKind::Event,
        )));
    };
    let requests = collect_worker_module_requests(&mut scope, module)?;
    let identity_hash = module.get_identity_hash().get();
    Ok((
        v8::Global::new(&scope, module),
        identity_hash,
        requests,
        None,
    ))
}

fn compile_worker_synthetic_module_record(
    scope: &mut v8::PinScope<'_, '_>,
    source_url: &Url,
) -> WorkerModuleBootstrapResult<(
    v8::Global<v8::Module>,
    i32,
    Vec<WorkerModuleRequest>,
    Option<WasmModuleRecord>,
)> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let module_name = v8::String::new(&scope, source_url.as_str()).expect("v8 string allocation");
    let default_export = v8::String::new(&scope, "default").expect("v8 string allocation");
    let module = v8::Module::create_synthetic_module(
        &scope,
        module_name,
        &[default_export],
        worker_synthetic_module_evaluation_steps,
    );
    let identity_hash = module.get_identity_hash().get();
    Ok((
        v8::Global::new(scope.as_ref(), module),
        identity_hash,
        Vec::new(),
        None,
    ))
}

fn compile_worker_wasm_module_record(
    scope: &mut v8::PinScope<'_, '_>,
    bytes: &[u8],
    source_url: &Url,
) -> WorkerModuleBootstrapResult<(
    v8::Global<v8::Module>,
    i32,
    Vec<WorkerModuleRequest>,
    Option<WasmModuleRecord>,
)> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let prepared = prepare_wasm_module_record(&scope, bytes)
        .map_err(|error| {
            Box::new(worker_bootstrap_error(
                &mut scope,
                source_url.as_str(),
                &error.to_string(),
                WorkerParentErrorEventKind::Event,
            ))
        })?
        .ok_or_else(|| {
            let exception = v8_exception_message_or(
                &scope,
                scope.exception(),
                "unknown wasm compile exception",
            );
            Box::new(worker_bootstrap_error(
                &mut scope,
                source_url.as_str(),
                &format!("v8 failed to compile WebAssembly module `{source_url}`: {exception}"),
                WorkerParentErrorEventKind::Event,
            ))
        })?;
    let requests = if prepared.has_reserved_name_link_error {
        Vec::new()
    } else {
        worker_wasm_module_requests_for_imports(prepared.record.imports())
    };
    let module_name = v8_string(&scope, source_url.as_str()).ok_or_else(|| {
        Box::new(worker_bootstrap_error(
            &mut scope,
            source_url.as_str(),
            "failed to allocate WebAssembly synthetic module worker name",
            WorkerParentErrorEventKind::Event,
        ))
    })?;
    let mut export_names = Vec::with_capacity(prepared.record.exports().len());
    for export in prepared.record.exports() {
        let Some(export_name) = v8_string(&scope, export.name()) else {
            return Err(Box::new(worker_bootstrap_error(
                &mut scope,
                source_url.as_str(),
                "failed to allocate WebAssembly synthetic export name",
                WorkerParentErrorEventKind::Event,
            )));
        };
        export_names.push(export_name);
    }
    let module = v8::Module::create_synthetic_module(
        &scope,
        module_name,
        &export_names,
        worker_synthetic_module_evaluation_steps,
    );
    let identity_hash = module.get_identity_hash().get();
    Ok((
        v8::Global::new(&scope, module),
        identity_hash,
        requests,
        Some(prepared.record),
    ))
}

fn worker_wasm_module_requests_for_imports(
    imports: &[WasmImportRecord],
) -> Vec<WorkerModuleRequest> {
    wasm_evaluation_import_modules(imports)
        .into_iter()
        .map(|module| WorkerModuleRequest {
            specifier: module.to_owned(),
            attributes: ModuleAttributesKey::empty(),
            phase: ModuleImportPhase::Evaluation,
        })
        .collect()
}

fn collect_worker_module_requests(
    scope: &mut v8::PinScope<'_, '_>,
    module: v8::Local<'_, v8::Module>,
) -> WorkerModuleBootstrapResult<Vec<WorkerModuleRequest>> {
    let requests = module.get_module_requests();
    let mut records = Vec::with_capacity(requests.length());
    for index in 0..requests.length() {
        let Some(request_data) = requests.get(scope, index) else {
            continue;
        };
        let Ok(request) = v8::Local::<v8::ModuleRequest>::try_from(request_data) else {
            return Err(Box::new(worker_bootstrap_error(
                scope,
                "worker module",
                "module request entry was not a ModuleRequest",
                WorkerParentErrorEventKind::Event,
            )));
        };
        let attributes = worker_module_request_attributes(scope, request);
        if let Some(invalid_key) = attributes.invalid_import_attribute_key() {
            return Err(Box::new(worker_bootstrap_error(
                scope,
                "worker module",
                &format!("Invalid attribute key \"{invalid_key}\"."),
                WorkerParentErrorEventKind::Event,
            )));
        }
        records.push(WorkerModuleRequest {
            specifier: request.get_specifier().to_rust_string_lossy(scope),
            attributes,
            phase: worker_module_import_phase(request.get_phase()),
        });
    }
    Ok(records)
}

fn worker_module_request_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::ModuleRequest>,
) -> ModuleAttributesKey {
    let attributes = request.get_import_attributes();
    let mut pairs = Vec::with_capacity(attributes.length() / 3);
    let mut index = 0;
    while index + 1 < attributes.length() {
        let key = attributes
            .get(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let value = attributes
            .get(scope, index + 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        if let (Some(key), Some(value)) = (key, value) {
            pairs.push((key, value));
        }
        index += 3;
    }
    ModuleAttributesKey::from_pairs(pairs)
}

fn worker_module_import_phase(phase: v8::ModuleImportPhase) -> ModuleImportPhase {
    match phase {
        v8::ModuleImportPhase::kSource => ModuleImportPhase::Source,
        _ => ModuleImportPhase::Evaluation,
    }
}

fn load_worker_static_module_dependency(
    base_url: &Url,
    specifier: &str,
) -> Result<WorkerModuleDependencyLoad, String> {
    let dependency_url = base_url
        .join(specifier)
        .or_else(|_| Url::parse(specifier))
        .map_err(|error| {
            format!("Failed to resolve module worker dependency `{specifier}`: {error}")
        })?;
    match dependency_url.scheme() {
        "data" => {
            let source = super::decode_data_url_script_source(
                &dependency_url,
                "Failed to load module worker dependency",
            )?;
            Ok(WorkerModuleDependencyLoad::Source {
                url: dependency_url,
                source: WorkerModuleSource::text(source),
            })
        }
        "http" | "https" => Ok(WorkerModuleDependencyLoad::NeedFetch(dependency_url)),
        scheme => Err(format!(
            "Module worker dependency scheme `{scheme}` is not allowed"
        )),
    }
}

fn worker_module_key_for_attributes(
    url: &Url,
    attributes: &ModuleAttributesKey,
) -> Result<WorkerModuleKey, String> {
    let Some(module_type) = attributes.module_type() else {
        if url.path().to_ascii_lowercase().ends_with(".wasm") {
            return Ok(WorkerModuleKey::webassembly(url.clone()));
        }
        return Ok(WorkerModuleKey::java_script(url.clone()));
    };
    match module_type {
        "json" => Ok(WorkerModuleKey::json(url.clone(), attributes.clone())),
        other => Err(format!("module type `{other}` is not a valid module type")),
    }
}

enum WorkerModuleDependencyLoad {
    Source {
        url: Url,
        source: WorkerModuleSource,
    },
    NeedFetch(Url),
}

struct WorkerModuleRuntimeFetchIdSlot {
    next_fetch_id: Rc<RefCell<WorkerModuleGraphFetchId>>,
}

struct WorkerModuleRuntimeEvaluationSlot {
    next_evaluation_id: Rc<RefCell<WorkerModuleEvaluationId>>,
    evaluation_completion_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
}

fn worker_bootstrap_error(
    scope: &mut v8::PinScope<'_, '_>,
    script_url: &str,
    summary: &str,
    event_kind: WorkerParentErrorEventKind,
) -> WorkerBootstrapError {
    let exception =
        v8::String::new(scope, summary).map(|message| v8::Exception::syntax_error(scope, message));
    (
        V8ExceptionReport {
            summary: summary.to_owned(),
            source: Some(script_url.to_owned()),
            line: Some(1),
            column: Some(1),
            source_line: None,
            stack: None,
            callback_context: None,
            exception: None,
        },
        exception.map(|value| v8::Global::new(scope, value)),
        event_kind,
    )
}

fn worker_bootstrap_value_error(
    scope: &mut v8::PinScope<'_, '_>,
    script_url: &str,
    value: v8::Local<'_, v8::Value>,
    event_kind: WorkerParentErrorEventKind,
) -> WorkerBootstrapError {
    let summary = value
        .to_string(scope)
        .map(|message| message.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "module worker evaluation failed".to_owned());
    (
        V8ExceptionReport {
            summary,
            source: Some(script_url.to_owned()),
            line: Some(1),
            column: Some(1),
            source_line: None,
            stack: None,
            callback_context: None,
            exception: None,
        },
        Some(v8::Global::new(scope, value)),
        event_kind,
    )
}

fn worker_resolve_static_module_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let graph = context.get_slot::<RefCell<WorkerModuleGraph>>()?;
    let specifier = specifier.to_rust_string_lossy(scope);
    let attributes = worker_import_attributes_key(scope, import_attributes);
    let graph = graph.borrow();
    let module = graph.resolve_static_dependency(referrer, &specifier, &attributes)?;
    Some(v8::Local::new(scope, module))
}

fn worker_resolve_static_source_callback<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::callback_scope!(unsafe scope, context);
    let graph = context.get_slot::<RefCell<WorkerModuleGraph>>()?;
    let specifier = specifier.to_rust_string_lossy(scope);
    let attributes = worker_import_attributes_key(scope, import_attributes);
    let graph = graph.borrow();
    let Some(record) = graph.resolve_static_dependency_record(referrer, &specifier, &attributes)
    else {
        return throw_worker_source_phase_syntax_error(
            scope,
            &format!("source-phase module `{specifier}` was not resolved"),
        );
    };
    let Some(wasm_record) = record.wasm_module.as_ref() else {
        return throw_worker_source_phase_syntax_error(
            scope,
            &format!("source-phase module `{specifier}` is not a WebAssembly module"),
        );
    };
    let Some(source) = wasm_record.source_module(scope) else {
        return throw_worker_source_phase_syntax_error(
            scope,
            &format!("failed to materialize WebAssembly source for `{specifier}`"),
        );
    };
    Some(source.into())
}

fn throw_worker_source_phase_syntax_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let message = v8_string(scope, message)?;
    let exception = v8::Exception::syntax_error(scope, message);
    scope.throw_exception(exception);
    None
}

fn worker_module_evaluation_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(evaluation_id) = worker_module_evaluation_reaction_id(scope, args.data()) else {
        return;
    };
    let context = scope.get_current_context();
    let Some(slot) = context.get_slot::<WorkerModuleRuntimeEvaluationSlot>() else {
        return;
    };
    let _ = slot
        .evaluation_completion_tx
        .send(WorkerModuleEvaluationCompletion::new(evaluation_id, Ok(())));
}

fn worker_module_evaluation_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(evaluation_id) = worker_module_evaluation_reaction_id(scope, args.data()) else {
        return;
    };
    let reason = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "unknown module worker top-level await rejection".to_owned());
    let context = scope.get_current_context();
    let Some(slot) = context.get_slot::<WorkerModuleRuntimeEvaluationSlot>() else {
        return;
    };
    let _ = slot
        .evaluation_completion_tx
        .send(WorkerModuleEvaluationCompletion::new(
            evaluation_id,
            Err(reason),
        ));
}

fn worker_module_evaluation_reaction_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<WorkerModuleEvaluationId> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    get_private_value(scope, data, WORKER_MODULE_EVALUATION_REACTION_ID_SLOT)?
        .number_value(scope)
        .map(|value| value as WorkerModuleEvaluationId)
}

pub(super) unsafe extern "C" fn worker_initialize_import_meta_object_callback(
    context: v8::Local<'_, v8::Context>,
    module: v8::Local<'_, v8::Module>,
    meta: v8::Local<'_, v8::Object>,
) {
    v8::callback_scope!(unsafe scope, context);
    let module_url = context
        .get_slot::<RefCell<WorkerModuleGraph>>()
        .and_then(|graph| graph.borrow().module_url_for(module))
        .or_else(|| worker_current_script_url(scope));
    let Some(module_url) = module_url else { return };
    let Some(value) = v8::String::new(scope, module_url.as_str()) else {
        return;
    };
    let _ = WorkerImportMetaDeclaration::new(value).initialize(scope, meta);
}

pub(crate) fn worker_wasm_instance_for_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    namespace: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let graph = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerModuleGraph>>()?;
    graph.borrow().wasm_instance_for_namespace(scope, namespace)
}

fn worker_import_meta_resolve_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let specifier = args.get(0);
    let Some(specifier) = specifier.to_string(scope) else {
        throw_worker_import_meta_resolve_type_error(scope, "Module specifier must be a string.");
        return;
    };
    let specifier = specifier.to_rust_string_lossy(scope);
    let Some(base_url) = v8::Local::<v8::String>::try_from(args.data())
        .ok()
        .and_then(|value| Url::parse(&value.to_rust_string_lossy(scope)).ok())
    else {
        throw_worker_import_meta_resolve_type_error(scope, "Module base URL is invalid.");
        return;
    };
    match base_url
        .join(&specifier)
        .or_else(|_| Url::parse(&specifier))
    {
        Ok(url) => {
            let Some(value) = v8::String::new(scope, url.as_str()) else {
                throw_worker_import_meta_resolve_type_error(
                    scope,
                    "Failed to allocate resolved module URL.",
                );
                return;
            };
            rv.set(value.into());
        }
        Err(error) => {
            throw_worker_import_meta_resolve_type_error(
                scope,
                &format!("Failed to resolve module specifier `{specifier}`: {error}"),
            );
        }
    }
}

fn throw_worker_import_meta_resolve_type_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let exception = v8::String::new(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    scope.throw_exception(exception);
}

fn worker_synthetic_module_evaluation_steps<'s>(
    context: v8::Local<'s, v8::Context>,
    module: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let synthetic = context
        .get_slot::<RefCell<WorkerModuleGraph>>()
        .and_then(|graph| graph.borrow().synthetic_module_record_for(module))?;
    match synthetic {
        WorkerSyntheticModuleRecord::Json(source) => {
            evaluate_worker_json_synthetic_module(scope, module, &source)
        }
        WorkerSyntheticModuleRecord::WebAssembly(wasm_record) => {
            evaluate_wasm_synthetic_module(scope, module, &wasm_record, |scope, import| {
                worker_wasm_import_value(scope, module, import)
            })
        }
    }
}

fn evaluate_worker_json_synthetic_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    source: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(json_source) = v8::String::new(scope, source) else {
        return throw_worker_synthetic_module_error(scope, "failed to allocate JSON module source");
    };
    let value = v8::json::parse(scope, json_source)?;
    set_worker_synthetic_default_export(scope, module, value)
}

fn set_worker_synthetic_default_export<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'s, v8::Module>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let export_name = v8::String::new(scope, "default")?;
    if module
        .set_synthetic_module_export(scope, export_name, value)
        .is_none_or(|ok| !ok)
    {
        return throw_worker_synthetic_module_error(
            scope,
            "failed to set worker synthetic module default export",
        );
    }
    Some(v8::undefined(scope).into())
}

fn worker_wasm_import_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    referrer: v8::Local<'s, v8::Module>,
    import: &WasmImportRecord,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(graph) = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerModuleGraph>>()
    else {
        return throw_wasm_link_error(scope, "worker wasm import graph is not available");
    };
    let attributes = ModuleAttributesKey::empty();
    let (dependency_module, dependency_wasm_record) = {
        let graph = graph.borrow();
        let Some(entry) =
            graph.resolve_static_dependency_entry(referrer, import.module(), &attributes)
        else {
            return throw_wasm_link_error(scope, "worker wasm import dependency is not available");
        };
        (
            graph.records[entry].module.clone(),
            graph.records[entry].wasm_module.clone(),
        )
    };
    let dependency = v8::Local::new(scope, &dependency_module);
    ensure_worker_dependency_module_namespace_ready(scope, &graph, dependency)?;
    wasm_dependency_export_value(
        scope,
        dependency,
        dependency_wasm_record.as_ref(),
        import.name(),
        "failed to allocate worker wasm import export name",
        "worker wasm import export is not available",
    )
}

fn ensure_worker_dependency_module_namespace_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    graph: &RefCell<WorkerModuleGraph>,
    module: v8::Local<'s, v8::Module>,
) -> Option<()> {
    let mut dependency_modules_for =
        |_: &mut v8::PinScope<'s, '_>, dependency: v8::Local<'s, v8::Module>| {
            let graph = graph.borrow();
            let entry = graph.entry_for_module(dependency)?;
            graph.records[entry]
                .requests
                .iter()
                .filter(|request| request.phase == ModuleImportPhase::Evaluation)
                .map(|request| {
                    graph
                        .resolve_static_dependency(
                            dependency,
                            &request.specifier,
                            &request.attributes,
                        )
                        .cloned()
                })
                .collect::<Option<Vec<_>>>()
        };
    ensure_wasm_dependency_module_namespace_ready(
        scope,
        module,
        |scope: &mut v8::PinScope<'s, '_>, module: v8::Local<'s, v8::Module>| match module
            .instantiate_module2(
                scope,
                worker_resolve_static_module_callback,
                worker_resolve_static_source_callback,
            ) {
            Some(true) => Some(()),
            Some(false) => {
                throw_wasm_synthetic_module_error(
                    scope,
                    "worker module dependency instantiate returned false",
                );
                None
            }
            None => {
                preserve_current_v8_module_exception(scope);
                None
            }
        },
        &mut dependency_modules_for,
        |scope| {
            scope.perform_microtask_checkpoint();
            crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
            Some(())
        },
        WasmDependencyModuleMessages {
            instantiating: "worker module dependency is still instantiating",
            already_failed: "worker module dependency already failed",
            evaluation_failed: "worker module dependency evaluation failed",
            not_instantiated: "worker module dependency is not instantiated",
            cyclic: "cyclic worker WebAssembly module evaluation through JavaScript dependencies is not supported yet",
            graph_unavailable: "worker module dependency graph is not available",
            pending: "worker module dependency evaluation is pending",
        },
    )
}

fn throw_worker_synthetic_module_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let exception = v8::String::new(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    scope.throw_exception(exception);
    None
}

fn worker_import_attributes_key(
    scope: &mut v8::PinScope<'_, '_>,
    attributes: v8::Local<'_, v8::FixedArray>,
) -> ModuleAttributesKey {
    let mut pairs = Vec::with_capacity(attributes.length() / 2);
    let mut index = 0;
    while index + 1 < attributes.length() {
        let key = attributes
            .get(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let value = attributes
            .get(scope, index + 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        if let (Some(key), Some(value)) = (key, value) {
            pairs.push((key, value));
        }
        index += 2;
    }
    ModuleAttributesKey::from_pairs(pairs)
}

pub(super) fn worker_dynamic_import_callback<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    _host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    queue_worker_dynamic_import(
        scope,
        resource_name,
        specifier,
        import_attributes,
        ModuleImportPhase::Evaluation,
    )
}

fn queue_worker_dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    phase: ModuleImportPhase,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let context = scope.get_current_context();
    let specifier = specifier.to_rust_string_lossy(scope);
    let attributes = worker_import_attributes_key(scope, import_attributes);
    if let Some(invalid_key) = attributes.invalid_import_attribute_key() {
        reject_worker_dynamic_import_resolver(
            scope,
            resolver,
            &format!("Invalid attribute key \"{invalid_key}\"."),
        );
        return Some(promise);
    }
    let resource_url = resource_name
        .to_string(scope)
        .and_then(|value| Url::parse(&value.to_rust_string_lossy(scope)).ok())
        .or_else(|| worker_current_script_url(scope));
    let audio_worklet_import =
        audio_worklet_dynamic_import(scope, resource_url.as_ref(), &specifier);
    if audio_worklet_import == AudioWorkletDynamicImport::Forbidden {
        reject_worker_dynamic_import_resolver(
            scope,
            resolver,
            "import() is disallowed on WorkletGlobalScope.",
        );
        return Some(promise);
    }
    let base_url = resource_url;
    let Some(base_url) = base_url else {
        reject_worker_dynamic_import_resolver(
            scope,
            resolver,
            "dynamic import in module worker has no base URL",
        );
        return Some(promise);
    };
    let fetch_initiator_url = worker_current_script_url(scope).unwrap_or_else(|| base_url.clone());
    let Some(dynamic_imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        reject_worker_dynamic_import_resolver(
            scope,
            resolver,
            "worker dynamic import resolver is not installed",
        );
        return Some(promise);
    };
    dynamic_imports.borrow_mut().queue_import(
        v8::Global::new(scope, context),
        v8::Global::new(scope, resolver),
        specifier,
        base_url,
        fetch_initiator_url,
        attributes,
        phase,
        audio_worklet_import == AudioWorkletDynamicImport::Bootstrap,
    );
    Some(promise)
}

pub(super) fn worker_dynamic_import_with_phase_callback<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    phase: v8::ModuleImportPhase,
    import_attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    match phase {
        v8::ModuleImportPhase::kEvaluation => worker_dynamic_import_callback(
            scope,
            host_defined_options,
            resource_name,
            specifier,
            import_attributes,
        ),
        v8::ModuleImportPhase::kSource => {
            if audio_worklet_dynamic_import(scope, None, "") == AudioWorkletDynamicImport::Forbidden
            {
                return reject_audio_worklet_dynamic_import(scope);
            }
            queue_worker_dynamic_import(
                scope,
                resource_name,
                specifier,
                import_attributes,
                ModuleImportPhase::Source,
            )
        }
        v8::ModuleImportPhase::kDefer => reject_unsupported_worker_dynamic_import_phase(
            scope,
            "defer-phase dynamic import is not supported yet",
        ),
    }
}

fn reject_worker_dynamic_import_resolver(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
) {
    let exception = v8::String::new(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, exception);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AudioWorkletDynamicImport {
    None,
    Bootstrap,
    Forbidden,
}

fn audio_worklet_dynamic_import(
    scope: &mut v8::PinScope<'_, '_>,
    resource_url: Option<&Url>,
    specifier: &str,
) -> AudioWorkletDynamicImport {
    let Some(bootstrap_url) = audio_worklet_bootstrap_module_url(scope) else {
        return AudioWorkletDynamicImport::None;
    };
    let Some(resource_url) = resource_url else {
        return AudioWorkletDynamicImport::Forbidden;
    };
    let is_bootstrap_import = worker_current_script_url(scope)
        .as_ref()
        .is_some_and(|current_url| resource_url == current_url)
        && resource_url.join(specifier).ok().as_ref() == Some(&bootstrap_url);
    if is_bootstrap_import {
        AudioWorkletDynamicImport::Bootstrap
    } else {
        AudioWorkletDynamicImport::Forbidden
    }
}

fn audio_worklet_bootstrap_module_url(scope: &mut v8::PinScope<'_, '_>) -> Option<Url> {
    let global = scope.get_current_context().global(scope);
    global
        .get(
            scope,
            v8str(scope, "__moliAudioWorkletBootstrapModuleUrl").into(),
        )
        .and_then(|value| value.to_string(scope))
        .and_then(|value| Url::parse(&value.to_rust_string_lossy(scope)).ok())
}

fn reject_audio_worklet_dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let exception = v8::String::new(scope, "import() is disallowed on WorkletGlobalScope.")
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, exception);
    Some(promise)
}

fn reject_unsupported_worker_dynamic_import_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let exception = v8::String::new(scope, message)
        .map(|message| v8::Exception::syntax_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, exception);
    Some(promise)
}

fn create_module_script_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: &str,
) -> v8::ScriptOrigin<'s> {
    let name = v8::String::new(scope, url).expect("v8 string allocation");
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        -1,
        None,
        false,
        false,
        true,
        None,
    )
}
