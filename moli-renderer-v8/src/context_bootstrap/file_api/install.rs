use super::*;
use anyhow::Result;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileReader", enumerable)]
struct FileReaderConstantsDeclaration {
    #[webapi(constant = "EMPTY", value = 0u32)]
    empty: (),
    #[webapi(constant = "LOADING", value = 1u32)]
    loading: (),
    #[webapi(constant = "DONE", value = 2u32)]
    done: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct FileApiRuntimeQueuesDeclaration {
    #[webapi(slot = FILE_READER_QUEUE_SLOT, init = "array")]
    file_reader_queue: (),
    #[webapi(slot = RESIZE_OBSERVER_QUEUE_SLOT, init = "array")]
    resize_observer_queue: (),
    #[webapi(slot = RESIZE_OBSERVER_REGISTRY_SLOT, init = "array")]
    resize_observer_registry: (),
}

pub(in crate::context_bootstrap) fn install_file_api_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    data_transfer::install_data_transfer_template_bindings(scope, template, interface_name);
    file::install_file_template_bindings(scope, template, interface_name);
    file_list::install_file_list_template_bindings(scope, template, interface_name);
    file_reader::install_file_reader_template_bindings(scope, template, interface_name);
    file_reader_sync::install_file_reader_sync_template_bindings(scope, template, interface_name);
    if interface_name == "FileReader" {
        let prototype = template.prototype_template(scope);
        FileReaderConstantsDeclaration::initialize_template(scope, template);
        FileReaderConstantsDeclaration::initialize_prototype_template(scope, prototype);
    }
}

pub(in crate::context_bootstrap) fn initialize_file_api_runtime_queues<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    FileApiRuntimeQueuesDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}
