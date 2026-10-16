mod callables;
mod callbacks;
mod constructors;
mod exceptions;
mod json;
mod microtasks;
mod object_readers;
mod properties;
mod strings;
mod symbols;

pub use callables::*;
pub use callbacks::*;
pub use constructors::*;
pub use exceptions::*;
pub use json::*;
pub use microtasks::*;
pub use object_readers::*;
pub use properties::*;
pub use strings::*;
pub use symbols::*;

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::{
        get_own_property, get_own_static_property, indexed_callback_data, object_own_bool_property,
        object_own_number_property, object_own_property_as_array, object_own_property_as_object,
        object_own_static_number_property, object_own_static_string_property,
        object_own_string_property, set_static_property, v8str,
    };
    use moli_v8_test_util::ensure_v8;

    #[test]
    fn indexed_callback_data_returns_expected_item() {
        assert_eq!(
            indexed_callback_data(&["alpha", "beta"], 1, "test slots"),
            Some("beta")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "callback data index 2 out of bounds for test slots")]
    fn indexed_callback_data_debug_panics_on_out_of_bounds_index() {
        let _ = indexed_callback_data(&["alpha"], 2, "test slots");
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn indexed_callback_data_returns_none_on_out_of_bounds_index() {
        assert_eq!(indexed_callback_data(&["alpha"], 2, "test slots"), None);
    }

    #[test]
    fn own_property_readers_ignore_prototype_properties() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let prototype = v8::Object::new(scope);
        let prototype_slot = v8::Number::new(scope, 99.0);
        set_static_property(scope, prototype, "__moliSlot", prototype_slot.into());
        let object = v8::Object::new(scope);
        let _ = object.set_prototype(scope, prototype.into());

        assert!(get_own_static_property(scope, object, "__moliSlot").is_none());
        assert!(get_own_property(scope, object, "__moliSlot").is_none());
        assert!(object_own_static_number_property(scope, object, "__moliSlot").is_none());

        let object_slot = v8::Number::new(scope, 7.0);
        set_static_property(scope, object, "__moliSlot", object_slot.into());
        assert_eq!(
            object_own_static_number_property(scope, object, "__moliSlot"),
            Some(7.0)
        );
        assert_eq!(
            get_own_property(scope, object, "__moliSlot")
                .and_then(|value| value.number_value(scope)),
            Some(7.0)
        );
        assert!(
            object
                .get(scope, v8str(scope, "__moliSlot").into())
                .is_some()
        );
    }

    #[test]
    fn own_dynamic_property_readers_are_typed_and_own_only() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let prototype = v8::Object::new(scope);
        let inherited_name = v8::String::new(scope, "inherited").unwrap();
        set_static_property(scope, prototype, "name", inherited_name.into());
        let object = v8::Object::new(scope);
        let _ = object.set_prototype(scope, prototype.into());

        assert_eq!(object_own_string_property(scope, object, "name"), None);
        assert_eq!(
            object_own_static_string_property(scope, object, "name"),
            None
        );
        assert_eq!(object_own_number_property(scope, object, "count"), None);
        assert_eq!(object_own_bool_property(scope, object, "enabled"), None);
        assert!(object_own_property_as_object(scope, object, "child").is_none());
        assert!(object_own_property_as_array(scope, object, "items").is_none());

        let name = v8::String::new(scope, "own").unwrap();
        set_static_property(scope, object, "name", name.into());
        let count = v8::Number::new(scope, 3.0);
        set_static_property(scope, object, "count", count.into());
        let enabled = v8::Boolean::new(scope, true);
        set_static_property(scope, object, "enabled", enabled.into());
        let child = v8::Object::new(scope);
        set_static_property(scope, object, "child", child.into());
        let items = v8::Array::new(scope, 0);
        set_static_property(scope, object, "items", items.into());

        assert_eq!(
            object_own_string_property(scope, object, "name"),
            Some("own".into())
        );
        assert_eq!(
            object_own_static_string_property(scope, object, "name"),
            Some("own".into())
        );
        assert_eq!(
            object_own_number_property(scope, object, "count"),
            Some(3.0)
        );
        assert_eq!(
            object_own_bool_property(scope, object, "enabled"),
            Some(true)
        );
        assert!(object_own_property_as_object(scope, object, "child").is_some());
        assert!(object_own_property_as_array(scope, object, "items").is_some());
    }
}
