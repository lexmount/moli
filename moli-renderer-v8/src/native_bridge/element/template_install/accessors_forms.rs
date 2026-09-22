use super::super::*;

mod controls;
mod input;
mod select;

use self::controls::install_form_control_template_accessors;
use self::input::install_input_template_accessors;
use self::select::install_select_option_template_accessors;

pub(super) fn install_form_template_accessors<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
    installer: SpecializedTemplateInstaller,
) -> bool {
    for install in [
        install_input_template_accessors,
        install_form_control_template_accessors,
        install_select_option_template_accessors,
    ] {
        if install(scope, template, installer) {
            return true;
        }
    }
    false
}
