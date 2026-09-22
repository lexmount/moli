use super::*;

mod accessors_forms;
mod accessors_media;
mod accessors_resource;
mod accessors_structural;
mod instance;
mod methods;

use accessors_forms::install_form_template_accessors;
use accessors_media::install_media_template_accessors;
use accessors_resource::install_resource_template_accessors;
use accessors_structural::install_structural_template_accessors;
pub(in crate::native_bridge) use instance::install_specialized_instance_properties;
use methods::install_specialized_template_methods;

fn install_specialized_template_accessors<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
    installer: SpecializedTemplateInstaller,
) {
    for install in [
        install_media_template_accessors,
        install_form_template_accessors,
        install_resource_template_accessors,
        install_structural_template_accessors,
    ] {
        if install(scope, template, installer) {
            return;
        }
    }
}

pub(in crate::native_bridge) fn install_specialized_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
    installer: SpecializedTemplateInstaller,
) {
    install_specialized_template_accessors(scope, template, installer);
    install_specialized_template_methods(scope, template, installer);
}
