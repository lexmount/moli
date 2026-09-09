use super::*;
use crate::util::{get_private_value, set_private_value};
use anyhow::Result;

mod date;
mod intl;
mod overrides;

const DATE_LOCALE_RUNTIME_INSTALLED_SLOT: &str = "__moliDateLocaleRuntimeInstalled";

pub(super) fn install_date_locale_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    if get_private_value(scope, global, DATE_LOCALE_RUNTIME_INSTALLED_SLOT).is_some() {
        return Ok(());
    }
    date::install_date_runtime_state(scope, global)?;
    intl::install_intl_default_override_constructors(scope, global)?;
    set_private_value(
        scope,
        global,
        DATE_LOCALE_RUNTIME_INSTALLED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    Ok(())
}
