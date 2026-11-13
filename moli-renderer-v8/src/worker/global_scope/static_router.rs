//! Web IDL conversion and install-event lifetime for native static routing.

use super::*;
use crate::service_worker_runtime::static_router::{RouterCondition, RouterRule, RouterSource};
use crate::webidl::{self, Context, WebIdlConverter, WebIdlError};
use std::cell::Cell;

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "RequestMode", parse_with = parse_request_mode)]
struct RouteRequestMode(String);

fn parse_request_mode(value: &str) -> Option<RouteRequestMode> {
    crate::service_worker_runtime::static_router::REQUEST_MODES
        .contains(&value)
        .then(|| RouteRequestMode(value.to_owned()))
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "RequestDestination", parse_with = parse_request_destination)]
struct RouteRequestDestination(String);

fn parse_request_destination(value: &str) -> Option<RouteRequestDestination> {
    crate::service_worker_runtime::static_router::REQUEST_DESTINATIONS
        .contains(&value)
        .then(|| RouteRequestDestination(value.to_owned()))
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "RunningStatus", rename_all = "kebab-case")]
enum RunningStatus {
    Running,
    NotRunning,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "RouterCondition")]
struct ConditionInput {
    #[webidl(converter = "raw")]
    not: Option<BoxedCondition>,
    #[webidl(sequence, converter = "raw")]
    or: Option<Vec<Self>>,
    #[webidl(converter = "enum")]
    request_destination: Option<RouteRequestDestination>,
    #[webidl(converter = "raw")]
    request_method: Option<webidl::ByteString>,
    #[webidl(converter = "enum")]
    request_mode: Option<RouteRequestMode>,
    #[webidl(converter = "enum")]
    running_status: Option<RunningStatus>,
    #[webidl(converter = "raw")]
    url_pattern: Option<crate::context_bootstrap::RoutingPatternInput>,
}

struct BoxedCondition(Box<ConditionInput>);

thread_local! {
    static CONVERSION_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct ConversionDepth;

impl Drop for ConversionDepth {
    fn drop(&mut self) {
        CONVERSION_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

impl<'s> WebIdlConverter<'s> for ConditionInput {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        // Valid rules have at most ten levels. Keep invalid cyclic/deep Web IDL
        // input from exhausting the native stack before rule verification.
        if CONVERSION_DEPTH.with(|depth| depth.get() >= 256) {
            let message = v8str(scope, "RouterCondition conversion exceeded the stack limit");
            let error = v8::Exception::range_error(scope, message);
            scope.throw_exception(error);
            return Err(WebIdlError::pending_exception(context));
        }
        CONVERSION_DEPTH.with(|depth| depth.set(depth.get() + 1));
        let _depth = ConversionDepth;
        webidl::convert::<webidl::Dictionary<Self>>(scope, value, context).map(|value| value.0)
    }
}

impl<'s> WebIdlConverter<'s> for BoxedCondition {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        webidl::convert::<ConditionInput>(scope, value, context).map(|value| Self(Box::new(value)))
    }
}

impl ConditionInput {
    fn compile(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        base: &Url,
    ) -> Result<RouterCondition, String> {
        let url_pattern = self
            .url_pattern
            .map(|value| value.into_router_pattern(scope, base))
            .transpose()?;
        let request_method = self
            .request_method
            .map(|value| {
                let value = value.to_scalar_string();
                crate::service_worker_runtime::static_router::normalize_request_method(&value)
            })
            .transpose()?;
        let or = self
            .or
            .map(|children| {
                children
                    .into_iter()
                    .map(|child| child.compile(scope, base))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let not = self
            .not
            .map(|child| child.0.compile(scope, base).map(Box::new))
            .transpose()?;
        let condition = RouterCondition {
            url_pattern,
            request_method,
            request_mode: self.request_mode.map(|value| value.0),
            request_destination: self.request_destination.map(|value| value.0),
            running: self
                .running_status
                .map(|value| matches!(value, RunningStatus::Running)),
            or,
            not,
        };
        if !condition.verify() {
            return Err(
                "Router conditions must be nonempty and cannot combine or/not with other fields"
                    .to_owned(),
            );
        }
        Ok(condition)
    }
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "RouterSourceEnum", rename_all = "kebab-case")]
enum SourceToken {
    Network,
    FetchEvent,
    Cache,
    RaceNetworkAndFetchHandler,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RouterSourceDict")]
struct SourceDictionary {
    #[webidl(converter = "dom_string16")]
    cache_name: Option<Vec<u16>>,
}

struct SourceInput(RouterSource);

impl<'s> WebIdlConverter<'s> for SourceInput {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if value.is_object() || value.is_null_or_undefined() {
            let dictionary =
                webidl::convert::<webidl::Dictionary<SourceDictionary>>(scope, value, context)?.0;
            let name = dictionary.cache_name.ok_or_else(|| {
                WebIdlError::custom_message("Router cache source requires cacheName")
            })?;
            return Ok(Self(RouterSource::Cache { name: Some(name) }));
        }
        let token = webidl::convert::<webidl::EnumValue<SourceToken>>(scope, value, context)?.0;
        Ok(Self(match token {
            SourceToken::Network => RouterSource::Network,
            SourceToken::FetchEvent => RouterSource::FetchEvent,
            SourceToken::Cache => RouterSource::Cache { name: None },
            SourceToken::RaceNetworkAndFetchHandler => RouterSource::RaceNetworkAndFetchHandler,
        }))
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RouterRule")]
struct RuleInput {
    #[webidl(required, converter = "raw")]
    condition: ConditionInput,
    #[webidl(required, converter = "raw")]
    source: SourceInput,
}

struct RulesInput(Vec<RuleInput>);

impl<'s> WebIdlConverter<'s> for RulesInput {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if let Some(values) = webidl::convert_optional_sequence::<webidl::Dictionary<RuleInput>>(
            scope,
            value,
            context,
            &(),
        )? {
            return Ok(Self(values.0.into_iter().map(|value| value.0).collect()));
        }
        webidl::convert::<webidl::Dictionary<RuleInput>>(scope, value, context)
            .map(|value| Self(vec![value.0]))
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "InstallEvent.addRoutes")]
struct AddRoutesArgs {
    #[webidl(required, converter = "raw")]
    rules: RulesInput,
}

struct PendingRegistration {
    result: v8::Global<v8::PromiseResolver>,
    lifetime: v8::Global<v8::PromiseResolver>,
}

#[derive(Default)]
pub(in crate::worker) struct WorkerRouterState {
    next_request_id: u64,
    pending: HashMap<u64, PendingRegistration>,
}

pub(in crate::worker) fn add_routes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AddRoutesArgs>(scope, &args) else {
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let (base, runtime, version_id) = {
        let state = state.borrow();
        let WorkerGlobalKind::Service { version_id, .. } = state.global_kind else {
            return;
        };
        let Some(runtime) = state.service_worker_runtime.clone() else {
            return;
        };
        let Some(base) = state.current_script_url.clone() else {
            return;
        };
        (base, runtime, version_id)
    };
    let mut rules = Vec::with_capacity(parsed.rules.0.len());
    for rule in parsed.rules.0 {
        let condition = match rule.condition.compile(scope, &base) {
            Ok(condition) => condition,
            Err(message) => {
                throw_type_error(scope, &message);
                return;
            }
        };
        let source = rule.source.0;
        if matches!(
            source,
            RouterSource::FetchEvent | RouterSource::RaceNetworkAndFetchHandler
        ) && !runtime.static_router_has_fetch_handler(version_id)
        {
            throw_type_error(
                scope,
                "A fetch-event router source requires a registered fetch listener",
            );
            return;
        }
        rules.push(RouterRule { condition, source });
    }
    let Some(lifetime) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let lifetime_promise = lifetime.get_promise(scope);
    if !super::super::thread::add_service_worker_lifetime_promise(
        scope,
        args.this(),
        lifetime_promise,
    ) {
        return;
    }
    let Some(result) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let (request_id, parent) = {
        let mut state = state.borrow_mut();
        state.static_router.next_request_id = state
            .static_router
            .next_request_id
            .checked_add(1)
            .expect("static router request IDs exhausted");
        let request_id = state.static_router.next_request_id;
        state.static_router.pending.insert(
            request_id,
            PendingRegistration {
                result: v8::Global::new(scope, result),
                lifetime: v8::Global::new(scope, lifetime),
            },
        );
        (request_id, state.parent_tx.clone())
    };
    let data = v8::BigInt::new_from_u64(scope, request_id);
    let Some(settle_lifetime) = v8::Function::builder(registration_settled)
        .data(data.into())
        .build(scope)
    else {
        return;
    };
    let promise = result.get_promise(scope);
    let _ = promise.then2(scope, settle_lifetime, settle_lifetime);
    if parent
        .send(WorkerToParentMessage::ServiceWorkerRegisterRouterRules { request_id, rules })
        .is_err()
    {
        complete_registration(
            scope,
            &state,
            request_id,
            Err("Service Worker routing queue closed".to_owned()),
        );
    }
    rv.set(promise.into());
}

fn registration_settled(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(request_id) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let request_id = request_id.u64_value().0;
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let Some(pending) = state.borrow_mut().static_router.pending.remove(&request_id) else {
        return;
    };
    let lifetime = v8::Local::new(scope, pending.lifetime);
    let undefined = v8::undefined(scope);
    let _ = lifetime.resolve(scope, undefined.into());
}

pub(in crate::worker) fn complete_registration(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    request_id: u64,
    result: Result<(), String>,
) {
    let resolver = state
        .borrow()
        .static_router
        .pending
        .get(&request_id)
        .map(|pending| pending.result.clone());
    let Some(resolver) = resolver else {
        return;
    };
    let resolver = v8::Local::new(scope, resolver);
    match result {
        Ok(()) => {
            let undefined = v8::undefined(scope);
            let _ = resolver.resolve(scope, undefined.into());
        }
        Err(message) => {
            let message = v8_string(scope, &message).unwrap();
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}
