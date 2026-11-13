mod module_map;
mod tree;

use module_map::{WorkerModuleMapEntry, WorkerModuleScript};
use moli_module_script_tree as module_tree;
use tree::{
    WorkerModuleTreeHost, WorkerModuleTreeTarget, WorkerModuleTreeTask, WorkerModuleTrees,
    WorkerSingleModuleCompletion,
};

use crate::module_runtime::import_attributes_key as worker_import_attributes_key;
use crate::module_runtime::module_request_attributes as worker_module_request_attributes;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::pin::pin;
use std::rc::Rc;

use crate::exception_reporting::{
    V8ExceptionReport, build_event_handler_exception_report, build_exception_report_without_stack,
};
use crate::module_runtime::{
    ModuleAttributesKey, ModuleImportPhase, WasmDependencyModuleMessages, WasmImportRecord,
    WasmModuleRecord, ensure_wasm_dependency_module_namespace_ready,
    evaluate_wasm_synthetic_module, preserve_current_v8_module_exception, throw_wasm_link_error,
    throw_wasm_synthetic_module_error, wasm_dependency_export_value,
};
use crate::util::{get_private_value, v8_string, v8str};
use crate::wasm_module_support::{prepare_wasm_module_record, wasm_evaluation_import_modules};
use moli_fetch::{BrowserRequestMetadata, RequestCredentialsMode};
use moli_webapi_declare::WebApiObject;
use tokio::sync::mpsc;
use url::Url;

use super::global_scope::worker_current_script_url;
use super::handle::{WorkerParentErrorEventKind, WorkerScriptResource, WorkerScriptResourceKind};
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
#[webapi(plain)]
struct WorkerModuleEvaluationReactionDataDeclaration {
    #[webapi(slot = WORKER_MODULE_EVALUATION_REACTION_ID_SLOT)]
    evaluation_id: f64,
}

#[derive(WebApiObject)]
#[webapi(plain)]
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
    key: WorkerModuleKey,
    fetch_id: WorkerModuleGraphFetchId,
    result: Result<WorkerModuleFetchedSource, module_tree::ModuleLoadError>,
    csp_violations: Vec<ContentSecurityPolicyUrlViolation>,
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
    result: Result<(), v8::Global<v8::Value>>,
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
}

impl From<String> for WorkerDynamicModuleImportError {
    fn from(message: String) -> Self {
        Self::type_error(message)
    }
}

impl From<Box<WorkerBootstrapError>> for WorkerDynamicModuleImportError {
    fn from(error: Box<WorkerBootstrapError>) -> Self {
        let (report, _, _) = *error;
        Self {
            message: report.summary,
            kind: WorkerDynamicModuleImportErrorKind::Type,
            // Bootstrap failures can contain a synthesized parent-facing
            // SyntaxError. Only a captured native exception supersedes the
            // dynamic import TypeError fallback.
            rejection: report.exception,
            stage: WorkerDynamicModuleImportErrorStage::Graph,
        }
    }
}

impl WorkerModuleGraphFetchCompletion {
    pub(super) fn new(
        fetch_id: WorkerModuleGraphFetchId,
        key: WorkerModuleKey,
        result: Result<WorkerModuleFetchedSource, String>,
    ) -> Self {
        let result = result.map_err(|message| {
            module_tree::ModuleLoadError::new(module_tree::ModuleLoadStage::Fetch, message)
                .with_key(tree::tree_key(&key))
        });
        Self {
            key,
            fetch_id,
            result,
            csp_violations: Vec::new(),
        }
    }

    pub(super) fn fetch_id(&self) -> WorkerModuleGraphFetchId {
        self.fetch_id
    }

    pub(super) fn with_csp_violations(
        mut self,
        violations: Vec<ContentSecurityPolicyUrlViolation>,
    ) -> Self {
        self.csp_violations = violations;
        self
    }

    pub(super) fn csp_violations(
        &self,
    ) -> impl Iterator<Item = &ContentSecurityPolicyUrlViolation> {
        self.csp_violations.iter()
    }

    pub(super) fn result(
        &self,
    ) -> Result<&WorkerModuleFetchedSource, &module_tree::ModuleLoadError> {
        self.result.as_ref()
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
    pub(super) fn from_response_parts(
        module_type: WorkerModuleType,
        url: &Url,
        headers: &[(String, Vec<u8>)],
        bytes: Vec<u8>,
    ) -> Result<Self, String> {
        let kind = module_type.response_kind(url, headers, &bytes)?;
        Ok(Self::from_bytes(kind, bytes))
    }

    pub(super) fn from_bytes(kind: WorkerScriptResourceKind, bytes: Vec<u8>) -> Self {
        if kind == WorkerScriptResourceKind::WebAssemblyModule {
            Self::binary(bytes)
        } else {
            Self::text(moli_encoding::decode_utf8(&bytes))
        }
    }

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
    fn new(
        evaluation_id: WorkerModuleEvaluationId,
        result: Result<(), v8::Global<v8::Value>>,
    ) -> Self {
        Self {
            evaluation_id,
            result,
        }
    }

    pub(super) fn evaluation_id(&self) -> WorkerModuleEvaluationId {
        self.evaluation_id
    }

    pub(super) fn result(&self) -> Result<(), &v8::Global<v8::Value>> {
        self.result.as_ref().map(|_| ())
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
    let root_url = match Url::parse(script_url) {
        Ok(url) => url,
        Err(_) => {
            return WorkerModuleBootstrapStart::Failed(Box::new(worker_bootstrap_error(
                scope,
                script_url,
                "Module worker script URL is invalid",
                WorkerParentErrorEventKind::Event,
            )));
        }
    };
    let runtime = WorkerModuleRuntime::new(
        credentials_mode,
        referrer_policy.clone(),
        evaluation_completion_tx,
    );
    runtime.install_context_slot(scope.get_current_context());
    let key = WorkerModuleKey::javascript_or_wasm(root_url.clone());
    let root_entry = create_worker_module_script(
        scope,
        &runtime.modules,
        &source,
        key.clone(),
        root_url.clone(),
        referrer_policy.clone(),
    );
    let root = module_tree::ModuleRootInput::Inline(module_tree::ModuleInlineRootInput {
        root_key: tree::tree_key(&key),
        root_entry: tree::entry_id(root_entry),
        source_url: root_url.clone(),
        base_url: root_url.clone(),
        phase: module_tree::ModuleImportPhase::Evaluation,
        fetch_metadata: tree::fetch_metadata(credentials_mode, referrer_policy.as_deref()),
        referrer: module_tree::ModuleReferrer::client(),
        position: module_tree::TextPosition::default(),
    });
    let task = WorkerModuleTreeTask {
        tree: module_tree::ModuleScriptTreeJob::new(
            root,
            module_tree::ModuleTreeConfig {
                tree_id: tree::BOOTSTRAP_TREE,
                owner: module_tree::ModuleTreeOwner {
                    kind: module_tree::ModuleTreeOwnerKind::WorkerModuleScript,
                    requester: module_tree::ModuleFetchRequester::RuntimeModuleScript,
                    ordering: module_tree::ModuleFetchOrdering::Runtime,
                },
                custom_fetch_type: module_tree::ModuleScriptCustomFetchType::WorkerConstructor,
                ..module_tree::ModuleTreeConfig::default()
            },
        ),
        target: WorkerModuleTreeTarget::Bootstrap,
        options: WorkerModuleGraphFetchOptions {
            initiator_url: static_import_initiator_url.unwrap_or(root_url),
            csp_source: WorkerModuleGraphFetchCspSource::StaticModuleGraph,
            browser_request_metadata: BrowserRequestMetadata::Fetch,
            referrer_policy,
        },
    };
    let mut bootstrap = WorkerModulePendingBootstrap {
        script_url: script_url.to_owned(),
        state: WorkerModulePendingBootstrapState::Tree(tree::BOOTSTRAP_TREE),
    };
    match drive_worker_module_tree(scope, &runtime.modules, &runtime.trees, task, None) {
        Some(terminal) => match bootstrap.resume_tree(scope, terminal.tree_id, terminal.result) {
            WorkerModuleBootstrapResume::Complete => WorkerModuleBootstrapStart::Complete,
            WorkerModuleBootstrapResume::Failed(error) => WorkerModuleBootstrapStart::Failed(error),
            WorkerModuleBootstrapResume::WaitingEvaluation => {
                WorkerModuleBootstrapStart::Pending(Box::new(bootstrap))
            }
        },
        None => WorkerModuleBootstrapStart::Pending(Box::new(bootstrap)),
    }
}

pub(super) enum WorkerModuleBootstrapStart {
    Complete,
    Pending(Box<WorkerModulePendingBootstrap>),
    Failed(Box<WorkerBootstrapError>),
}

pub(super) enum WorkerModuleBootstrapResume {
    Complete,
    WaitingEvaluation,
    Failed(Box<WorkerBootstrapError>),
}

pub(super) struct WorkerModulePendingBootstrap {
    script_url: String,
    state: WorkerModulePendingBootstrapState,
}

enum WorkerModulePendingBootstrapState {
    Tree(module_tree::ModuleTreeId),
    Evaluation {
        evaluation_id: WorkerModuleEvaluationId,
    },
}

impl WorkerModulePendingBootstrap {
    fn resume_tree(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        tree_id: module_tree::ModuleTreeId,
        result: Result<module_tree::ModuleGraphHandle, module_tree::ModuleLoadError>,
    ) -> WorkerModuleBootstrapResume {
        assert!(
            matches!(self.state, WorkerModulePendingBootstrapState::Tree(id) if id == tree_id),
            "module tree completion must match the bootstrap client"
        );
        let graph = match result {
            Ok(graph) => graph,
            Err(error) => {
                return WorkerModuleBootstrapResume::Failed(worker_bootstrap_tree_error(
                    scope,
                    &self.script_url,
                    error,
                ));
            }
        };
        match finish_worker_module_bootstrap(scope, &graph, &self.script_url) {
            Ok(WorkerModuleFinish::Complete) => WorkerModuleBootstrapResume::Complete,
            Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id }) => {
                self.state = WorkerModulePendingBootstrapState::Evaluation { evaluation_id };
                WorkerModuleBootstrapResume::WaitingEvaluation
            }
            Err(error) => WorkerModuleBootstrapResume::Failed(error),
        }
    }

    pub(super) fn resume_evaluation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: WorkerModuleEvaluationCompletion,
    ) -> WorkerModuleBootstrapResume {
        assert!(
            matches!(self.state, WorkerModulePendingBootstrapState::Evaluation { evaluation_id } if evaluation_id == completion.evaluation_id),
            "module evaluation completion must match its bootstrap owner"
        );
        match completion.result {
            Ok(()) => WorkerModuleBootstrapResume::Complete,
            Err(reason) => {
                let value = v8::Local::new(scope, reason);
                WorkerModuleBootstrapResume::Failed(Box::new(worker_bootstrap_value_error(
                    scope,
                    &self.script_url,
                    value,
                    WorkerParentErrorEventKind::ErrorEvent,
                )))
            }
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
    graph: &module_tree::ModuleGraphHandle,
    script_url: &str,
) -> WorkerModuleBootstrapResult<WorkerModuleFinish> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let scope = &mut scope;
    let modules = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerModuleMap>>()
        .expect("worker module map installed");
    let root_module_global = modules.borrow().module(graph.root_entry.0 as usize).clone();
    let root_module = v8::Local::new(scope, &root_module_global);

    // Include dependencies of synthetic Wasm modules, which V8's IsGraphAsync
    // cannot see, while excluding dependencies reached only by import source.
    // Reject before instantiation or evaluation can execute any module code.
    if super::global_scope::get_worker_state(scope).is_some_and(|state| {
        matches!(
            state.borrow().global_kind,
            super::thread::WorkerGlobalKind::Service { .. }
        )
    }) && graph.entries.iter().any(|entry| {
        graph.entry_phases.get(entry) == Some(&module_tree::ModuleImportPhase::Evaluation)
            && v8::Local::new(scope, modules.borrow().module(entry.0 as usize))
                .has_top_level_await()
    }) {
        let message = v8str(scope, "Top-level await is not allowed in service workers.");
        let exception = v8::Exception::type_error(scope, message);
        return Err(Box::new(worker_bootstrap_value_error(
            scope,
            script_url,
            exception,
            WorkerParentErrorEventKind::Event,
        )));
    }

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

    let Some(value) = crate::script_execution::evaluate_module(scope, root_module) else {
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
                let evaluation_id = reserve_worker_module_evaluation_id(scope)
                    .expect("worker evaluation slot installed");
                let promise = v8::Global::new(scope, promise);
                attach_worker_module_evaluation_reactions(scope, evaluation_id, promise)?;
                return Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id });
            }
        }
    }

    Ok(WorkerModuleFinish::Complete)
}

#[derive(Clone)]
struct WorkerModuleRuntime {
    modules: Rc<RefCell<WorkerModuleMap>>,
    trees: Rc<RefCell<WorkerModuleTrees>>,
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
            modules: Rc::new(RefCell::new(WorkerModuleMap::new(
                credentials_mode,
                referrer_policy,
            ))),
            trees: Rc::new(RefCell::new(WorkerModuleTrees::default())),
            next_evaluation_id: Rc::new(RefCell::new(0)),
            dynamic_imports: Rc::new(RefCell::new(WorkerDynamicModuleResolver::default())),
            evaluation_completion_tx,
        }
    }

    fn install_context_slot(&self, context: v8::Local<'_, v8::Context>) {
        let _previous_modules = context.set_slot(self.modules.clone());
        let _previous_trees = context.set_slot(self.trees.clone());
        let _previous_evaluation = context.set_slot(Rc::new(WorkerModuleRuntimeEvaluationSlot {
            next_evaluation_id: self.next_evaluation_id.clone(),
            evaluation_completion_tx: self.evaluation_completion_tx.clone(),
        }));
        let _previous_imports = context.set_slot(self.dynamic_imports.clone());
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

struct WorkerModuleTreeTerminal {
    tree_id: module_tree::ModuleTreeId,
    target: WorkerModuleTreeTarget,
    result: Result<module_tree::ModuleGraphHandle, module_tree::ModuleLoadError>,
}

fn drive_worker_module_tree(
    scope: &mut v8::PinScope<'_, '_>,
    modules: &Rc<RefCell<WorkerModuleMap>>,
    work: &Rc<RefCell<WorkerModuleTrees>>,
    mut task: WorkerModuleTreeTask,
    completion: Option<WorkerSingleModuleCompletion>,
) -> Option<WorkerModuleTreeTerminal> {
    let tree_id = task.tree.tree_id();
    let drive = {
        let mut host = WorkerModuleTreeHost {
            scope,
            modules: modules.clone(),
            work: work.clone(),
            options: &task.options,
        };
        match completion {
            Some(completion) => {
                let outcome = match completion.result {
                    Ok(entry) => tree::ready_module(&modules.borrow(), entry),
                    Err(error) => module_tree::ModuleFetchOutcome::Failed(error),
                };
                task.tree.resume_single_module_outcome_and_drive(
                    &mut host,
                    completion.client,
                    outcome,
                )
            }
            None => task.tree.drive(&mut host),
        }
    };
    let result = match drive {
        module_tree::ModuleScriptTreeDrive::Complete(graph) => Ok(graph),
        module_tree::ModuleScriptTreeDrive::Failed(error) => {
            modules.borrow_mut().detach_tree(tree_id);
            Err(error)
        }
        module_tree::ModuleScriptTreeDrive::Aborted(_) => {
            modules.borrow_mut().detach_tree(tree_id);
            Err(module_tree::ModuleLoadError::new(
                module_tree::ModuleLoadStage::Abort,
                "worker module tree was cancelled",
            ))
        }
        module_tree::ModuleScriptTreeDrive::Pending(_)
        | module_tree::ModuleScriptTreeDrive::NeedFetches(_)
        | module_tree::ModuleScriptTreeDrive::WaitingForSingleModuleClients(_)
        | module_tree::ModuleScriptTreeDrive::IgnoredStaleCompletion(_) => {
            work.borrow_mut().tasks.insert(tree_id, task);
            return None;
        }
    };
    Some(WorkerModuleTreeTerminal {
        tree_id,
        target: task.target,
        result,
    })
}

pub(super) fn run_next_worker_dynamic_module_import(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let context = scope.get_current_context();
    let Some(imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() else {
        return false;
    };
    let Some(job) = imports.borrow_mut().pending_imports.pop_front() else {
        return false;
    };
    let modules = context
        .get_slot::<RefCell<WorkerModuleMap>>()
        .expect("worker module map installed");
    let work = context
        .get_slot::<RefCell<WorkerModuleTrees>>()
        .expect("worker module tree runtime installed");
    let url = match resolve_worker_module_specifier(&job.specifier, &job.base_url) {
        Ok(url) => url,
        Err(message) => {
            reject_worker_dynamic_module_import(scope, job, message.into());
            return true;
        }
    };
    let key = match worker_module_key_for_attributes(&url, &job.attributes) {
        Ok(key) => key,
        Err(message) => {
            let error = WorkerDynamicModuleImportError::type_error(format!(
                "{message} for dynamic import `{}`",
                job.specifier,
            ));
            reject_worker_dynamic_module_import(scope, job, error);
            return true;
        }
    };
    let referrer_policy = modules.borrow().referrer_policy_for_url(&job.base_url);
    let metadata = tree::fetch_metadata(
        modules.borrow().credentials_mode,
        referrer_policy.as_deref(),
    );
    let root = module_tree::ModuleRootInput::External(module_tree::ModuleExternalRootInput {
        source_url: url.clone(),
        base_url: url,
        initiator_url: job.fetch_initiator_url.clone(),
        attributes: module_tree::ModuleAttributesKey::from_pairs(job.attributes.pairs().to_vec()),
        phase: tree::tree_phase(job.phase),
        kind_hint: Some(tree::tree_key(&key).kind),
        fetch_metadata: metadata,
        referrer: module_tree::ModuleReferrer::client(),
        position: module_tree::TextPosition::default(),
    });
    let options = WorkerModuleGraphFetchOptions {
        initiator_url: job.fetch_initiator_url.clone(),
        csp_source: WorkerModuleGraphFetchCspSource::DynamicImportGraph,
        browser_request_metadata: job.browser_request_metadata(),
        referrer_policy,
    };
    let task = WorkerModuleTreeTask {
        tree: module_tree::ModuleScriptTreeJob::new(
            root,
            module_tree::ModuleTreeConfig {
                tree_id: job.tree_id,
                owner: module_tree::ModuleTreeOwner::dynamic_import(),
                custom_fetch_type: if job.audio_worklet_bootstrap {
                    module_tree::ModuleScriptCustomFetchType::WorkletAddModule
                } else {
                    module_tree::ModuleScriptCustomFetchType::None
                },
                ..module_tree::ModuleTreeConfig::default()
            },
        ),
        target: WorkerModuleTreeTarget::Import(Box::new(job)),
        options,
    };
    if let Some(terminal) = drive_worker_module_tree(scope, &modules, &work, task, None) {
        let WorkerModuleTreeTarget::Import(job) = terminal.target else {
            unreachable!("started an import tree");
        };
        finish_worker_dynamic_module_tree(scope, *job, terminal.result);
    }
    true
}

pub(super) fn take_worker_module_graph_fetches(
    scope: &mut v8::PinScope<'_, '_>,
) -> Vec<WorkerModuleGraphFetchRequest> {
    let Some(work) = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerModuleTrees>>()
    else {
        return Vec::new();
    };
    std::mem::take(&mut work.borrow_mut().fetches_to_start)
}

pub(super) struct WorkerModuleGraphFetchOutcome {
    pub(super) bootstrap_client: bool,
}

pub(super) fn complete_worker_module_graph_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    completion: &WorkerModuleGraphFetchCompletion,
) -> Option<WorkerModuleGraphFetchOutcome> {
    let context = scope.get_current_context();
    let modules = context.get_slot::<RefCell<WorkerModuleMap>>()?;
    let work = context.get_slot::<RefCell<WorkerModuleTrees>>()?;
    let (request, clients) = modules
        .borrow_mut()
        .take_fetch(&completion.key, completion.fetch_id)?;
    let result = match &completion.result {
        Ok(source) => Ok(create_worker_module_script(
            scope,
            &modules,
            source.source(),
            request.key.clone(),
            source.final_url().clone(),
            source.effective_referrer_policy(request.referrer_policy()),
        )),
        Err(error) => Err(error.clone()),
    };
    let bootstrap_client = clients
        .iter()
        .any(|client| client.tree_id == tree::BOOTSTRAP_TREE);
    let mut work = work.borrow_mut();
    for client in clients {
        work.notifications.push_back(WorkerSingleModuleCompletion {
            client,
            result: result.clone(),
        });
    }
    Some(WorkerModuleGraphFetchOutcome { bootstrap_client })
}

pub(super) fn resume_worker_module_trees(
    scope: &mut v8::PinScope<'_, '_>,
    mut bootstrap: Option<&mut WorkerModulePendingBootstrap>,
) -> Option<WorkerModuleBootstrapResume> {
    let context = scope.get_current_context();
    let modules = context.get_slot::<RefCell<WorkerModuleMap>>()?;
    let work = context.get_slot::<RefCell<WorkerModuleTrees>>()?;
    let mut bootstrap_result = None;
    loop {
        let Some(completion) = work.borrow_mut().notifications.pop_front() else {
            break;
        };
        let task = work.borrow_mut().tasks.remove(&completion.client.tree_id);
        let Some(task) = task else {
            continue;
        };
        let Some(terminal) =
            drive_worker_module_tree(scope, &modules, &work, task, Some(completion))
        else {
            continue;
        };
        match terminal.target {
            WorkerModuleTreeTarget::Import(job) => {
                finish_worker_dynamic_module_tree(scope, *job, terminal.result)
            }
            WorkerModuleTreeTarget::Bootstrap => {
                if let Some(bootstrap) = bootstrap.as_deref_mut() {
                    bootstrap_result =
                        Some(bootstrap.resume_tree(scope, terminal.tree_id, terminal.result));
                }
            }
        }
    }
    bootstrap_result
}

fn worker_bootstrap_tree_error(
    scope: &mut v8::PinScope<'_, '_>,
    script_url: &str,
    error: module_tree::ModuleLoadError,
) -> Box<WorkerBootstrapError> {
    let modules = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerModuleMap>>()
        .expect("worker module map installed");
    if let Some(entry) = error.module_entry
        && let Err(stored) = &modules.borrow().script(entry.0 as usize).creation
    {
        return stored.clone();
    }
    Box::new(worker_bootstrap_error(
        scope,
        script_url,
        &error.message,
        WorkerParentErrorEventKind::Event,
    ))
}

fn worker_dynamic_tree_error(
    scope: &mut v8::PinScope<'_, '_>,
    error: module_tree::ModuleLoadError,
) -> WorkerDynamicModuleImportError {
    let modules = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerModuleMap>>()
        .expect("worker module map installed");
    if let Some(entry) = error.module_entry
        && let Err(stored) = &modules.borrow().script(entry.0 as usize).creation
    {
        return stored.clone().into();
    }
    if error.error_constructor == Some(module_tree::ModuleErrorConstructorKind::SyntaxError) {
        WorkerDynamicModuleImportError::syntax_error(error.message)
    } else {
        WorkerDynamicModuleImportError::type_error(error.message)
    }
}

fn finish_worker_dynamic_module_tree(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
    result: Result<module_tree::ModuleGraphHandle, module_tree::ModuleLoadError>,
) {
    let graph = match result {
        Ok(graph) => graph,
        Err(error) => {
            let error = worker_dynamic_tree_error(scope, error);
            reject_worker_dynamic_module_import(scope, job, error);
            return;
        }
    };
    let root_entry = graph.root_entry.0 as usize;
    if job.phase == ModuleImportPhase::Source {
        resolve_worker_dynamic_module_import(scope, job, root_entry);
        return;
    }
    match finish_worker_dynamic_module_import_evaluation(scope, root_entry) {
        Ok(WorkerModuleFinish::Complete) => {
            resolve_worker_dynamic_module_import(scope, job, root_entry)
        }
        Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id }) => {
            let imports = scope
                .get_current_context()
                .get_slot::<RefCell<WorkerDynamicModuleResolver>>()
                .expect("worker import resolver installed");
            imports.borrow_mut().evaluations.insert(
                evaluation_id,
                WorkerDynamicModuleEvaluation { job, root_entry },
            );
        }
        Err(error) => {
            if job.audio_worklet_bootstrap
                && error.stage == WorkerDynamicModuleImportErrorStage::Evaluate
            {
                resolve_worker_dynamic_module_import(scope, job, root_entry);
            } else {
                reject_worker_dynamic_module_import(scope, job, error);
            }
        }
    }
}

pub(super) fn worker_has_pending_dynamic_module_imports(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let context = scope.get_current_context();
    context
        .get_slot::<RefCell<WorkerDynamicModuleResolver>>()
        .is_some_and(|imports| {
            let imports = imports.borrow();
            !imports.pending_imports.is_empty() || !imports.evaluations.is_empty()
        })
        || context
            .get_slot::<RefCell<WorkerModuleTrees>>()
            .is_some_and(|work| {
                let work = work.borrow();
                !work.tasks.is_empty()
                    || !work.notifications.is_empty()
                    || !work.fetches_to_start.is_empty()
            })
        || context
            .get_slot::<RefCell<WorkerModuleMap>>()
            .is_some_and(|modules| modules.borrow().has_pending_fetches())
}

pub(super) fn worker_has_runnable_dynamic_module_imports(scope: &mut v8::PinScope<'_, '_>) -> bool {
    scope
        .get_current_context()
        .get_slot::<RefCell<WorkerDynamicModuleResolver>>()
        .is_some_and(|imports| !imports.borrow().pending_imports.is_empty())
}

pub(super) fn retire_worker_module_runtime(context: v8::Local<'_, v8::Context>) {
    if let Some(work) = context.get_slot::<RefCell<WorkerModuleTrees>>() {
        let mut work = work.borrow_mut();
        for (_, mut task) in work.tasks.drain() {
            task.tree
                .cancel(module_tree::ModuleTreeAbortReason::ContextDestroyed);
        }
        work.notifications.clear();
        work.fetches_to_start.clear();
    }
    if let Some(imports) = context.get_slot::<RefCell<WorkerDynamicModuleResolver>>() {
        let mut imports = imports.borrow_mut();
        imports.pending_imports.clear();
        imports.evaluations.clear();
    }
    if let Some(modules) = context.get_slot::<RefCell<WorkerModuleMap>>() {
        let mut modules = modules.borrow_mut();
        modules.entries.clear();
        modules.scripts.clear();
        modules.module_to_entries.clear();
        modules.pending_fetch_count = 0;
    }
}

pub(super) fn resume_worker_dynamic_module_evaluation(
    scope: &mut v8::PinScope<'_, '_>,
    completion: &WorkerModuleEvaluationCompletion,
) -> bool {
    let Some(imports) = scope
        .get_current_context()
        .get_slot::<RefCell<WorkerDynamicModuleResolver>>()
    else {
        return false;
    };
    let Some(evaluation) = imports
        .borrow_mut()
        .evaluations
        .remove(&completion.evaluation_id)
    else {
        return false;
    };
    match completion.result() {
        Ok(()) => {
            resolve_worker_dynamic_module_import(scope, evaluation.job, evaluation.root_entry)
        }
        Err(reason) => {
            if evaluation.job.audio_worklet_bootstrap {
                resolve_worker_dynamic_module_import(scope, evaluation.job, evaluation.root_entry);
            } else {
                let value = v8::Local::new(scope, reason);
                let error = WorkerDynamicModuleImportError::evaluation_rejection_value(
                    scope,
                    value,
                    "dynamic import module evaluation rejected",
                );
                reject_worker_dynamic_module_import(scope, evaluation.job, error);
            }
        }
    }
    true
}

fn finish_worker_dynamic_module_import_evaluation(
    scope: &mut v8::PinScope<'_, '_>,
    root_entry: usize,
) -> WorkerDynamicModuleImportResult<WorkerModuleFinish> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let scope = &mut scope;
    let context = scope.get_current_context();
    let graph = context
        .get_slot::<RefCell<WorkerModuleMap>>()
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
    } else if root_module.get_status() == v8::ModuleStatus::Errored {
        return Err(WorkerDynamicModuleImportError::evaluation_rejection_value(
            scope,
            root_module.get_exception(),
            "dynamic import module evaluation failed",
        ));
    }
    let Some(value) = crate::script_execution::evaluate_module(scope, root_module) else {
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
            v8::PromiseState::Fulfilled => Ok(WorkerModuleFinish::Complete),
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
                attach_worker_module_evaluation_reactions(scope, evaluation_id, promise)?;
                Ok(WorkerModuleFinish::PendingEvaluation { evaluation_id })
            }
        }
    } else {
        Ok(WorkerModuleFinish::Complete)
    }
}

fn resolve_worker_dynamic_module_import(
    scope: &mut v8::PinScope<'_, '_>,
    job: WorkerDynamicModuleImportJob,
    entry: usize,
) {
    let context = v8::Local::new(scope, &job.context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(graph) = context.get_slot::<RefCell<WorkerModuleMap>>() else {
        reject_worker_dynamic_module_import(
            scope,
            job,
            WorkerDynamicModuleImportError::type_error("worker module graph is not available"),
        );
        return;
    };
    let resolver = v8::Local::new(scope, &job.resolver);
    let resolved_value = {
        let graph = graph.borrow();
        let record = graph.record(entry);
        match job.phase {
            ModuleImportPhase::Evaluation => {
                let module = v8::Local::new(scope, &record.module);
                Ok(Some(module.get_module_namespace()))
            }
            ModuleImportPhase::Source => record
                .wasm_module
                .as_ref()
                .ok_or_else(|| {
                    WorkerDynamicModuleImportError::syntax_error(
                        "source-phase dynamic import did not resolve to a WebAssembly module",
                    )
                })
                .map(|wasm_record| {
                    wasm_record
                        .source_module(scope)
                        .map(v8::Local::<v8::Value>::from)
                }),
        }
    };
    let resolved_value = match resolved_value {
        Ok(value) => value,
        Err(error) => {
            reject_worker_dynamic_module_import(scope, job, error);
            return;
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

#[derive(Default)]
struct WorkerDynamicModuleResolver {
    next_import_id: u64,
    pending_imports: VecDeque<WorkerDynamicModuleImportJob>,
    evaluations: HashMap<WorkerModuleEvaluationId, WorkerDynamicModuleEvaluation>,
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
        self.next_import_id = self
            .next_import_id
            .checked_add(1)
            .expect("worker module tree id space exhausted");
        self.pending_imports
            .push_back(WorkerDynamicModuleImportJob {
                tree_id: module_tree::ModuleTreeId(self.next_import_id),
                context,
                resolver,
                specifier,
                base_url,
                fetch_initiator_url,
                attributes,
                phase,
                audio_worklet_bootstrap,
            });
    }
}

struct WorkerDynamicModuleImportJob {
    tree_id: module_tree::ModuleTreeId,
    context: v8::Global<v8::Context>,
    resolver: v8::Global<v8::PromiseResolver>,
    specifier: String,
    base_url: Url,
    fetch_initiator_url: Url,
    attributes: ModuleAttributesKey,
    phase: ModuleImportPhase,
    audio_worklet_bootstrap: bool,
}

impl WorkerDynamicModuleImportJob {
    fn browser_request_metadata(&self) -> BrowserRequestMetadata {
        if self.audio_worklet_bootstrap {
            BrowserRequestMetadata::AudioWorklet
        } else {
            BrowserRequestMetadata::Fetch
        }
    }
}

struct WorkerDynamicModuleEvaluation {
    job: WorkerDynamicModuleImportJob,
    root_entry: usize,
}

#[derive(Clone)]
struct WorkerModuleGraphFetchOptions {
    initiator_url: Url,
    csp_source: WorkerModuleGraphFetchCspSource,
    referrer_policy: Option<String>,
    browser_request_metadata: BrowserRequestMetadata,
}

#[derive(Clone)]
pub(super) struct WorkerModuleGraphFetchRequest {
    fetch_id: WorkerModuleGraphFetchId,
    key: WorkerModuleKey,
    initiator_url: Url,
    csp_source: WorkerModuleGraphFetchCspSource,
    credentials_mode: RequestCredentialsMode,
    referrer_policy: Option<String>,
    browser_request_metadata: BrowserRequestMetadata,
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
        credentials_mode: RequestCredentialsMode,
        referrer_policy: Option<String>,
        browser_request_metadata: BrowserRequestMetadata,
    ) -> Self {
        Self {
            fetch_id,
            key,
            initiator_url,
            csp_source,
            credentials_mode,
            referrer_policy,
            browser_request_metadata,
        }
    }

    pub(super) fn module_key(&self) -> WorkerModuleKey {
        self.key.clone()
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

    pub(super) fn module_type(&self) -> WorkerModuleType {
        self.key.module_type
    }

    pub(super) fn credentials_mode(&self) -> RequestCredentialsMode {
        self.credentials_mode
    }

    pub(super) fn referrer_policy(&self) -> Option<&str> {
        self.referrer_policy.as_deref()
    }

    pub(super) fn browser_request_metadata(&self) -> BrowserRequestMetadata {
        worker_module_browser_request_metadata(self.key.module_type, self.browser_request_metadata)
    }
}

fn worker_module_browser_request_metadata(
    module_type: WorkerModuleType,
    graph_metadata: BrowserRequestMetadata,
) -> BrowserRequestMetadata {
    match module_type {
        WorkerModuleType::Json => BrowserRequestMetadata::JsonModule,
        WorkerModuleType::Text => BrowserRequestMetadata::TextModule,
        WorkerModuleType::JavaScriptOrWebAssembly => graph_metadata,
    }
}

struct WorkerModuleMap {
    credentials_mode: RequestCredentialsMode,
    root_referrer_policy: Option<String>,
    entries: HashMap<WorkerModuleKey, WorkerModuleMapEntry>,
    scripts: Vec<WorkerModuleScript>,
    next_fetch_id: WorkerModuleGraphFetchId,
    pending_fetch_count: usize,
    module_to_entries: HashMap<i32, Vec<usize>>,
}

struct WorkerModuleRecord {
    source: WorkerModuleSource,
    json_value: Option<v8::Global<v8::Value>>,
    module: v8::Global<v8::Module>,
    requests: Vec<WorkerModuleRequest>,
    dependencies: Vec<WorkerModuleDependency>,
    wasm_module: Option<WasmModuleRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct WorkerModuleKey {
    url: Url,
    module_type: WorkerModuleType,
    attributes: ModuleAttributesKey,
}

impl WorkerModuleKey {
    fn javascript_or_wasm(url: Url) -> Self {
        Self {
            url,
            module_type: WorkerModuleType::JavaScriptOrWebAssembly,
            attributes: ModuleAttributesKey::empty(),
        }
    }

    fn json(url: Url, attributes: ModuleAttributesKey) -> Self {
        Self {
            url,
            module_type: WorkerModuleType::Json,
            attributes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum WorkerModuleType {
    // HTML keys the module map by the requested type. JavaScript and Wasm
    // share this entry; the fetched MIME type selects the compiled record.
    JavaScriptOrWebAssembly,
    Json,
    Text,
}

impl WorkerModuleType {
    pub(super) fn csp_resource_kind(
        self,
        script_kind: crate::content_security_policy::ContentSecurityPolicyResourceKind,
    ) -> crate::content_security_policy::ContentSecurityPolicyResourceKind {
        let kind = match self {
            Self::JavaScriptOrWebAssembly => crate::module_runtime::ModuleKind::JavaScript,
            Self::Json => crate::module_runtime::ModuleKind::Json,
            Self::Text => crate::module_runtime::ModuleKind::Text,
        };
        crate::content_security_policy::module_csp_resource_kind(kind, script_kind)
    }

    pub(super) fn response_kind(
        self,
        url: &Url,
        headers: &[(String, Vec<u8>)],
        bytes: &[u8],
    ) -> Result<WorkerScriptResourceKind, String> {
        match self {
            Self::Json => {
                super::module_mime::ensure_worker_json_module_mime_from_headers(headers)?;
                Ok(WorkerScriptResourceKind::JsonModule)
            }
            Self::Text => Ok(WorkerScriptResourceKind::TextModule),
            Self::JavaScriptOrWebAssembly => {
                if super::worker_response_has_webassembly_mime(headers) {
                    return Ok(WorkerScriptResourceKind::WebAssemblyModule);
                }
                super::ensure_worker_script_mime_acceptable(url, headers, bytes).map_err(
                    |error| {
                        error.replace(
                            "unsupported script MIME type",
                            "unsupported module script MIME type",
                        )
                    },
                )?;
                Ok(WorkerScriptResourceKind::JavaScript)
            }
        }
    }
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
    Json(v8::Global<v8::Value>),
    Text(String),
    WebAssembly(WasmModuleRecord),
}

impl WorkerModuleMap {
    fn new(credentials_mode: RequestCredentialsMode, root_referrer_policy: Option<String>) -> Self {
        Self {
            credentials_mode,
            root_referrer_policy,
            entries: HashMap::new(),
            scripts: Vec::new(),
            next_fetch_id: 0,
            pending_fetch_count: 0,
            module_to_entries: HashMap::new(),
        }
    }

    fn referrer_policy_for_url(&self, url: &Url) -> Option<String> {
        self.scripts
            .iter()
            .filter(|script| script.creation.is_ok())
            .find(|script| script.key.url == *url || script.response_url == *url)
            .and_then(|script| script.referrer_policy.clone())
            .or_else(|| self.root_referrer_policy.clone())
    }

    fn module(&self, entry: usize) -> &v8::Global<v8::Module> {
        &self.record(entry).module
    }

    fn add_dependency(
        &mut self,
        entry: usize,
        specifier: String,
        attributes: ModuleAttributesKey,
        target_entry: usize,
    ) {
        self.record_mut(entry)
            .dependencies
            .push(WorkerModuleDependency {
                specifier,
                attributes,
                target_entry,
            });
    }

    fn dependency_entry(
        &self,
        entry: usize,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<usize> {
        self.record(entry)
            .dependencies
            .iter()
            .find(|dependency| {
                dependency.specifier == specifier && dependency.attributes == *attributes
            })
            .map(|dependency| dependency.target_entry)
    }

    fn module_url_for(&self, module: v8::Local<'_, v8::Module>) -> Option<Url> {
        let entry = self.entry_for_module(module)?;
        Some(self.script(entry).response_url.clone())
    }

    fn wasm_instance_for_namespace<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        namespace: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        for record in self
            .scripts
            .iter()
            .filter_map(|script| script.creation.as_ref().ok())
        {
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
        let record = self.record(entry);
        if let Some(wasm) = &record.wasm_module {
            return Some(WorkerSyntheticModuleRecord::WebAssembly(wasm.clone()));
        }
        match self.script(entry).key.module_type {
            WorkerModuleType::Json => record
                .json_value
                .clone()
                .map(WorkerSyntheticModuleRecord::Json),
            WorkerModuleType::Text => record
                .source
                .text_source()
                .map(|source| WorkerSyntheticModuleRecord::Text(source.to_owned())),
            WorkerModuleType::JavaScriptOrWebAssembly => None,
        }
    }

    fn resolve_static_dependency_entry(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<usize> {
        let referrer_entry = self.entry_for_module(referrer)?;
        self.dependency_entry(referrer_entry, specifier, attributes)
    }

    fn resolve_static_dependency_record(
        &self,
        referrer: v8::Local<'_, v8::Module>,
        specifier: &str,
        attributes: &ModuleAttributesKey,
    ) -> Option<&WorkerModuleRecord> {
        let dependency_entry =
            self.resolve_static_dependency_entry(referrer, specifier, attributes)?;
        Some(self.record(dependency_entry))
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
            .find(|entry| module == self.record(*entry).module)
    }
}

type WorkerModuleBootstrapResult<T> = Result<T, Box<WorkerBootstrapError>>;

fn create_worker_module_script(
    scope: &mut v8::PinScope<'_, '_>,
    modules: &Rc<RefCell<WorkerModuleMap>>,
    source: &WorkerModuleSource,
    key: WorkerModuleKey,
    response_url: Url,
    referrer_policy: Option<String>,
) -> usize {
    if let Some(entry) = modules.borrow().loaded_entry(&key) {
        return entry;
    }
    let compiled = compile_worker_module_record(scope, source, &key, &response_url);
    let (creation, identity_hash) = match compiled {
        Ok((module, identity_hash, requests, wasm_module, json_value)) => (
            Ok(WorkerModuleRecord {
                source: source.clone(),
                json_value,
                module,
                requests,
                dependencies: Vec::new(),
                wasm_module,
            }),
            Some(identity_hash),
        ),
        Err(error) => (Err(error), None),
    };
    let mut modules = modules.borrow_mut();
    let entry = modules.insert_script(WorkerModuleScript {
        key,
        response_url,
        referrer_policy,
        creation,
    });
    if let Some(identity_hash) = identity_hash {
        modules
            .module_to_entries
            .entry(identity_hash)
            .or_default()
            .push(entry);
    }
    entry
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
    Option<v8::Global<v8::Value>>,
)> {
    if matches!(
        key.module_type,
        WorkerModuleType::Json | WorkerModuleType::Text
    ) {
        let json_source = if key.module_type == WorkerModuleType::Json {
            Some(source.text_source().ok_or_else(|| {
                Box::new(worker_bootstrap_error(
                    scope,
                    source_url.as_str(),
                    "JSON module worker source is not text",
                    WorkerParentErrorEventKind::Event,
                ))
            })?)
        } else {
            None
        };
        return compile_worker_synthetic_module_record(scope, json_source, source_url);
    }
    if let Some(bytes) = source.binary_source() {
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
        None,
    ))
}

fn compile_worker_synthetic_module_record(
    scope: &mut v8::PinScope<'_, '_>,
    source: Option<&str>,
    source_url: &Url,
) -> WorkerModuleBootstrapResult<(
    v8::Global<v8::Module>,
    i32,
    Vec<WorkerModuleRequest>,
    Option<WasmModuleRecord>,
    Option<v8::Global<v8::Value>>,
)> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let value = if let Some(source) = source {
        let Some(value) =
            crate::module_runtime::parse_json_module(&mut scope, source, source_url.as_str())
        else {
            let exception = scope.exception();
            let message = scope.message();
            let report = build_exception_report_without_stack(&mut scope, exception, message);
            return Err(Box::new((
                report,
                exception.map(|value| v8::Global::new(&scope, value)),
                WorkerParentErrorEventKind::Event,
            )));
        };
        Some(v8::Global::new(&scope, value))
    } else {
        None
    };
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
        value,
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
    Option<v8::Global<v8::Value>>,
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
            let exception = scope.exception();
            let message = scope.message();
            let mut report = build_exception_report_without_stack(&mut scope, exception, message);
            report.summary = format!(
                "v8 failed to compile WebAssembly module `{source_url}`: {}",
                report.summary
            );
            report.source.get_or_insert_with(|| source_url.to_string());
            Box::new((
                report,
                exception.map(|value| v8::Global::new(&scope, value)),
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
        None,
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

fn worker_module_import_phase(phase: v8::ModuleImportPhase) -> ModuleImportPhase {
    match phase {
        v8::ModuleImportPhase::kSource => ModuleImportPhase::Source,
        _ => ModuleImportPhase::Evaluation,
    }
}

fn resolve_worker_module_specifier(specifier: &str, base_url: &Url) -> Result<Url, String> {
    moli_import_map::resolve_url_like_module_specifier(specifier, base_url).ok_or_else(|| {
        format!("Failed to resolve module specifier `{specifier}` from `{base_url}`")
    })
}

fn load_worker_static_module_dependency(
    dependency_url: Url,
    module_type: WorkerModuleType,
) -> Result<WorkerModuleDependencyLoad, String> {
    match dependency_url.scheme() {
        "data" => {
            let (bytes, mime) = moli_web_mime::data_url_body_and_mime_type(dependency_url.as_str())
                .ok_or_else(|| format!("Failed to load module worker dependency: invalid data URL `{dependency_url}`"))?;
            let source = WorkerModuleSource::from_response_parts(
                module_type,
                &dependency_url,
                &[("content-type".to_owned(), mime.into_bytes())],
                bytes,
            )?;
            Ok(WorkerModuleDependencyLoad::Source {
                url: dependency_url,
                source,
            })
        }
        "http" | "https" => Ok(WorkerModuleDependencyLoad::Network),
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
        return Ok(WorkerModuleKey::javascript_or_wasm(url.clone()));
    };
    match module_type {
        "json" => Ok(WorkerModuleKey::json(url.clone(), attributes.clone())),
        "text" => Ok(WorkerModuleKey {
            url: url.clone(),
            module_type: WorkerModuleType::Text,
            attributes: attributes.clone(),
        }),
        other => Err(format!("module type `{other}` is not a valid module type")),
    }
}

enum WorkerModuleDependencyLoad {
    Source {
        url: Url,
        source: WorkerModuleSource,
    },
    Network,
}

struct WorkerModuleRuntimeEvaluationSlot {
    next_evaluation_id: Rc<RefCell<WorkerModuleEvaluationId>>,
    evaluation_completion_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
}

pub(super) fn worker_bootstrap_error(
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
    let graph = context.get_slot::<RefCell<WorkerModuleMap>>()?;
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
    let graph = context.get_slot::<RefCell<WorkerModuleMap>>()?;
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
    let reason = v8::Global::new(scope, args.get(0));
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
        .get_slot::<RefCell<WorkerModuleMap>>()
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
        .get_slot::<RefCell<WorkerModuleMap>>()?;
    graph.borrow().wasm_instance_for_namespace(scope, namespace)
}

fn worker_import_meta_resolve_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let specifier = args.get(0);
    let Some(specifier) = specifier.to_string(scope) else {
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
    match resolve_worker_module_specifier(&specifier, &base_url) {
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
        Err(message) => {
            throw_worker_import_meta_resolve_type_error(scope, &message);
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
        .get_slot::<RefCell<WorkerModuleMap>>()
        .and_then(|graph| graph.borrow().synthetic_module_record_for(module))?;
    match synthetic {
        WorkerSyntheticModuleRecord::Text(source) => {
            let value = v8_string(scope, &source)?;
            set_worker_synthetic_default_export(scope, module, value.into())
        }
        WorkerSyntheticModuleRecord::Json(value) => {
            let value = v8::Local::new(scope, value);
            set_worker_synthetic_default_export(scope, module, value)
        }
        WorkerSyntheticModuleRecord::WebAssembly(wasm_record) => {
            evaluate_wasm_synthetic_module(scope, module, &wasm_record, |scope, import| {
                worker_wasm_import_value(scope, module, import)
            })
        }
    }
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
        .get_slot::<RefCell<WorkerModuleMap>>()
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
            graph.record(entry).module.clone(),
            graph.record(entry).wasm_module.clone(),
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
    graph: &RefCell<WorkerModuleMap>,
    module: v8::Local<'s, v8::Module>,
) -> Option<()> {
    let mut dependency_modules_for =
        |_: &mut v8::PinScope<'s, '_>, dependency: v8::Local<'s, v8::Module>| {
            let graph = graph.borrow();
            let entry = graph.entry_for_module(dependency)?;
            graph
                .record(entry)
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
    // HostLoadImportedModule forbids dynamic imports in service workers,
    // including modules already present in the static graph. Reject before
    // resolving the specifier or scheduling any graph fetch.
    if super::global_scope::get_worker_state(scope).is_some_and(|state| {
        matches!(
            state.borrow().global_kind,
            super::thread::WorkerGlobalKind::Service { .. }
        )
    }) {
        reject_worker_dynamic_import_resolver(
            scope,
            resolver,
            "import() is disallowed on ServiceWorkerGlobalScope.",
        );
        return Some(promise);
    }
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
