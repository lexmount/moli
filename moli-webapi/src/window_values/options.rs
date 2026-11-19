use moli_v8_util::throw_type_error;

use moli_webidl as webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "Window.requestIdleCallback")]
pub struct IdleRequestOptions {
    #[webidl(converter = "unsigned_long")]
    pub timeout: Option<u32>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "Window.postMessage")]
pub struct StructuredSerializeOptions<'s> {
    #[webidl(sequence, converter = "raw", default = Vec::new())]
    pub transfer: Vec<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "Window.postMessage")]
pub struct WindowPostMessageOptions<'s> {
    #[webidl(inherit)]
    pub serialization: StructuredSerializeOptions<'s>,
    #[webidl(name = "targetOrigin", converter = "usv_string", default = "/")]
    pub target_origin: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.postMessage")]
pub struct WindowPostMessageOptionsArgs<'s> {
    #[webidl(required, converter = "raw")]
    pub message: v8::Local<'s, v8::Value>,
    #[webidl(dictionary)]
    pub options: WindowPostMessageOptions<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ViewTransitionOptions")]
struct ViewTransitionOptionsMembers {
    #[webidl(sequence, legacy_nullish, default = Vec::new())]
    types: Vec<String>,
    #[webidl(nullable, converter = "callback_function")]
    update: Option<webidl::WebIdlCallbackFunction>,
}

pub struct StartViewTransitionOptions {
    pub callback: Option<webidl::WebIdlCallbackFunction>,
    pub types: Vec<String>,
}

pub fn parse_start_view_transition_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<StartViewTransitionOptions> {
    if args.length() == 0 || args.get(0).is_null_or_undefined() {
        return Some(StartViewTransitionOptions {
            callback: None,
            types: Vec::new(),
        });
    }

    let value = args.get(0);
    if let Ok(callback) = v8::Local::<v8::Object>::try_from(value)
        && callback.is_callable()
    {
        let callback = match webidl::convert::<webidl::WebIdlCallbackFunction>(
            scope,
            value,
            webidl::Context::argument("Document.startViewTransition", 1),
        ) {
            Ok(callback) => callback,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
        return Some(StartViewTransitionOptions {
            callback: Some(callback),
            types: Vec::new(),
        });
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to execute 'startViewTransition' on 'Document': Overload resolution failed.",
        );
        return None;
    };

    let options =
        match webidl::parse_dictionary_object::<ViewTransitionOptionsMembers>(scope, options) {
            Ok(options) => options,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
    Some(StartViewTransitionOptions {
        callback: options.update,
        types: options.types,
    })
}
