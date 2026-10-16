use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FileReader, enumerable)]
struct FileReaderConstantsDeclaration {
    #[webapi(constant = "EMPTY", value = 0u32)]
    empty: (),
    #[webapi(constant = "LOADING", value = 1u32)]
    loading: (),
    #[webapi(constant = "DONE", value = 2u32)]
    done: (),
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
