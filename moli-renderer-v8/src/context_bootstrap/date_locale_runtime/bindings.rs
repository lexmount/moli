//! Fixed-shape plumbing shared by the Date and Intl wrappers. Only V8's
//! Proxy trap envelope and our private callback data are decoded here. The
//! page's argument values and options must still be interpreted by V8, not
//! eagerly converted as WebIDL strings, numbers, sequences or dictionaries.

use crate::util::v8str;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
pub(super) struct ConstructorApplyArgs<'s> {
    #[webidl(required)]
    pub target: v8::Local<'s, v8::Value>,
    #[webidl(required)]
    pub receiver: v8::Local<'s, v8::Value>,
    #[webidl(required)]
    pub arguments: v8::Local<'s, v8::Array>,
}

#[derive(webidl::WebIdlArgs)]
pub(super) struct ConstructorConstructArgs<'s> {
    #[webidl(required)]
    pub target: v8::Local<'s, v8::Value>,
    #[webidl(required)]
    pub arguments: v8::Local<'s, v8::Array>,
    #[webidl(required)]
    pub new_target: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlDictionary)]
pub(super) struct ReflectIntrinsics<'s> {
    #[webidl(required)]
    pub apply: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    pub construct: v8::Local<'s, v8::Function>,
}

impl<'s> ReflectIntrinsics<'s> {
    pub fn from_global(
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let reflect = global.get(scope, v8str(scope, "Reflect").into())?;
        let reflect = v8::Local::<v8::Object>::try_from(reflect).ok()?;
        webidl::parse_dictionary_object(scope, reflect).ok()
    }
}

pub(super) fn callback_data<'s, T: webidl::WebIdlDictionary<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<T> {
    // These objects are built by our declarations, retained only in native
    // callback data and never published to page JavaScript. Named own fields
    // replace positional arrays without introducing page-visible coercions.
    let object = v8::Local::<v8::Object>::try_from(data).ok()?;
    webidl::parse_dictionary_object(scope, object).ok()
}
