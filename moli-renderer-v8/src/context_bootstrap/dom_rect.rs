use super::*;
use crate::native_bridge::throw_dom_exception;
use crate::util::{callback_data_index_value, get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const DOM_RECT_X_SLOT: &str = "__moliDomRectX";
const DOM_RECT_Y_SLOT: &str = "__moliDomRectY";
const DOM_RECT_WIDTH_SLOT: &str = "__moliDomRectWidth";
const DOM_RECT_HEIGHT_SLOT: &str = "__moliDomRectHeight";
const DOM_RECT_BRAND_SLOT: &str = "__moliDomRectBrand";
const DOM_RECT_MUTABLE_BRAND_SLOT: &str = "__moliDomRectMutableBrand";
const DOM_RECT_RESTRICTED_NUMBER_SLOT: &str = "__moliDomRectRestrictedNumber";
const DOM_RECT_SVG_VIEW_BOX_OWNER_SLOT: &str = "__moliDomRectSvgViewBoxOwner";
const DOM_RECT_SVG_READ_ONLY_SLOT: &str = "__moliDomRectSvgReadOnly";

#[derive(WebApiObject)]
#[webapi(interface = "DOMRect")]
struct DomRectObjectDeclaration {
    #[webapi(slot = DOM_RECT_BRAND_SLOT, init = true)]
    brand: (),
    #[webapi(slot = DOM_RECT_MUTABLE_BRAND_SLOT, init = true)]
    mutable_brand: (),

    #[webapi(slot = DOM_RECT_X_SLOT)]
    x: f64,
    #[webapi(slot = DOM_RECT_Y_SLOT)]
    y: f64,
    #[webapi(slot = DOM_RECT_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = DOM_RECT_HEIGHT_SLOT)]
    height: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "DOMRectReadOnly")]
struct DomRectReadOnlyObjectDeclaration {
    #[webapi(slot = DOM_RECT_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = DOM_RECT_X_SLOT)]
    x: f64,
    #[webapi(slot = DOM_RECT_Y_SLOT)]
    y: f64,
    #[webapi(slot = DOM_RECT_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = DOM_RECT_HEIGHT_SLOT)]
    height: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMRectReadOnly")]
struct DomRectReadOnlyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    x: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    y: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_readonly_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    top: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_readonly_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    right: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_readonly_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    bottom: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_readonly_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    left: (),
    #[webapi(method = "toJSON", callback = dom_rect_to_json_callback, length = 0, enumerable)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMRect")]
struct DomRectPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        setter = dom_rect_setter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    x: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        setter = dom_rect_setter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    y: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        setter = dom_rect_setter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = dom_rect_writable_getter_callback,
        setter = dom_rect_setter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    height: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMRectReadOnly", enumerable)]
struct DomRectReadOnlyConstructorDeclaration {
    #[webapi(
        static_method = "fromRect",
        length = 0,
        callback = dom_rect_readonly_from_rect_callback
    )]
    from_rect: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMRect", enumerable)]
struct DomRectConstructorDeclaration {
    #[webapi(
        static_method = "fromRect",
        length = 0,
        callback = dom_rect_from_rect_callback
    )]
    from_rect: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct DomRectJsonDeclaration {
    #[webapi(data_property, enumerable)]
    x: f64,
    #[webapi(data_property, enumerable)]
    y: f64,
    #[webapi(data_property, enumerable)]
    width: f64,
    #[webapi(data_property, enumerable)]
    height: f64,
    #[webapi(data_property, enumerable)]
    top: f64,
    #[webapi(data_property, enumerable)]
    right: f64,
    #[webapi(data_property, enumerable)]
    bottom: f64,
    #[webapi(data_property, enumerable)]
    left: f64,
}

#[derive(Clone, Copy)]
enum DomRectReadonlyAttribute {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMRect")]
struct DomRectConstructorArgs {
    #[webidl(default = 0.0)]
    x: f64,
    #[webidl(default = 0.0)]
    y: f64,
    #[webidl(default = 0.0)]
    width: f64,
    #[webidl(default = 0.0)]
    height: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMRectReadOnly")]
struct DomRectReadOnlyConstructorArgs {
    #[webidl(default = 0.0)]
    x: f64,
    #[webidl(default = 0.0)]
    y: f64,
    #[webidl(default = 0.0)]
    width: f64,
    #[webidl(default = 0.0)]
    height: f64,
}

#[derive(Clone, Copy, Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "DOMRectInit")]
pub(super) struct DomRectInit {
    #[webidl(default = 0.0)]
    pub(super) x: f64,
    #[webidl(default = 0.0)]
    pub(super) y: f64,
    #[webidl(default = 0.0)]
    pub(super) width: f64,
    #[webidl(default = 0.0)]
    pub(super) height: f64,
}

pub(super) fn dom_rect_readonly_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMRectReadOnly': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<DomRectReadOnlyConstructorArgs>(scope, &args) else {
        return;
    };
    initialize_dom_rect_readonly_object(
        scope,
        args.this(),
        parsed.x,
        parsed.y,
        parsed.width,
        parsed.height,
    );
    rv.set(args.this().into());
}

pub(super) fn dom_rect_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMRect': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<DomRectConstructorArgs>(scope, &args) else {
        return;
    };
    initialize_dom_rect_object(
        scope,
        args.this(),
        parsed.x,
        parsed.y,
        parsed.width,
        parsed.height,
    );
    rv.set(args.this().into());
}

pub(crate) fn build_dom_rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> v8::Local<'s, v8::Object> {
    DomRectObjectDeclaration::new(x, y, width, height)
        .bind(scope)
        .expect("DOMRect declaration should bind")
}

pub(in crate::context_bootstrap) fn build_svg_rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let object = build_dom_rect_object(scope, 0.0, 0.0, 0.0, 0.0);
    set_private_value(
        scope,
        object,
        DOM_RECT_RESTRICTED_NUMBER_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    object
}

pub(in crate::context_bootstrap) fn build_svg_view_box_rect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    [x, y, width, height]: [f64; 4],
    read_only: bool,
) -> v8::Local<'s, v8::Object> {
    let object = build_dom_rect_object(scope, x, y, width, height);
    set_private_value(
        scope,
        object,
        DOM_RECT_RESTRICTED_NUMBER_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_value(
        scope,
        object,
        DOM_RECT_SVG_VIEW_BOX_OWNER_SLOT,
        owner.into(),
    );
    set_private_value(
        scope,
        object,
        DOM_RECT_SVG_READ_ONLY_SLOT,
        v8::Boolean::new(scope, read_only).into(),
    );
    object
}

pub(in crate::context_bootstrap) fn svg_view_box_rect_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, DOM_RECT_SVG_VIEW_BOX_OWNER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn svg_view_box_rect_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> [f64; 4] {
    [
        dom_rect_slot(object, scope, DOM_RECT_X_SLOT),
        dom_rect_slot(object, scope, DOM_RECT_Y_SLOT),
        dom_rect_slot(object, scope, DOM_RECT_WIDTH_SLOT),
        dom_rect_slot(object, scope, DOM_RECT_HEIGHT_SLOT),
    ]
}

pub(in crate::context_bootstrap) fn set_svg_view_box_rect_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    [x, y, width, height]: [f64; 4],
) {
    for (slot, value) in [
        (DOM_RECT_X_SLOT, x),
        (DOM_RECT_Y_SLOT, y),
        (DOM_RECT_WIDTH_SLOT, width),
        (DOM_RECT_HEIGHT_SLOT, height),
    ] {
        set_private_value(scope, object, slot, v8::Number::new(scope, value).into());
    }
}

fn build_dom_rect_readonly_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> v8::Local<'s, v8::Object> {
    DomRectReadOnlyObjectDeclaration::new(x, y, width, height)
        .bind(scope)
        .expect("DOMRectReadOnly declaration should bind")
}

fn initialize_dom_rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    DomRectObjectDeclaration::new(x, y, width, height)
        .bind_into(scope, object)
        .expect("DOMRect declaration should initialize object");
}

fn initialize_dom_rect_readonly_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    DomRectReadOnlyObjectDeclaration::new(x, y, width, height)
        .bind_into(scope, object)
        .expect("DOMRectReadOnly declaration should initialize object");
}

pub(in crate::context_bootstrap) fn install_dom_rect_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "DOMRectReadOnly" => {
            DomRectReadOnlyConstructorDeclaration::initialize_template(scope, template);
            DomRectReadOnlyPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "DOMRect" => {
            DomRectConstructorDeclaration::initialize_template(scope, template);
            DomRectPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

fn dom_rect_readonly_from_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(init) = dom_rect_init_arg(scope, &args, "DOMRectReadOnly.fromRect") else {
        return;
    };
    rv.set(build_dom_rect_readonly_object(scope, init.x, init.y, init.width, init.height).into());
}

fn dom_rect_from_rect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(init) = dom_rect_init_arg(scope, &args, "DOMRect.fromRect") else {
        return;
    };
    rv.set(build_dom_rect_object(scope, init.x, init.y, init.width, init.height).into());
}

pub(super) fn dom_rect_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> Option<DomRectInit> {
    if args.length() == 0 || args.get(0).is_undefined() {
        return Some(DomRectInit::default());
    }
    match webidl::parse_dictionary::<DomRectInit>(
        scope,
        args.get(0),
        webidl::Context::argument(prefix, 1),
    ) {
        Ok(Some(init)) => Some(init),
        Ok(None) => Some(DomRectInit::default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn dom_rect_writable_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        DOM_RECT_WRITABLE_ATTRIBUTE_SLOTS,
        "DOMRect writable attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    if !dom_rect_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    sync_svg_view_box_rect_if_attached(scope, args.this());
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn dom_rect_readonly_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        DOM_RECT_READONLY_ATTRIBUTES,
        "DOMRect readonly attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if !dom_rect_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    sync_svg_view_box_rect_if_attached(scope, args.this());
    let value = dom_rect_readonly_attribute_value(scope, args.this(), attribute);
    rv.set(v8::Number::new(scope, value).into());
}

fn dom_rect_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        DOM_RECT_WRITABLE_ATTRIBUTE_SLOTS,
        "DOMRect writable attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    if !dom_rect_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if get_private_value(scope, args.this(), DOM_RECT_SVG_READ_ONLY_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "The SVG rectangle is read-only.",
        );
        return;
    }
    sync_svg_view_box_rect_if_attached(scope, args.this());
    let context = webidl::Context::member("DOMRect", slot);
    let restricted = get_private_value(scope, args.this(), DOM_RECT_RESTRICTED_NUMBER_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let value = if restricted {
        match webidl::convert::<webidl::Double>(scope, args.get(0), context) {
            Ok(value) => value.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        }
    } else {
        match webidl::convert::<webidl::UnrestrictedDouble>(scope, args.get(0), context) {
            Ok(value) => value.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        }
    };
    set_private_value(
        scope,
        args.this(),
        slot,
        v8::Number::new(scope, value).into(),
    );
    if svg_view_box_rect_owner(scope, args.this()).is_some() {
        super::svg_runtime::reflect_svg_view_box_rect_mutation(scope, args.this());
    }
    rv.set_undefined();
}

fn sync_svg_view_box_rect_if_attached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if svg_view_box_rect_owner(scope, object).is_some() {
        super::svg_runtime::sync_svg_view_box_rect_from_owner(scope, object);
    }
}

fn dom_rect_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    if !dom_rect_receiver_branded(scope, this) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let declaration = DomRectJsonDeclaration {
        x: dom_rect_slot(this, scope, DOM_RECT_X_SLOT),
        y: dom_rect_slot(this, scope, DOM_RECT_Y_SLOT),
        width: dom_rect_slot(this, scope, DOM_RECT_WIDTH_SLOT),
        height: dom_rect_slot(this, scope, DOM_RECT_HEIGHT_SLOT),
        top: dom_rect_readonly_attribute_value(scope, this, DomRectReadonlyAttribute::Top),
        right: dom_rect_readonly_attribute_value(scope, this, DomRectReadonlyAttribute::Right),
        bottom: dom_rect_readonly_attribute_value(scope, this, DomRectReadonlyAttribute::Bottom),
        left: dom_rect_readonly_attribute_value(scope, this, DomRectReadonlyAttribute::Left),
    };
    let Ok(object) = declaration.bind(scope) else {
        return;
    };
    rv.set(object.into());
}

fn dom_rect_readonly_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: DomRectReadonlyAttribute,
) -> f64 {
    match attribute {
        DomRectReadonlyAttribute::Top => {
            let y = dom_rect_slot(object, scope, DOM_RECT_Y_SLOT);
            let height = dom_rect_slot(object, scope, DOM_RECT_HEIGHT_SLOT);
            dom_rect_min(y, y + height)
        }
        DomRectReadonlyAttribute::Right => {
            let x = dom_rect_slot(object, scope, DOM_RECT_X_SLOT);
            let width = dom_rect_slot(object, scope, DOM_RECT_WIDTH_SLOT);
            dom_rect_max(x, x + width)
        }
        DomRectReadonlyAttribute::Bottom => {
            let y = dom_rect_slot(object, scope, DOM_RECT_Y_SLOT);
            let height = dom_rect_slot(object, scope, DOM_RECT_HEIGHT_SLOT);
            dom_rect_max(y, y + height)
        }
        DomRectReadonlyAttribute::Left => {
            let x = dom_rect_slot(object, scope, DOM_RECT_X_SLOT);
            let width = dom_rect_slot(object, scope, DOM_RECT_WIDTH_SLOT);
            dom_rect_min(x, x + width)
        }
    }
}

fn dom_rect_min(lhs: f64, rhs: f64) -> f64 {
    if lhs.is_nan() || rhs.is_nan() {
        f64::NAN
    } else {
        lhs.min(rhs)
    }
}

fn dom_rect_max(lhs: f64, rhs: f64) -> f64 {
    if lhs.is_nan() || rhs.is_nan() {
        f64::NAN
    } else {
        lhs.max(rhs)
    }
}

fn dom_rect_slot<'s>(
    object: v8::Local<'s, v8::Object>,
    scope: &mut v8::PinScope<'s, '_>,
    key: &str,
) -> f64 {
    get_private_value(scope, object, key)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0)
}

fn dom_rect_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, DOM_RECT_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(super) fn dom_rect_clone_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(bool, [f64; 4])> {
    if !dom_rect_receiver_branded(scope, object) {
        return None;
    }
    let mutable = get_private_value(scope, object, DOM_RECT_MUTABLE_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    Some((
        mutable,
        [
            dom_rect_slot(object, scope, DOM_RECT_X_SLOT),
            dom_rect_slot(object, scope, DOM_RECT_Y_SLOT),
            dom_rect_slot(object, scope, DOM_RECT_WIDTH_SLOT),
            dom_rect_slot(object, scope, DOM_RECT_HEIGHT_SLOT),
        ],
    ))
}

pub(super) fn build_dom_rect_clone_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mutable: bool,
    [x, y, width, height]: [f64; 4],
) -> v8::Local<'s, v8::Object> {
    if mutable {
        build_dom_rect_object(scope, x, y, width, height)
    } else {
        build_dom_rect_readonly_object(scope, x, y, width, height)
    }
}

const DOM_RECT_WRITABLE_ATTRIBUTE_SLOTS: &[&str] = &[
    DOM_RECT_X_SLOT,
    DOM_RECT_Y_SLOT,
    DOM_RECT_WIDTH_SLOT,
    DOM_RECT_HEIGHT_SLOT,
];

const DOM_RECT_READONLY_ATTRIBUTES: &[DomRectReadonlyAttribute] = &[
    DomRectReadonlyAttribute::Top,
    DomRectReadonlyAttribute::Right,
    DomRectReadonlyAttribute::Bottom,
    DomRectReadonlyAttribute::Left,
];
