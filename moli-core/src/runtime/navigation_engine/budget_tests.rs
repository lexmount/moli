use super::{NavigationEngine, NavigationRuntimeConfig};
use crate::{
    LayoutPolicy, LayoutWorkingSetBudget, OptionalResourceFetchMask,
    runtime::{Browser, BrowserConfig},
};
use moli_fetch::FetchConfig;

fn config(policy: LayoutPolicy) -> NavigationRuntimeConfig {
    NavigationRuntimeConfig::new(
        FetchConfig::default(),
        OptionalResourceFetchMask::NONE,
        true,
        policy,
    )
}

fn resolved_pool(engine: &NavigationEngine) -> LayoutWorkingSetBudget {
    engine
        .runtime_config()
        .layout_working_set_budget()
        .expect("Realtime engine must expose its resolved pool")
        .clone()
}

#[test]
fn legacy_browser_and_navigation_do_not_create_working_set_budgets() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let browser_config = BrowserConfig::default().with_layout_policy(policy);
        assert!(browser_config.layout_working_set_budget().is_none());
        let mut navigation_config = NavigationRuntimeConfig::from(&browser_config);
        navigation_config.ensure_layout_working_set_budget();
        assert!(navigation_config.layout_working_set_budget().is_none());

        let browser = Browser::new(browser_config).expect("legacy browser");
        assert!(browser.config().layout_working_set_budget().is_none());
        let engine = NavigationEngine::new_with_runtime_config(navigation_config);
        assert!(
            engine
                .runtime_config()
                .layout_working_set_budget()
                .is_none()
        );
    }
}

#[test]
fn legacy_resolution_does_not_retain_an_explicit_working_set_budget() {
    let pool = LayoutWorkingSetBudget::default();
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let browser = Browser::new(
            BrowserConfig::default()
                .with_layout_policy(policy)
                .with_layout_working_set_budget(pool.clone()),
        )
        .expect("legacy browser ignores the explicit pool");
        assert!(browser.config().layout_working_set_budget().is_none());

        let engine = NavigationEngine::new_with_runtime_config(
            config(policy).with_layout_working_set_budget(pool.clone()),
        );
        assert!(
            engine
                .runtime_config()
                .layout_working_set_budget()
                .is_none()
        );
    }
    assert_eq!(pool.snapshot().open_pages, 0);
    assert_eq!(pool.snapshot().charged_bytes, 0);
}

#[test]
fn realtime_root_config_allocates_once_and_clones_preserve_budget_identity() {
    let mut original = config(LayoutPolicy::Realtime);
    assert!(original.layout_working_set_budget().is_none());
    original.ensure_layout_working_set_budget();
    let expected = original
        .layout_working_set_budget()
        .expect("root allocation")
        .clone();
    original.ensure_layout_working_set_budget();
    let mut cloned = original.clone();
    cloned.ensure_layout_working_set_budget();
    assert!(
        expected.shares_state_with(original.layout_working_set_budget().expect("original pool"))
    );
    assert!(expected.shares_state_with(cloned.layout_working_set_budget().expect("cloned pool")));

    let mut browser_config = BrowserConfig::default()
        .with_layout_policy(LayoutPolicy::Realtime)
        .with_layout_working_set_budget(expected.clone());
    let converted = NavigationRuntimeConfig::from(&browser_config.clone());
    assert!(
        expected.shares_state_with(
            converted
                .layout_working_set_budget()
                .expect("converted pool")
        )
    );
    browser_config.set_layout_working_set_budget(None);
    assert!(browser_config.layout_working_set_budget().is_none());
    assert!(expected.shares_state_with(converted.layout_working_set_budget().expect("kept clone")));
}

#[test]
fn realtime_browser_resolves_and_publishes_its_budget_before_page_creation() {
    let browser = Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))
        .expect("Realtime browser");
    let pool = browser
        .config()
        .layout_working_set_budget()
        .expect("Browser config must contain the resolved pool")
        .clone();
    let cloned_browser = browser.clone();
    let config = NavigationRuntimeConfig::from(cloned_browser.config());
    assert!(pool.shares_state_with(config.layout_working_set_budget().expect("navigation pool")));
    assert_eq!(pool.snapshot().open_pages, 0);

    let other = Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))
        .expect("independent browser");
    assert!(
        !pool.shares_state_with(
            other
                .config()
                .layout_working_set_budget()
                .expect("independent pool")
        )
    );
}

#[test]
fn realtime_navigation_replacement_config_keeps_the_resolved_budget() {
    let original = NavigationEngine::new_with_runtime_config(config(LayoutPolicy::Realtime));
    let pool = resolved_pool(&original);
    let replacement_config = original.runtime_config();
    let cloned = original.clone();
    assert!(pool.shares_state_with(&resolved_pool(&cloned)));
    drop(cloned);
    drop(original);

    let replacement = NavigationEngine::new_with_runtime_config(replacement_config);
    assert!(pool.shares_state_with(&resolved_pool(&replacement)));
    let independent = NavigationEngine::new_with_runtime_config(config(LayoutPolicy::Realtime));
    assert!(!pool.shares_state_with(&resolved_pool(&independent)));
}

#[test]
fn realtime_navigation_context_resolves_none_and_rejects_a_different_pool() {
    let original = NavigationEngine::new_with_runtime_config(config(LayoutPolicy::Realtime));
    let pool = resolved_pool(&original);
    let context_access = original.browser_context_owner_access();
    let second = NavigationEngine::new_with_runtime_config_and_browser_context_access(
        config(LayoutPolicy::Realtime),
        context_access.clone(),
    )
    .expect("None must reuse the existing context pool");
    assert!(pool.shares_state_with(&resolved_pool(&second)));
    assert!(!original.shares_renderer_owner_with(&second));

    let conflicting = config(LayoutPolicy::Realtime)
        .with_layout_working_set_budget(LayoutWorkingSetBudget::default());
    assert!(
        NavigationEngine::new_with_runtime_config_and_browser_context_access(
            conflicting,
            context_access.clone(),
        )
        .is_err()
    );
    let third = NavigationEngine::new_with_runtime_config_and_browser_context_access(
        config(LayoutPolicy::Realtime).with_layout_working_set_budget(pool.clone()),
        context_access,
    )
    .expect("rejected binding must leave the existing pool usable");
    assert!(pool.shares_state_with(&resolved_pool(&third)));
}

#[test]
fn shared_renderer_navigation_preserves_implicit_budget_and_rejects_conflicts() {
    let original = NavigationEngine::new_with_runtime_config(config(LayoutPolicy::Realtime));
    let pool = resolved_pool(&original);
    let second = NavigationEngine::new_with_fetch_config_and_shared_renderer_owner(
        FetchConfig::default(),
        &original,
        OptionalResourceFetchMask::NONE,
        true,
    )
    .expect("implicit shared renderer configuration");
    assert!(original.shares_renderer_owner_with(&second));
    assert!(pool.shares_state_with(&resolved_pool(&second)));

    assert!(
        NavigationEngine::new_with_runtime_config_and_shared_renderer_owner(
            config(LayoutPolicy::Realtime)
                .with_layout_working_set_budget(LayoutWorkingSetBudget::default()),
            &original,
        )
        .is_err()
    );
    let third = NavigationEngine::new_with_runtime_config_and_shared_renderer_owner(
        original.runtime_config(),
        &original,
    )
    .expect("rejected pool must not replace the shared renderer configuration");
    assert!(pool.shares_state_with(&resolved_pool(&third)));
}

#[test]
fn explicit_browser_budget_is_shared_across_distinct_contexts() {
    let pool = LayoutWorkingSetBudget::default();
    let browser_config = BrowserConfig::default()
        .with_layout_policy(LayoutPolicy::Realtime)
        .with_layout_working_set_budget(pool.clone());
    let first = Browser::new(browser_config.clone()).expect("first browser context");
    let second = Browser::new(browser_config).expect("second browser context");
    assert!(
        pool.shares_state_with(
            first
                .config()
                .layout_working_set_budget()
                .expect("first pool")
        )
    );
    assert!(
        pool.shares_state_with(
            second
                .config()
                .layout_working_set_budget()
                .expect("second pool")
        )
    );

    pool.set_pressure_target(0)
        .expect("explicit shared pressure");
    assert_eq!(
        first
            .config()
            .layout_working_set_budget()
            .expect("first pool")
            .snapshot()
            .pressure_target,
        0
    );
    assert_eq!(
        second
            .config()
            .layout_working_set_budget()
            .expect("second pool")
            .snapshot()
            .pressure_target,
        0
    );
}
