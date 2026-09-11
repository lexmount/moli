use crate::{
    BindError, implements_interface, initialize_web_api_object, register_web_api_interfaces, v8,
};

/// Native interface metadata shared by constructors, factories and receivers.
/// Public JavaScript prototypes never supply this identity or its inheritance.
#[derive(Clone, Copy, Debug)]
pub struct WebApiInterfaceDescriptor {
    name: &'static str,
    parent: Option<&'static Self>,
}

impl WebApiInterfaceDescriptor {
    pub const fn new(name: &'static str, parent: Option<&'static Self>) -> Self {
        Self { name, parent }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn parent_name(self) -> Option<&'static str> {
        match self.parent {
            Some(parent) => Some(parent.name),
            None => None,
        }
    }

    /// Register the full ancestry even in a realm without exposed constructors.
    /// Each edge is validated before following it; repeated metadata is cheap,
    /// and invalid cyclic native descriptors fail rather than looping forever.
    pub fn register<C>(self, scope: &mut v8::PinScope<'_, '_, C>) -> Result<(), BindError> {
        let mut current = Some(self);
        while let Some(interface) = current {
            register_web_api_interfaces(scope, [(interface.name, interface.parent_name())])?;
            current = interface.parent.copied();
        }
        Ok(())
    }

    pub fn initialize<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<(), BindError> {
        self.register(scope)?;
        initialize_web_api_object(scope, object, self.name)
    }

    pub fn is_instance<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> bool {
        implements_interface(scope, object, self.name)
    }
}

/// Declare native interface identities once and refer to them from derives.
///
/// ```ignore
/// declare_web_api_interfaces! {
///     pub Event;
///     pub ProgressEvent: Event;
///     pub HeadersIterator = "Headers Iterator";
/// }
///
/// #[derive(WebApiObject)]
/// #[webapi(interface = ProgressEvent)]
/// struct ProgressEventState { /* private slots */ }
///
/// #[derive(WebApiFunctionTemplate)]
/// #[webapi(interface = Event, receiver)]
/// struct EventMethods { /* instance methods and accessors */ }
/// ```
///
/// The optional JavaScript name is for names that are not Rust identifiers.
/// A descriptor defines native ancestry; it does not install a JS prototype.
#[macro_export]
macro_rules! declare_web_api_interfaces {
    ($($visibility:vis $name:ident $(= $js_name:literal)? $(: $parent:path)?;)+) => {
        $(
            #[allow(clippy::upper_case_acronyms)]
            $visibility struct $name;
            // Generated receiver helpers are intentionally optional at each use.
            #[allow(dead_code)]
            impl $name {
                pub const NAME: &'static str = $crate::declare_web_api_interfaces!(@name $name $(, $js_name)?);

                pub const DESCRIPTOR: $crate::WebApiInterfaceDescriptor =
                    $crate::WebApiInterfaceDescriptor::new(
                        $crate::declare_web_api_interfaces!(@name $name $(, $js_name)?),
                        $crate::declare_web_api_interfaces!(@parent $($parent)?),
                    );

                pub fn is_instance<'s>(
                    scope: &mut $crate::v8::PinScope<'s, '_>,
                    object: $crate::v8::Local<'s, $crate::v8::Object>,
                ) -> bool {
                    Self::DESCRIPTOR.is_instance(scope, object)
                }
            }
        )+
    };
    (@name $name:ident) => { stringify!($name) };
    (@name $name:ident, $js_name:literal) => { $js_name };
    (@parent) => { ::std::option::Option::None };
    (@parent $parent:path) => { ::std::option::Option::Some(&<$parent>::DESCRIPTOR) };
}
