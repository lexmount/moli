use crate::page::PermissionOverrideRegistration;

/// Context-owned rules. The Browser's defaults are read at snapshot time,
/// never copied into every Context as another source of truth.
#[derive(Debug, Default)]
pub struct PermissionOverrides {
    entries: Vec<OrderedOverride>,
}

/// Browser-wide defaults and the order of permission writes across scopes.
/// This order preserves last-matching-write semantics; it is not an object
/// incarnation or renderer command sequence.
#[derive(Debug, Default)]
pub struct PermissionDefaults {
    overrides: PermissionOverrides,
    next_order: u64,
}

#[derive(Debug)]
struct OrderedOverride {
    order: u64,
    registration: PermissionOverrideRegistration,
}

impl PermissionDefaults {
    fn order(&mut self, registration: PermissionOverrideRegistration) -> OrderedOverride {
        let order = self.next_order;
        self.next_order = order
            .checked_add(1)
            .expect("permission write order exhausted");
        OrderedOverride {
            order,
            registration,
        }
    }

    pub fn set(&mut self, registration: PermissionOverrideRegistration) {
        let update = self.order(registration);
        self.overrides.apply(update);
    }

    pub fn clear(&mut self) {
        self.overrides.clear();
    }

    pub fn override_count(&self) -> usize {
        self.overrides.override_count()
    }

    pub fn snapshot(&self) -> Vec<PermissionOverrideRegistration> {
        self.overrides
            .entries
            .iter()
            .map(|entry| entry.registration.clone())
            .collect()
    }
}

impl PermissionOverrides {
    pub fn set(
        &mut self,
        defaults: &mut PermissionDefaults,
        registration: PermissionOverrideRegistration,
    ) {
        self.apply(defaults.order(registration));
    }

    fn apply(&mut self, update: OrderedOverride) {
        self.entries.retain(|entry| {
            let current = &entry.registration;
            let replacement = &update.registration;
            current.permission != replacement.permission
                || current.origin != replacement.origin
                || current.embedded_origin != replacement.embedded_origin
        });
        self.entries.push(update);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn override_count(&self) -> usize {
        self.entries.len()
    }

    pub fn snapshot(&self, defaults: &PermissionDefaults) -> Vec<PermissionOverrideRegistration> {
        let mut entries = defaults
            .overrides
            .entries
            .iter()
            .chain(&self.entries)
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.order);
        entries
            .into_iter()
            .map(|entry| entry.registration.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(setting: &str) -> PermissionOverrideRegistration {
        PermissionOverrideRegistration {
            permission: json!({"name": "geolocation"}),
            setting: setting.into(),
            origin: None,
            embedded_origin: None,
        }
    }

    #[test]
    fn merging_scopes_retains_write_order_and_does_not_copy_defaults() {
        let mut defaults = PermissionDefaults::default();
        let mut context = PermissionOverrides::default();
        defaults.set(rule("denied"));
        context.set(&mut defaults, rule("granted"));
        assert_eq!(
            context.snapshot(&defaults),
            vec![rule("denied"), rule("granted")]
        );
        defaults.set(rule("prompt"));
        assert_eq!(
            context.snapshot(&defaults),
            vec![rule("granted"), rule("prompt")]
        );
        context.set(&mut defaults, rule("denied"));
        assert_eq!(
            context.snapshot(&defaults),
            vec![rule("prompt"), rule("denied")]
        );
        assert_eq!(context.override_count(), 1);
        assert_eq!(defaults.override_count(), 1);
        assert_eq!(
            PermissionOverrides::default().snapshot(&defaults),
            vec![rule("prompt")]
        );
    }

    #[test]
    fn reset_and_drop_release_only_the_owning_scope() {
        let mut defaults = PermissionDefaults::default();
        let mut first = PermissionOverrides::default();
        let mut second = PermissionOverrides::default();
        defaults.set(rule("denied"));
        first.set(&mut defaults, rule("granted"));
        second.set(&mut defaults, rule("prompt"));
        first.clear();
        assert_eq!(first.snapshot(&defaults), vec![rule("denied")]);
        assert_eq!(
            second.snapshot(&defaults),
            vec![rule("denied"), rule("prompt")]
        );
        drop(first);
        assert_eq!(defaults.snapshot(), vec![rule("denied")]);
        defaults.clear();
        assert_eq!(second.snapshot(&defaults), vec![rule("prompt")]);
        defaults.set(rule("granted"));
        assert_eq!(
            second.snapshot(&defaults),
            vec![rule("prompt"), rule("granted")]
        );
    }

    #[test]
    fn replacing_a_rule_matches_the_full_descriptor_and_origin_pair() {
        let mut defaults = PermissionDefaults::default();
        let mut context = PermissionOverrides::default();
        let mut embedded = rule("granted");
        embedded.origin = Some("https://embedding.test".into());
        embedded.embedded_origin = Some("https://requesting.test".into());
        let mut other_descriptor = embedded.clone();
        other_descriptor.permission = json!({"name": "midi", "sysex": true});
        for registration in [rule("denied"), embedded.clone(), other_descriptor.clone()] {
            context.set(&mut defaults, registration);
        }
        embedded.setting = "prompt".into();
        context.set(&mut defaults, embedded.clone());
        assert_eq!(
            context.snapshot(&defaults),
            vec![rule("denied"), other_descriptor, embedded]
        );
        assert_eq!(context.override_count(), 3);
    }
}
