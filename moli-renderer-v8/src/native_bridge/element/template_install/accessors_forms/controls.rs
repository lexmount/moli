use super::super::super::*;
use crate::native_bridge::element::forms::{
    form_indexed_definer, form_indexed_deleter, form_indexed_descriptor, form_indexed_enumerator,
    form_indexed_getter, form_indexed_query, form_indexed_setter, form_named_definer,
    form_named_deleter, form_named_descriptor, form_named_getter, form_named_query,
};

pub(super) fn install_form_control_template_accessors<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
    installer: SpecializedTemplateInstaller,
) -> bool {
    match installer {
        SpecializedTemplateInstaller::HtmlFormElement => {
            install_form_accessors(scope, template);
            true
        }
        SpecializedTemplateInstaller::HtmlButtonElement
        | SpecializedTemplateInstaller::HtmlDataListElement
        | SpecializedTemplateInstaller::HtmlFieldSetElement
        | SpecializedTemplateInstaller::HtmlLegendElement
        | SpecializedTemplateInstaller::HtmlObjectElement
        | SpecializedTemplateInstaller::HtmlOutputElement
        | SpecializedTemplateInstaller::HtmlMeterElement
        | SpecializedTemplateInstaller::HtmlProgressElement
        | SpecializedTemplateInstaller::HtmlLabelElement
        | SpecializedTemplateInstaller::HtmlTextAreaElement => true,
        _ => false,
    }
}

fn install_form_accessors<'s, 'i>(
    _scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(form_indexed_getter)
            .setter(form_indexed_setter)
            .query(form_indexed_query)
            .descriptor(form_indexed_descriptor)
            .deleter(form_indexed_deleter)
            .enumerator(form_indexed_enumerator)
            .definer(form_indexed_definer),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(form_named_getter)
            .query(form_named_query)
            .descriptor(form_named_descriptor)
            .deleter(form_named_deleter)
            .definer(form_named_definer),
    );
}
