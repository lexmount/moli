use crate::util::v8_string;

pub(super) const ADOPTED_STYLE_SHEETS_SLOT: &str = "__moliAdoptedStyleSheets";
pub(super) const STYLE_SHEETS_SLOT: &str = "__moliStyleSheets";
pub(super) const FONTS_SLOT: &str = "__moliFonts";
pub(super) const FONT_FACE_SET_CONNECTED_FACES_SLOT: &str = "__moliFontFaceSetConnectedFaces";
pub(super) const FONT_FACE_SET_CONNECTED_OWNER_FACES_SLOT: &str =
    "__moliFontFaceSetConnectedOwnerFaces";

pub(super) fn object_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<bool> {
    let key = v8_string(scope, key)?;
    let value = object.get(scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    Some(value.boolean_value(scope))
}
