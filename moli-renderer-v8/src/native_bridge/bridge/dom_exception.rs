use crate::context_bootstrap::new_most_derived_dom_exception_value;

pub(crate) fn throw_dom_exception(
    scope: &mut v8::PinScope<'_, '_>,
    name: &'static str,
    _code: i32,
    message: &str,
) {
    let exception = new_most_derived_dom_exception_value(scope, message, name);
    scope.throw_exception(exception);
}
