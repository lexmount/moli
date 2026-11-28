use moli_layout::{
    LayoutAnonymousStyleContext, LayoutDisplay, LayoutError, LayoutStyleResolver, LayoutViewport,
    ResolvedLayoutElementStyles, ResolvedLayoutPseudoStyle, ResolvedLayoutStyle,
};
use std::time::Instant;
use style::{
    matching::StyleChange, properties::ComputedValues, servo::restyle_damage::ServoRestyleDamage,
};

use crate::{
    document_runtime::DomHandle,
    dom_presentation::suppresses_noscript_box,
    native_bridge::{PageScriptHost, element::StyleObservation},
    style_engine::{
        ScreenStyleCoverage, StyleResolutionPass, StyleViewport, StyloAnonymousBoxKind,
        StyloStyleEnvironment,
    },
};

pub(super) struct NativeLayoutStyleResolver<'a, 'world, 'dom> {
    runtime: &'a PageScriptHost,
    pass: &'a mut StyleResolutionPass<'world, 'dom>,
    scripting_enabled: bool,
    screen_style_coverage: Option<&'a mut ScreenStyleCoverage>,
    profile: Option<NativeLayoutStyleResolverProfile>,
}

#[derive(Default)]
struct NativeLayoutStyleResolverProfile {
    primary_count: u64,
    primary_read_ns: u128,
    primary_projection_ns: u128,
    primary_policy_ns: u128,
    eager_pseudo_count: u64,
    eager_pseudo_projection_ns: u128,
    marker_count: u64,
    marker_read_ns: u128,
    marker_projection_ns: u128,
    anonymous_count: u64,
    anonymous_read_ns: u128,
    anonymous_projection_ns: u128,
}

pub(super) fn with_layout_styles<R>(
    runtime: &PageScriptHost,
    document: DomHandle,
    viewport: LayoutViewport,
    environment_override: Option<StyloStyleEnvironment>,
    mut screen_style_coverage: Option<&mut ScreenStyleCoverage>,
    prepare: impl FnOnce(&mut NativeLayoutStyleResolver<'_, '_, '_>) -> R,
) -> R {
    let mut observation =
        layout_style_read_scope(runtime, document, viewport, environment_override);
    observation.with_resolution_pass(document, |pass| {
        if let Some(coverage) = screen_style_coverage.as_deref_mut() {
            *coverage = pass.screen_style_coverage();
        }
        let mut resolver = NativeLayoutStyleResolver {
            runtime,
            pass,
            scripting_enabled: runtime.document_scripting_enabled(document),
            screen_style_coverage,
            profile: moli_trace::cpu_profile_enabled()
                .then(NativeLayoutStyleResolverProfile::default),
        };
        let result = prepare(&mut resolver);
        resolver.trace_profile(document);
        result
    })
}

impl NativeLayoutStyleResolver<'_, '_, '_> {
    fn trace_profile(&self, document: DomHandle) {
        let Some(profile) = self.profile.as_ref() else {
            return;
        };
        self.pass.trace_profile();
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "layout_style_resolver",
            document = document.index_u32(),
            primary_count = profile.primary_count,
            primary_read_us = profile.primary_read_ns / 1_000,
            primary_projection_us = profile.primary_projection_ns / 1_000,
            primary_policy_us = profile.primary_policy_ns / 1_000,
            eager_pseudo_count = profile.eager_pseudo_count,
            eager_pseudo_projection_us = profile.eager_pseudo_projection_ns / 1_000,
            marker_count = profile.marker_count,
            marker_read_us = profile.marker_read_ns / 1_000,
            marker_projection_us = profile.marker_projection_ns / 1_000,
            anonymous_count = profile.anonymous_count,
            anonymous_read_us = profile.anonymous_read_ns / 1_000,
            anonymous_projection_us = profile.anonymous_projection_ns / 1_000,
        );
    }
}

pub(super) fn layout_style_read_scope<'a>(
    runtime: &'a PageScriptHost,
    document: DomHandle,
    viewport: LayoutViewport,
    environment_override: Option<StyloStyleEnvironment>,
) -> StyleObservation<'a> {
    let viewport = layout_style_viewport(runtime, viewport);
    if environment_override.is_none()
        && document == runtime.document_handle()
        && viewport == runtime.style_viewport()
    {
        // Keep the normal main-document observation environment so layout and
        // rendered-text collection read the same persistent style world. Only
        // an override viewport/media or embedded Document needs an explicit context.
        StyleObservation::new(runtime)
    } else {
        StyleObservation::new_for_document_viewport(
            runtime,
            document,
            viewport,
            environment_override,
        )
    }
}

fn layout_style_viewport(runtime: &PageScriptHost, viewport: LayoutViewport) -> StyleViewport {
    let screen = runtime.style_viewport();
    StyleViewport::new(
        Some(f64::from(viewport.css_width)),
        Some(f64::from(viewport.css_height)),
    )
    .with_screen_size(screen.screen_width, screen.screen_height)
    .with_device_pixel_ratio(screen.device_pixel_ratio)
}

impl LayoutStyleResolver<DomHandle> for NativeLayoutStyleResolver<'_, '_, '_> {
    fn can_reuse_element_style(&mut self, node: DomHandle, previous: &ResolvedLayoutStyle) -> bool {
        // Pending damage schedules the check. Intermediate computed styles can
        // accumulate damage even when the published box already matches.
        self.pass.element_styles(node).is_some_and(|styles| {
            let (primary, before, after) = styles.into_element_computed_values();
            before.is_none()
                && after.is_none()
                && previous
                    .stylo_computed_values()
                    .is_some_and(|previous| same_layout_style(previous, &primary))
                && !suppresses_noscript_box(self.runtime.dom_host(), node, self.scripting_enabled)
        })
    }

    fn element_styles(
        &mut self,
        node: DomHandle,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let Some(style) = self.pass.element_styles(node) else {
            if let Some(profile) = self.profile.as_mut() {
                profile.primary_count = profile.primary_count.saturating_add(1);
                profile.primary_read_ns = profile.primary_read_ns.saturating_add(
                    phase_started
                        .map(|started| started.elapsed().as_nanos())
                        .unwrap_or_default(),
                );
            }
            return Ok(None);
        };
        let (computed, before, after) = style.into_element_computed_values();
        let eager_pseudo_count = u64::from(before.is_some()) + u64::from(after.is_some());
        if let Some(profile) = self.profile.as_mut() {
            profile.primary_count = profile.primary_count.saturating_add(1);
            profile.primary_read_ns = profile.primary_read_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let mut resolved = ResolvedLayoutStyle::from_stylo(computed);
        if let Some(profile) = self.profile.as_mut() {
            profile.primary_projection_ns = profile.primary_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let before = before.map(ResolvedLayoutStyle::from_stylo);
        let after = after.map(ResolvedLayoutStyle::from_stylo);
        if let Some(profile) = self.profile.as_mut() {
            profile.eager_pseudo_count = profile
                .eager_pseudo_count
                .saturating_add(eager_pseudo_count);
            profile.eager_pseudo_projection_ns = profile.eager_pseudo_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        if suppresses_noscript_box(self.runtime.dom_host(), node, self.scripting_enabled) {
            // Match Blink's HTMLNoScriptElement::LayoutObjectIsNeeded(): the
            // computed `display` remains observable, including author
            // overrides, but no layout object is generated while this exact
            // Document can execute scripts.
            resolved.force_display_none();
        }
        if let Some(profile) = self.profile.as_mut() {
            profile.primary_policy_ns = profile.primary_policy_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        Ok(Some(ResolvedLayoutElementStyles::new(
            resolved, before, after,
        )))
    }

    fn marker_style(
        &mut self,
        node: DomHandle,
    ) -> Result<Option<ResolvedLayoutPseudoStyle>, LayoutError> {
        if let Some(coverage) = &mut self.screen_style_coverage {
            coverage.lazy_pseudo_owners.insert(node);
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let computed = self.pass.marker_for_generation(node);
        if let Some(profile) = self.profile.as_mut() {
            profile.marker_count = profile.marker_count.saturating_add(1);
            profile.marker_read_ns = profile.marker_read_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let resolved = computed.map(|computed| {
            ResolvedLayoutPseudoStyle::new(ResolvedLayoutStyle::from_stylo(computed))
        });
        if let Some(profile) = self.profile.as_mut() {
            profile.marker_projection_ns = profile.marker_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        Ok(resolved)
    }

    fn can_reuse_marker_style(
        &self,
        previous: &ResolvedLayoutStyle,
        current: &ResolvedLayoutStyle,
    ) -> bool {
        previous
            .stylo_computed_values()
            .zip(current.stylo_computed_values())
            .is_some_and(|(previous, current)| same_layout_style(previous, current))
    }

    fn placeholder_style(
        &mut self,
        node: DomHandle,
    ) -> Result<Option<ResolvedLayoutPseudoStyle>, LayoutError> {
        if let Some(coverage) = &mut self.screen_style_coverage {
            coverage.lazy_pseudo_owners.insert(node);
        }
        Ok(self.pass.pseudo_style(node, "placeholder").map(|computed| {
            ResolvedLayoutPseudoStyle::new(ResolvedLayoutStyle::from_stylo(computed))
        }))
    }

    fn anonymous_style(
        &mut self,
        context: LayoutAnonymousStyleContext<'_, DomHandle>,
        display: LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        let owner = context.owner();
        let parent = context.parent_style();
        let parent_computed = parent.stylo_computed_values().ok_or_else(|| {
            LayoutError::style_resolution(
                format!("{owner:?}"),
                "native anonymous box parent has no retained Stylo ComputedValues",
            )
        })?;
        let anonymous_kind = anonymous_box_kind(display);
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let computed = self
            .pass
            .anonymous_style(owner, parent_computed.as_ref(), anonymous_kind)
            .ok_or_else(|| {
                LayoutError::style_resolution(
                    format!("{owner:?}"),
                    "anonymous owner document does not match the prepared style context",
                )
            })?;
        if let Some(profile) = self.profile.as_mut() {
            profile.anonymous_count = profile.anonymous_count.saturating_add(1);
            profile.anonymous_read_ns = profile.anonymous_read_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let mut resolved = ResolvedLayoutStyle::from_stylo(computed);
        resolved.force_layout_display(display);
        if let Some(profile) = self.profile.as_mut() {
            profile.anonymous_projection_ns = profile.anonymous_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        Ok(resolved)
    }
}

fn same_layout_style(previous: &ComputedValues, current: &ComputedValues) -> bool {
    if std::ptr::eq(previous, current) {
        return true;
    }
    if previous.effective_zoom != current.effective_zoom
        || previous.writing_mode != current.writing_mode
        || previous.flags != current.flags
        || match (previous.visited_style(), current.visited_style()) {
            (None, None) => false,
            (Some(previous), Some(current)) => !std::ptr::eq(previous, current),
            _ => true,
        }
    {
        return false;
    }
    // A DOM move can re-resolve an unchanged style into a new allocation.
    let difference = ServoRestyleDamage::compute_style_difference::<moli_selector::StyloElement<'_>>(
        previous, current,
    );
    difference.damage.is_empty() && matches!(difference.change, StyleChange::Unchanged)
}

const fn anonymous_box_kind(display: LayoutDisplay) -> StyloAnonymousBoxKind {
    match display {
        LayoutDisplay::Table | LayoutDisplay::InlineTable => StyloAnonymousBoxKind::Table,
        LayoutDisplay::TableRow => StyloAnonymousBoxKind::TableRow,
        LayoutDisplay::TableCell => StyloAnonymousBoxKind::TableCell,
        LayoutDisplay::None
        | LayoutDisplay::Contents
        | LayoutDisplay::Block
        | LayoutDisplay::FlowRoot
        | LayoutDisplay::Inline
        | LayoutDisplay::InlineBlock
        | LayoutDisplay::Flex
        | LayoutDisplay::InlineFlex
        | LayoutDisplay::Grid
        | LayoutDisplay::InlineGrid
        | LayoutDisplay::BlockListItem
        | LayoutDisplay::InlineListItem
        | LayoutDisplay::TableCaption
        | LayoutDisplay::TableRowGroup
        | LayoutDisplay::TableHeaderGroup
        | LayoutDisplay::TableFooterGroup
        | LayoutDisplay::TableColumnGroup
        | LayoutDisplay::TableColumn => StyloAnonymousBoxKind::Generic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style_engine::take_style_resolution_context_count_for_test;
    use moli_layout::LayoutDisplay;

    fn fixture() -> crate::script_vm::StandaloneScriptVmHarness {
        use crate::{
            dom::native::DomHost, parser::HtmlParser, script_vm::ScriptVmDefaultWorldBootstrap,
        };
        let _runtime = crate::JsRuntime::initialize();
        let document = HtmlParser::SCRIPTING_ENABLED.parse(
            url::Url::parse("https://anonymous-style.test/").unwrap(),
            "<!doctype html><style>html,body{margin:0}#owner{font-size:23px;color:red}#other{font-size:41px;color:blue}</style><div id=owner>one</div><div id=other>two</div><iframe id=frame></iframe>".into(),
        );
        let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let mut vm = ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
            DomHost::from_dom(document),
            queue.owner_attached_runtime_page_task_sender_for_test(),
            queue.parser_boundary_sender(),
        )
        .unwrap()
        .finish()
        .unwrap();
        vm.install_page_task_residence_for_executor_test(queue.residence());
        vm.set_layout_policy(crate::real_layout_test_policy());
        vm
    }

    fn layout_world(
        resolver: &mut NativeLayoutStyleResolver<'_, '_, '_>,
        root: DomHandle,
    ) -> moli_layout::LayoutWorld<DomHandle> {
        let source = super::super::source_view::NativeLayoutSourceView::new(resolver.runtime, root);
        moli_layout::build_layout_world(&source, resolver).unwrap()
    }

    fn parent_context(
        world: &moli_layout::LayoutWorld<DomHandle>,
        owner: DomHandle,
    ) -> LayoutAnonymousStyleContext<'_, DomHandle> {
        let parent = world.box_by_id(world.source_box(owner).unwrap()).unwrap();
        LayoutAnonymousStyleContext::from_parent(parent).unwrap()
    }

    #[test]
    fn anonymous_styles_reuse_one_prepared_observation() {
        let vm = fixture();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let root = host.dom_host().document_element_handle().unwrap();
        let owner = host.dom_host().element_handle_by_id("owner").unwrap();
        let viewport = host.layout_viewport_for_document(document);
        take_style_resolution_context_count_for_test();
        with_layout_styles(&host, document, viewport, None, None, |resolver| {
            assert_eq!(take_style_resolution_context_count_for_test(), 0);
            let world = layout_world(resolver, root);
            let parent = parent_context(&world, owner);
            assert_eq!(take_style_resolution_context_count_for_test(), 1);
            let resolutions = resolver.pass.resolutions_for_test();
            for _ in 0..3 {
                resolver.profile = Some(NativeLayoutStyleResolverProfile::default());
                for _ in 0..10_000 {
                    let style = resolver
                        .anonymous_style(parent, LayoutDisplay::Block)
                        .unwrap();
                    assert_eq!(style.display(), LayoutDisplay::Block);
                    std::hint::black_box(style);
                }
                let profile = resolver.profile.as_ref().unwrap();
                assert_eq!(profile.anonymous_count, 10_000);
                assert_eq!(
                    resolver.pass.resolutions_for_test(),
                    resolutions,
                    "anonymous boxes must reuse the parent box style"
                );
                eprintln!(
                    "anonymous_count={} anonymous_read_ns={} anonymous_projection_ns={}",
                    profile.anonymous_count,
                    profile.anonymous_read_ns,
                    profile.anonymous_projection_ns
                );
            }
        });
    }

    #[test]
    fn anonymous_styles_reject_foreign_documents() {
        let mut vm = fixture();
        vm.eval("document.getElementById('frame').contentDocument.body.textContent = 'child'")
            .unwrap();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let frame = host.dom_host().element_handle_by_id("frame").unwrap();
        let foreign_document = host.child_browsing_context_document_handle(frame).unwrap();
        let foreign_root = host
            .dom_host()
            .dom()
            .document_element_handle_for_document(foreign_document)
            .unwrap();
        let foreign_viewport = host.layout_viewport_for_document(foreign_document);
        let foreign_world = with_layout_styles(
            &host,
            foreign_document,
            foreign_viewport,
            None,
            None,
            |foreign| {
                let world = layout_world(foreign, foreign_root);
                let parent = parent_context(&world, foreign_root);
                assert!(
                    foreign
                        .anonymous_style(parent, LayoutDisplay::Block)
                        .is_ok()
                );
                world
            },
        );
        let foreign_parent = parent_context(&foreign_world, foreign_root);
        with_layout_styles(
            &host,
            document,
            host.layout_viewport_for_document(document),
            None,
            None,
            |resolver| {
                let error = resolver
                    .anonymous_style(foreign_parent, LayoutDisplay::Block)
                    .unwrap_err();
                assert!(error.to_string().contains("owner document"), "{error}");
                assert!(resolver.pass.element_styles(foreign_root).is_none());
            },
        );
    }

    #[test]
    fn anonymous_table_styles_inherit_from_their_parent_boxes() {
        let mut vm = fixture();
        vm.eval(
            r#"document.getElementById('owner').innerHTML = '<span style="display:table-cell">one</span><span style="display:table-row">two</span>'"#,
        )
        .unwrap();
        vm.sync_live_document_style_sources();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let root = host.dom_host().document_element_handle().unwrap();
        let owner = host.dom_host().element_handle_by_id("owner").unwrap();
        with_layout_styles(
            &host,
            document,
            host.layout_viewport_for_document(document),
            None,
            None,
            |resolver| {
                let world = layout_world(resolver, root);
                let reads = resolver.pass.resolutions_for_test();
                let mut pending = vec![world.source_box(owner).unwrap()];
                let mut displays = Vec::new();
                while let Some(id) = pending.pop() {
                    let parent = world.box_by_id(id).unwrap();
                    pending.extend_from_slice(parent.children());
                    if parent.anonymous_reason().is_none() {
                        continue;
                    }
                    let context = LayoutAnonymousStyleContext::from_parent(parent).unwrap();
                    assert_eq!(context.owner(), parent.owner().unwrap());
                    assert_eq!(context.parent_style().font_size(), 23.0);
                    let anonymous = resolver
                        .anonymous_style(context, LayoutDisplay::Block)
                        .unwrap();
                    assert_eq!(anonymous.font_size(), 23.0);
                    displays.push(parent.style().display());
                }
                for display in [
                    LayoutDisplay::Table,
                    LayoutDisplay::TableRowGroup,
                    LayoutDisplay::TableRow,
                    LayoutDisplay::TableCell,
                ] {
                    assert!(
                        displays.contains(&display),
                        "missing {display:?}: {displays:?}"
                    );
                }
                assert_eq!(resolver.pass.resolutions_for_test(), reads);
            },
        );
    }

    #[test]
    fn anonymous_styles_prepare_pending_invalidations_for_each_pass() {
        let mut vm = fixture();
        vm.sync_live_document_style_sources();
        for (mutation, expected) in [
            (None, 23.0),
            (
                Some("document.getElementById('owner').style.fontSize = '37px'"),
                37.0,
            ),
            (
                Some(
                    "document.querySelector('style').textContent += '#owner {font-size:51px!important}'",
                ),
                51.0,
            ),
        ] {
            if let Some(mutation) = mutation {
                vm.eval(mutation).unwrap();
                vm.sync_live_document_style_sources();
            }
            let host = vm.context_host_weak_for_test().upgrade().unwrap();
            let host = host.borrow();
            let document = host.document_handle();
            let root = host.dom_host().document_element_handle().unwrap();
            let owner = host.dom_host().element_handle_by_id("owner").unwrap();
            with_layout_styles(
                &host,
                document,
                host.layout_viewport_for_document(document),
                None,
                None,
                |resolver| {
                    let world = layout_world(resolver, root);
                    let parent = parent_context(&world, owner);
                    assert_eq!(parent.parent_style().font_size(), expected);
                    let resolutions = resolver.pass.resolutions_for_test();
                    let anonymous = resolver
                        .anonymous_style(parent, LayoutDisplay::Block)
                        .unwrap();
                    assert_eq!(anonymous.font_size(), expected);
                    assert_eq!(resolver.pass.resolutions_for_test(), resolutions);
                },
            );
        }
    }

    #[test]
    fn layout_style_pass_recovers_ancestor_filters_across_branches_and_results() {
        let mut vm = fixture();
        vm.eval(
            r#"
            document.querySelector('style').textContent +=
              '#owner section .target { font-size:31px } #other .target { font-size:47px } input::placeholder { color:green }';
            document.getElementById('owner').innerHTML =
              '<section><div style="display:contents"><span id="deep" class="target">A</span></div></section>';
            document.getElementById('other').innerHTML =
              '<span id="shallow" class="target">B</span><input id="input" placeholder="hint">';
            "#,
        )
        .unwrap();
        vm.sync_live_document_style_sources();
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let document = host.document_handle();
        let viewport = host.layout_viewport_for_document(document);
        let owner = host.dom_host().element_handle_by_id("owner").unwrap();
        let deep = host.dom_host().element_handle_by_id("deep").unwrap();
        let shallow = host.dom_host().element_handle_by_id("shallow").unwrap();
        let input = host.dom_host().element_handle_by_id("input").unwrap();

        take_style_resolution_context_count_for_test();
        let interrupted = with_layout_styles(&host, document, viewport, None, None, |resolver| {
            assert!(resolver.pass.element_styles(owner).is_some());
            Err::<(), _>("stop layout preparation")
        });
        assert_eq!(interrupted, Err("stop layout preparation"));
        assert_eq!(take_style_resolution_context_count_for_test(), 1);

        for contexts in [1, 0] {
            with_layout_styles(&host, document, viewport, None, None, |resolver| {
                for (target, expected) in [(shallow, "47px"), (deep, "31px"), (shallow, "47px")] {
                    let style = resolver.pass.element_styles(target).unwrap();
                    assert_eq!(style.property_value("font-size").as_deref(), Some(expected));
                }
                assert!(resolver.placeholder_style(input).unwrap().is_some());
            });
            assert_eq!(take_style_resolution_context_count_for_test(), contexts);
        }
    }

    #[test]
    fn anonymous_table_roles_use_the_matching_servo_precomputed_pseudo() {
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::Table),
            StyloAnonymousBoxKind::Table
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::TableRow),
            StyloAnonymousBoxKind::TableRow
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::TableCell),
            StyloAnonymousBoxKind::TableCell
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::TableRowGroup),
            StyloAnonymousBoxKind::Generic
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::Block),
            StyloAnonymousBoxKind::Generic
        );
    }
}
