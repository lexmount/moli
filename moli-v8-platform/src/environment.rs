//! Renderer-process defaults, not a second implementation of Date or Intl.
//!
//! Like Blink's TimeZoneController/LocaleController, one owner may override
//! each ICU default at a time. All Pages and Workers in this process observe
//! that default; independent environments require separate processes. Native
//! V8 retains responsibility for parsing, coercion, locale matching and DST.

use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;

use super::registered_isolate_owners;

static ENVIRONMENT: OnceLock<Mutex<Environment>> = OnceLock::new();

/// A committed change to the renderer-process defaults, delivered as owner
/// work rather than checked at every V8 entry. Delivery must enqueue the
/// notification; only the isolate's owner may apply it, including in a nested
/// Inspector pause loop. It carries no isolate pointer or configuration lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessEnvironmentChange {
    LocaleChanged,
    TimezoneChanged,
}

impl ProcessEnvironmentChange {
    /// Notify native V8 caches on the entered isolate's owner thread. Locale
    /// changes also invalidate cached Date locale formatters, as in Blink.
    pub fn notify_isolate(self, isolate: &mut v8::Isolate) {
        if self == Self::LocaleChanged {
            isolate.locale_configuration_change_notification();
        }
        // Redetect would overwrite the emulated ICU timezone with the host zone.
        isolate.date_time_configuration_change_notification(v8::TimeZoneDetection::Skip);
    }
}

fn environment() -> &'static Mutex<Environment> {
    ENVIRONMENT.get_or_init(|| {
        Mutex::new(Environment {
            original_locale: v8::icu::get_default_locale_name(),
            original_timezone: v8::icu::get_default_time_zone(),
            locale: None,
            timezone: None,
        })
    })
}

struct Claim {
    owner: EnvironmentOwnerId,
    value: String,
}

struct Environment {
    original_locale: String,
    original_timezone: String,
    locale: Option<Claim>,
    timezone: Option<Claim>,
}

/// Unique authority for one configuration owner. Moving it transfers ownership;
/// dropping it restores the defaults it still owns. It deliberately cannot be
/// cloned into policy snapshots or deferred work that outlives a session.
#[derive(Debug, PartialEq, Eq)]
pub struct ProcessEnvironmentOwner {
    id: EnvironmentOwnerId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EnvironmentOwnerId(u64);

impl Default for ProcessEnvironmentOwner {
    fn default() -> Self {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("process environment owner ID exhausted");
        Self {
            id: EnvironmentOwnerId(id),
        }
    }
}

impl ProcessEnvironmentOwner {
    pub fn set_locale(&self, locale: Option<&str>) -> Result<(), &'static str> {
        let locale = locale.filter(|value| !value.is_empty());
        let mut state = environment().lock();
        if state
            .locale
            .as_ref()
            .is_some_and(|claim| claim.owner != self.id)
        {
            return Err("Another locale override is already in effect");
        }
        if state.locale.as_ref().map(|claim| claim.value.as_str()) == locale {
            return Ok(());
        }
        if let Some(locale) = locale {
            if !v8::icu::try_set_default_locale(locale) {
                return Err("Invalid locale name");
            }
        } else {
            v8::icu::set_default_locale(&state.original_locale);
        }
        state.locale = locale.map(|value| Claim {
            owner: self.id,
            value: value.to_owned(),
        });
        publish_change(state, ProcessEnvironmentChange::LocaleChanged);
        Ok(())
    }

    /// As in Chromium, requesting an already-installed timezone succeeds but
    /// does not acquire another lease. Clearing a non-owner is a no-op.
    pub fn set_timezone(&self, timezone: Option<&str>) -> Result<(), &'static str> {
        let timezone = timezone.filter(|value| !value.is_empty());
        let mut state = environment().lock();
        if state.timezone.as_ref().map(|claim| claim.value.as_str()) == timezone {
            return Ok(());
        }
        let owns = state
            .timezone
            .as_ref()
            .is_some_and(|claim| claim.owner == self.id);
        if timezone.is_none() && !owns {
            return Ok(());
        }
        if state.timezone.is_some() && !owns {
            return Err("Timezone override is already in effect");
        }
        let applied = match timezone {
            Some(timezone) => v8::icu::set_default_time_zone(timezone),
            None => v8::icu::restore_default_time_zone(&state.original_timezone),
        };
        if !applied {
            return Err("Invalid timezone id");
        }
        state.timezone = timezone.map(|value| Claim {
            owner: self.id,
            value: value.to_owned(),
        });
        publish_change(state, ProcessEnvironmentChange::TimezoneChanged);
        Ok(())
    }

    /// Only claims belonging to this identity, not another owner's inherited
    /// process defaults. This is also the sole source of retained policy state.
    pub fn locale(&self) -> Option<String> {
        let state = ENVIRONMENT.get()?.lock();
        state
            .locale
            .as_ref()
            .filter(|claim| claim.owner == self.id)
            .map(|claim| claim.value.clone())
    }

    pub fn timezone(&self) -> Option<String> {
        let state = ENVIRONMENT.get()?.lock();
        state
            .timezone
            .as_ref()
            .filter(|claim| claim.owner == self.id)
            .map(|claim| claim.value.clone())
    }

    pub fn has_override(&self) -> bool {
        let Some(environment) = ENVIRONMENT.get() else {
            return false;
        };
        let state = environment.lock();
        [&state.locale, &state.timezone]
            .iter()
            .any(|claim| claim.as_ref().is_some_and(|claim| claim.owner == self.id))
    }

    /// Releases both claims without retiring this owner. Repeated release (or
    /// a later drop) cannot clear defaults subsequently claimed by another owner.
    pub fn release(&self) {
        release(self.id);
    }
}

impl Drop for ProcessEnvironmentOwner {
    fn drop(&mut self) {
        self.release();
    }
}

fn release(owner: EnvironmentOwnerId) {
    // Unused identities must not initialize ICU or capture host defaults.
    let Some(environment) = ENVIRONMENT.get() else {
        return;
    };
    let mut state = environment.lock();
    let locale = state
        .locale
        .as_ref()
        .is_some_and(|claim| claim.owner == owner);
    let timezone = state
        .timezone
        .as_ref()
        .is_some_and(|claim| claim.owner == owner);
    if !locale && !timezone {
        return;
    }
    if locale {
        v8::icu::set_default_locale(&state.original_locale);
        state.locale = None;
    }
    if timezone {
        let restored = v8::icu::restore_default_time_zone(&state.original_timezone);
        debug_assert!(restored, "the original ICU timezone must be restorable");
        state.timezone = None;
    }
    publish_change(
        state,
        if locale {
            ProcessEnvironmentChange::LocaleChanged
        } else {
            ProcessEnvironmentChange::TimezoneChanged
        },
    );
}

/// Keep ICU updates and notification publication serialized, as in Blink's
/// controllers. Dispatchers only enqueue owner work, never enter V8 or acquire
/// this configuration lock. Publication finishes before the setter returns,
/// so a subsequent command cannot be enqueued ahead of its notification.
fn publish_change(
    _state: parking_lot::MutexGuard<'_, Environment>,
    change: ProcessEnvironmentChange,
) {
    for owner in registered_isolate_owners() {
        if owner.registration.generation.is_active() {
            (owner.registration.environment_changed)(change);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_owner_queries_and_drop_do_not_initialize_process_defaults() {
        // Nextest runs each process-global environment test in its own process.
        assert!(ENVIRONMENT.get().is_none());
        let owner = ProcessEnvironmentOwner::default();
        assert_eq!(owner.locale(), None);
        assert_eq!(owner.timezone(), None);
        assert!(!owner.has_override());
        owner.release();
        drop(owner);
        assert!(ENVIRONMENT.get().is_none());
    }

    #[test]
    fn independent_claims_and_released_owners_do_not_clear_replacements() {
        let locale = v8::icu::get_default_locale_name();
        let timezone = v8::icu::get_default_time_zone();
        let locale_owner = ProcessEnvironmentOwner::default();
        let timezone_owner = ProcessEnvironmentOwner::default();
        locale_owner.set_locale(Some("fr_FR")).unwrap();
        timezone_owner.set_timezone(Some("Europe/Paris")).unwrap();
        let moved_owner = locale_owner;
        assert_eq!(moved_owner.locale().as_deref(), Some("fr_FR"));
        moved_owner.release();
        assert_eq!(v8::icu::get_default_locale_name(), locale);
        assert_eq!(v8::icu::get_default_time_zone(), "Europe/Paris");

        let replacement = ProcessEnvironmentOwner::default();
        replacement.set_locale(Some("de_DE")).unwrap();
        moved_owner.release();
        drop(moved_owner);
        assert_eq!(replacement.locale().as_deref(), Some("de_DE"));
        drop(timezone_owner);
        assert_eq!(v8::icu::get_default_time_zone(), timezone);
        assert_eq!(v8::icu::get_default_locale_name(), "de_DE");
        drop(replacement);
        assert_eq!(v8::icu::get_default_locale_name(), locale);
    }

    #[test]
    fn environment_changes_are_published_before_return_without_polling_or_redundant_work() {
        moli_v8_init::ensure_v8_initialized(crate::create_platform);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut isolate = v8::Isolate::new(Default::default());
            let registration = crate::V8PlatformIsolateRegistration::register(
                &mut isolate,
                crate::V8ForegroundTaskWake::queued(|_| {}),
                move |change| tx.send(change).unwrap(),
            );
            let owner = ProcessEnvironmentOwner::default();
            owner.set_locale(Some("fr_FR")).unwrap();
            owner.set_timezone(Some("Europe/Paris")).unwrap();
            // No runtime yield or isolate entry: publication is synchronous,
            // ordered, and application belongs to the receiving owner.
            assert_eq!(
                rx.try_recv().unwrap(),
                ProcessEnvironmentChange::LocaleChanged
            );
            assert_eq!(
                rx.try_recv().unwrap(),
                ProcessEnvironmentChange::TimezoneChanged
            );
            owner.set_locale(Some("fr_FR")).unwrap();
            owner.set_timezone(Some("Europe/Paris")).unwrap();
            assert!(owner.set_timezone(Some("Invalid/Zone")).is_err());
            let peer = ProcessEnvironmentOwner::default();
            assert!(peer.set_locale(Some("de_DE")).is_err());
            peer.set_timezone(Some("Europe/Paris")).unwrap();
            peer.set_timezone(None).unwrap();
            assert!(rx.try_recv().is_err());
            owner.release();
            assert_eq!(
                rx.try_recv().unwrap(),
                ProcessEnvironmentChange::LocaleChanged
            );
            owner.release();
            assert!(rx.try_recv().is_err());
            registration.unregister();
            drop(registration);
            drop(isolate);
            owner.set_locale(Some("de_DE")).unwrap();
            assert!(
                rx.try_recv().is_err(),
                "retired owners receive no notification"
            );
        });
    }

    #[test]
    fn native_default_zone_offset_includes_dst_and_rejects_invalid_input() {
        let owner = ProcessEnvironmentOwner::default();
        owner.set_timezone(Some("Europe/Paris")).unwrap();
        assert_eq!(
            v8::icu::default_time_zone_offset_seconds(1_704_067_200_000.0),
            Some(3600)
        );
        assert_eq!(
            v8::icu::default_time_zone_offset_seconds(1_719_792_000_000.0),
            Some(7200)
        );
        assert_eq!(v8::icu::default_time_zone_offset_seconds(f64::NAN), None);
        owner.set_timezone(Some("GMT+05:30")).unwrap();
        assert_eq!(
            v8::icu::default_time_zone_offset_seconds(1_704_067_200_000.0),
            Some(19_800)
        );
        let locale = v8::icu::get_default_locale_name();
        owner.set_locale(Some("en_US_POSIX")).unwrap();
        assert_eq!(v8::icu::get_default_locale_name(), "en_US_POSIX");
        owner.set_locale(None).unwrap();
        assert_eq!(v8::icu::get_default_locale_name(), locale);
    }
}
