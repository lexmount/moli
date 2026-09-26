use super::*;
use crate::forms::normalize_textarea_api_value;

impl DomHost {
    pub(super) fn textarea_value_excluding_children(
        &self,
        parent: DomHandle,
        excluded: &[DomHandle],
    ) -> Option<String> {
        let element = self.node(parent)?.as_element()?;
        if !element.is_html_textarea() || element.input_value_dirty() {
            return None;
        }
        let mut value = String::new();
        for child in self.child_handles(parent) {
            if excluded.contains(&child) {
                continue;
            }
            let Some(node) = self.node(child) else {
                continue;
            };
            if node.is_text() || node.is_cdata_section() {
                value.push_str(node.node_value().unwrap_or_default());
            }
        }
        Some(normalize_textarea_api_value(&value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_and_textarea() -> (DomHost, DomHandle, DomHandle) {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://textarea-values.test/").unwrap(),
        ));
        let textarea = host.create_element("textarea");
        let first = host.create_text_node("A😀");
        let last = host.create_text_node("B\nC");
        assert!(host.append_child(textarea, first));
        assert!(host.append_child(textarea, last));
        (host, textarea, first)
    }

    #[test]
    fn textarea_replacement_retains_intermediate_lengths_without_observer_records() {
        for observe in [false, true] {
            let (mut host, textarea, _) = host_and_textarea();
            host.set_mutation_observer_records_enabled(observe);
            let effects = host.set_text_content_effects(textarea, "A😀B\nC");
            assert_eq!(
                effects
                    .textarea_values()
                    .iter()
                    .map(|change| (change.target(), change.length()))
                    .collect::<Vec<_>>(),
                vec![(textarea, 3), (textarea, 0), (textarea, 6)],
                "observer enabled: {observe}",
            );
            assert_eq!(effects.observer_records().records().is_empty(), !observe);
        }
    }

    #[test]
    fn textarea_character_data_changes_use_normalized_utf16_lengths_and_respect_dirty_values() {
        for observe in [false, true] {
            let (mut host, textarea, first) = host_and_textarea();
            host.set_mutation_observer_records_enabled(observe);
            let effects = host.set_text_content_effects(first, "😀\r\n");
            assert_eq!(effects.textarea_values().len(), 1);
            assert_eq!(effects.textarea_values()[0].length(), 6);
            assert!(
                host.set_text_content_effects(first, "😀\n")
                    .textarea_values()
                    .is_empty()
            );
            host.set_input_value(textarea, "dirty");
            assert!(
                host.set_text_content_effects(first, "replacement")
                    .textarea_values()
                    .is_empty()
            );
        }
    }

    #[test]
    fn moving_text_retains_the_removal_length_even_when_the_final_value_is_unchanged() {
        for observe in [false, true] {
            let (mut host, textarea, first) = host_and_textarea();
            host.set_mutation_observer_records_enabled(observe);
            let effects = host.insert_before_effects(textarea, first, Some(first));
            assert_eq!(
                effects
                    .textarea_values()
                    .iter()
                    .map(|change| change.length())
                    .collect::<Vec<_>>(),
                vec![3, 6],
            );
        }
    }
}
