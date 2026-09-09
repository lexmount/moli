use super::navigation_entry::{history_index, navigation_current_entry_index};
use super::navigation_serialize::serialize_history_entries;
use super::navigation_window::window_history_for_holder;
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use moli_page_types::{
    NavigationHistoryEntrySeed, NavigationHistoryMutation, cross_document_navigation_seed,
};
use url::Url;

pub(crate) struct FormNavigationHistory {
    pub(crate) mutation: NavigationHistoryMutation,
    pub(crate) entry_seed: Option<NavigationHistoryEntrySeed>,
}

impl FormNavigationHistory {
    pub(crate) fn capture(
        scope: &mut v8::PinScope<'_, '_>,
        host: &mut JsContextHost,
        source_document: Option<DomHandle>,
        target_child: Option<DomHandle>,
        destination: &Url,
        requested_mutation: Option<NavigationHistoryMutation>,
    ) -> Self {
        let mutation = requested_mutation.unwrap_or_else(|| {
            host.form_navigation_history_mutation(source_document, target_child, destination)
        });
        let owner = match target_child {
            Some(handle) => host.existing_child_browsing_context_window_wrapper(scope, handle),
            None => Some(scope.get_current_context().global(scope)),
        };
        let entry_seed = owner.and_then(|owner| {
            let history = window_history_for_holder(scope, owner)?;
            let index = history_index(scope, history);
            let navigation_index = navigation_current_entry_index(scope, owner).unwrap_or(0);
            Some(cross_document_navigation_seed(
                serialize_history_entries(scope, history),
                index,
                navigation_index,
                destination,
                mutation,
            ))
        });
        Self {
            mutation,
            entry_seed,
        }
    }
}
