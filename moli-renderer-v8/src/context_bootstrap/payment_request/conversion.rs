use crate::{
    util::{throw_type_error, v8_string},
    webidl,
};
use moli_webapi_declare::{WebApiObject, WebApiValue};
use std::collections::HashSet;

#[derive(Clone)]
pub(super) struct Text(pub(super) webidl::DomString16);

impl Text {
    pub(super) fn from_string(value: &str) -> Self {
        Self(webidl::DomString16(value.encode_utf16().collect()))
    }

    fn string(&self) -> String {
        String::from_utf16_lossy(&self.0.0)
    }
}

impl<'s> webidl::WebIdlConverter<'s> for Text {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::DomString16>(scope, value, context).map(Self)
    }
}

impl<'s> WebApiValue<'s> for Text {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_two_byte(scope, &self.0.0, v8::NewStringType::Normal).map(Into::into)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "PaymentShippingType", rename_all = "kebab-case")]
pub(super) enum ShippingType {
    Shipping,
    Delivery,
    Pickup,
}

impl<'s> WebApiValue<'s> for ShippingType {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Shipping => "shipping",
            Self::Delivery => "delivery",
            Self::Pickup => "pickup",
        }
        .to_v8_value(scope)
    }
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentMethodData")]
#[webapi(plain, enumerable)]
pub(super) struct MethodData<'s> {
    #[webidl(converter = "raw")]
    #[webapi(data_property)]
    pub(super) data: Option<v8::Local<'s, v8::Object>>,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    pub(super) supported_methods: Text,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentCurrencyAmount")]
#[webapi(plain, enumerable)]
pub(super) struct Amount {
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    currency: Text,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    value: Text,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentItem")]
#[webapi(plain, enumerable)]
pub(super) struct Item {
    #[webidl(required, dictionary)]
    #[webapi(data_property)]
    amount: Amount,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    label: Text,
    #[webidl(default = false)]
    #[webapi(data_property)]
    pending: bool,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentShippingOption")]
#[webapi(plain, enumerable)]
pub(super) struct ShippingOption {
    #[webidl(required, dictionary)]
    #[webapi(data_property)]
    amount: Amount,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    id: Text,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    label: Text,
    #[webidl(default = false)]
    #[webapi(data_property)]
    selected: bool,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentDetailsModifier")]
#[webapi(plain, enumerable)]
pub(super) struct Modifier<'s> {
    #[webidl(sequence, converter = "dictionary")]
    #[webapi(data_property)]
    additional_display_items: Option<Vec<Item>>,
    #[webidl(converter = "raw")]
    #[webapi(data_property)]
    data: Option<v8::Local<'s, v8::Object>>,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    supported_methods: Text,
    #[webidl(dictionary)]
    #[webapi(data_property)]
    total: Option<Item>,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentDetailsBase")]
#[webapi(plain, enumerable)]
pub(super) struct DetailsBase<'s> {
    #[webidl(sequence, converter = "dictionary")]
    #[webapi(data_property)]
    display_items: Option<Vec<Item>>,
    #[webidl(sequence, converter = "dictionary")]
    #[webapi(data_property)]
    modifiers: Option<Vec<Modifier<'s>>>,
    #[webidl(sequence, converter = "dictionary")]
    #[webapi(data_property)]
    shipping_options: Option<Vec<ShippingOption>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PaymentDetailsInit")]
pub(super) struct Details<'s> {
    #[webidl(inherit)]
    base: DetailsBase<'s>,
    #[webidl(converter = "raw")]
    pub(super) id: Option<Text>,
    #[webidl(required, dictionary)]
    total: Item,
}

impl<'s> Details<'s> {
    pub(super) fn into_snapshot(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Object> {
        let object = self.base.bind(scope).expect("converted PaymentDetailsBase");
        let id = self
            .id
            .expect("assigned payment id")
            .to_v8_value(scope)
            .expect("payment id");
        let total = self.total.bind(scope).expect("converted payment total");
        assert_eq!(
            object.create_data_property(scope, crate::util::v8str(scope, "id").into(), id),
            Some(true)
        );
        assert_eq!(
            object.create_data_property(
                scope,
                crate::util::v8str(scope, "total").into(),
                total.into()
            ),
            Some(true)
        );
        object
    }
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "PaymentOptions")]
#[webapi(plain, enumerable)]
pub(super) struct Options {
    #[webidl(default = false)]
    #[webapi(data_property)]
    request_billing_address: bool,
    #[webidl(default = false)]
    #[webapi(data_property)]
    request_payer_email: bool,
    #[webidl(default = false)]
    #[webapi(data_property)]
    request_payer_name: bool,
    #[webidl(default = false)]
    #[webapi(data_property)]
    request_payer_phone: bool,
    #[webidl(default = false)]
    #[webapi(data_property)]
    pub(super) request_shipping: bool,
    #[webidl(converter = "enum", default = ShippingType::Shipping)]
    #[webapi(data_property)]
    pub(super) shipping_type: ShippingType,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PaymentRequest")]
pub(super) struct Args<'s> {
    #[webidl(required, sequence, converter = "dictionary")]
    pub(super) method_data: Vec<MethodData<'s>>,
    #[webidl(required, dictionary)]
    pub(super) details: Details<'s>,
    #[webidl(dictionary)]
    pub(super) options: Options,
}

pub(super) fn process_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    methods: &mut [MethodData<'s>],
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    if methods.is_empty() {
        throw_type_error(scope, "At least one payment method is required.");
        return None;
    }
    let mut seen = HashSet::new();
    let mut data = Vec::new();
    for method in methods {
        let value = method.supported_methods.string();
        let key = match payment_method_key(&value) {
            Some(key) => key,
            None => {
                range_error(scope, "Invalid payment method identifier.");
                return None;
            }
        };
        if !seen.insert(key) {
            range_error(scope, "Duplicate payment method identifier.");
            return None;
        }
        data.push(match method.data.take() {
            Some(object) => serialize(scope, object)?.into(),
            None => v8::null(scope).into(),
        });
    }
    Some(data)
}

pub(super) fn process_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    details: &mut Details<'s>,
    shipping: bool,
) -> Option<(Option<Text>, Vec<v8::Local<'s, v8::Value>>)> {
    check_amount(scope, &mut details.total.amount, true)?;
    for item in details.base.display_items.iter_mut().flatten() {
        check_amount(scope, &mut item.amount, false)?;
    }
    let mut selected = None;
    if shipping {
        let mut seen = HashSet::new();
        for option in details.base.shipping_options.iter_mut().flatten() {
            check_amount(scope, &mut option.amount, false)?;
            if !seen.insert(option.id.0.0.clone()) {
                throw_type_error(scope, "Shipping option identifiers must be unique.");
                return None;
            }
            if option.selected {
                selected = Some(option.id.clone());
            }
        }
    }
    let mut data = Vec::new();
    for modifier in details.base.modifiers.iter_mut().flatten() {
        if let Some(total) = &mut modifier.total {
            check_amount(scope, &mut total.amount, true)?;
        }
        for item in modifier.additional_display_items.iter_mut().flatten() {
            check_amount(scope, &mut item.amount, false)?;
        }
        data.push(match modifier.data.take() {
            Some(object) => serialize(scope, object)?.into(),
            None => v8::null(scope).into(),
        });
    }
    Some((selected, data))
}

fn check_amount(scope: &mut v8::PinScope<'_, '_>, amount: &mut Amount, total: bool) -> Option<()> {
    let currency = &amount.currency.0.0;
    if currency.len() != 3
        || !currency
            .iter()
            .all(|unit| matches!(*unit, 0x41..=0x5a | 0x61..=0x7a))
    {
        range_error(scope, "Currency must contain three ASCII letters.");
        return None;
    }
    amount.currency = Text::from_string(&amount.currency.string().to_ascii_uppercase());
    let value = amount.value.string();
    let unsigned = value.strip_prefix('-').unwrap_or(&value);
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(i, f)| (i, Some(f)));
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(integer)
        || fraction.is_some_and(|value| !digits(value))
        || total && value.starts_with('-')
    {
        throw_type_error(scope, "Invalid decimal monetary value.");
        return None;
    }
    Some(())
}

fn payment_method_key(value: &str) -> Option<String> {
    if let Ok(url) = url::Url::parse(value) {
        return (url.scheme() == "https"
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none_or(str::is_empty))
        .then(|| url.into());
    }
    value
        .split('-')
        .all(|part| {
            let mut bytes = part.bytes();
            bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
                && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        .then(|| value.to_owned())
}

fn serialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::String>> {
    let result = {
        let catcher = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut catcher.init();
        match v8::json::stringify(scope, object.into()) {
            Some(value) if value.to_rust_string_lossy(scope) != "undefined" => {
                Ok(Some(v8::Global::new(scope, value)))
            }
            Some(_) => Ok(None),
            None if scope.has_caught() => {
                let error = scope.exception().expect("JSON exception");
                Err(v8::Global::new(scope, error))
            }
            None => Ok(None),
        }
    };
    match result {
        Ok(Some(value)) => Some(v8::Local::new(scope, value)),
        Ok(None) => {
            throw_type_error(scope, "Payment method data must serialize to JSON.");
            None
        }
        Err(error) => {
            let error = v8::Local::new(scope, error);
            scope.throw_exception(error);
            None
        }
    }
}

fn range_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let message = v8_string(scope, message).expect("payment validation message");
    let error = v8::Exception::range_error(scope, message);
    scope.throw_exception(error);
}
