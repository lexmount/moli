//! GC-visible data shared by the two reactions of a transform finish operation.
//! These Rust views never outlive their HandleScope; the V8 callback owns the
//! private-slot carrier across promise jobs.

use moli_webapi_declare::WebApiObject;

use super::super::utils::{StreamOwnerPublication, publish_required_stream_value};
use crate::util::{get_private_object, private_key};

const WRITABLE_SLOT: &str = "__moliTransformFinishWritable";
const READABLE_SLOT: &str = "__moliTransformFinishReadable";
const RESIDENCE_SLOT: &str = "__moliTransformFinishResidence";
const REASON_SLOT: &str = "__moliTransformFinishReason";

#[derive(WebApiObject)]
#[webapi(plain)]
struct FinishDeclaration<'s> {
    #[webapi(slot = WRITABLE_SLOT)]
    writable: v8::Local<'s, v8::Object>,
    #[webapi(slot = READABLE_SLOT)]
    readable: v8::Local<'s, v8::Object>,
    #[webapi(slot = RESIDENCE_SLOT)]
    residence: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(fragment)]
struct ReasonDeclaration<'s> {
    #[webapi(slot = REASON_SLOT)]
    reason: v8::Local<'s, v8::Value>,
}

pub(super) struct TransformFinishContext<'s> {
    pub(super) writable: v8::Local<'s, v8::Object>,
    pub(super) readable: v8::Local<'s, v8::Object>,
    pub(super) residence: v8::Local<'s, v8::Object>,
}

impl<'s> TransformFinishContext<'s> {
    pub(super) fn into_callback_data(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> StreamOwnerPublication<v8::Local<'s, v8::Object>> {
        let data = FinishDeclaration::new(self.writable, self.readable, self.residence)
            .bind(scope)
            .ok();
        publish_required_stream_value(scope, data, "callback data creation", "transform finish")
    }

    fn decode(scope: &mut v8::PinScope<'s, '_>, data: v8::Local<'s, v8::Value>) -> Option<Self> {
        let data = v8::Local::<v8::Object>::try_from(data).ok()?;
        Some(Self {
            writable: get_private_object(scope, data, WRITABLE_SLOT)?,
            readable: get_private_object(scope, data, READABLE_SLOT)?,
            residence: get_private_object(scope, data, RESIDENCE_SLOT)?,
        })
    }

    pub(super) fn from_callback_data(
        scope: &mut v8::PinScope<'s, '_>,
        data: v8::Local<'s, v8::Value>,
    ) -> StreamOwnerPublication<Self> {
        let context = Self::decode(scope, data);
        publish_required_stream_value(scope, context, "callback data decoding", "transform finish")
    }
}

pub(super) struct TransformFinishWithReason<'s> {
    pub(super) finish: TransformFinishContext<'s>,
    pub(super) reason: v8::Local<'s, v8::Value>,
}

impl<'s> TransformFinishWithReason<'s> {
    pub(super) fn into_callback_data(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> StreamOwnerPublication<v8::Local<'s, v8::Object>> {
        let StreamOwnerPublication::Published(data) = self.finish.into_callback_data(scope) else {
            return StreamOwnerPublication::OwnerTerminating;
        };
        let data = ReasonDeclaration::new(self.reason)
            .initialize(scope, data)
            .ok()
            .map(|_| data);
        publish_required_stream_value(
            scope,
            data,
            "callback data creation",
            "transform finish reason",
        )
    }

    pub(super) fn from_callback_data(
        scope: &mut v8::PinScope<'s, '_>,
        data: v8::Local<'s, v8::Value>,
    ) -> StreamOwnerPublication<Self> {
        let context = (|| {
            let finish = TransformFinishContext::decode(scope, data)?;
            let data = v8::Local::<v8::Object>::try_from(data).ok()?;
            // get_private_value treats undefined as absent. A cancel/abort
            // reason may be undefined, but the slot itself must be present.
            let key = private_key(scope, REASON_SLOT)?;
            if data.has_private(scope, key) != Some(true) {
                return None;
            }
            let reason = data.get_private(scope, key)?;
            Some(Self { finish, reason })
        })();
        publish_required_stream_value(
            scope,
            context,
            "callback data decoding",
            "transform finish with reason",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish<'s>(scope: &mut v8::PinScope<'s, '_>) -> TransformFinishContext<'s> {
        TransformFinishContext {
            writable: v8::Object::new(scope),
            readable: v8::Object::new(scope),
            residence: v8::Object::new(scope),
        }
    }

    #[test]
    fn undefined_cancel_reason_survives_callback_data() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let context = TransformFinishWithReason {
            finish: finish(scope),
            reason: v8::undefined(scope).into(),
        };
        let StreamOwnerPublication::Published(data) = context.into_callback_data(scope) else {
            panic!("live context must publish callback data")
        };
        let StreamOwnerPublication::Published(context) =
            TransformFinishWithReason::from_callback_data(scope, data.into())
        else {
            panic!("undefined is a valid cancel reason")
        };
        assert!(context.reason.is_undefined());
    }

    #[test]
    #[should_panic(expected = "callback data decoding for `transform finish with reason`")]
    fn close_context_cannot_supply_a_missing_cancel_reason() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let StreamOwnerPublication::Published(data) = finish(scope).into_callback_data(scope)
        else {
            panic!("live context must publish callback data")
        };
        TransformFinishWithReason::from_callback_data(scope, data.into())
            .finish_at_owner_boundary();
    }
}
