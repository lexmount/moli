//! Initializer dictionaries used when the embedder invokes an event constructor.

use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct ServiceWorkerExtendableMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "lastEventId", enumerable)]
    last_event_id: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}
