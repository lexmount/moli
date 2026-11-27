use std::{collections::HashSet, rc::Rc};

use style::{
    author_styles::AuthorStyles,
    invalidation::stylesheets::StylesheetInvalidationSet,
    shared_lock::{SharedRwLock, SharedRwLockReadGuard},
    stylesheets::{
        CustomMediaMap, DocumentStyleSheet, StylesheetInDocument, scope_rule::ImplicitScopeRoot,
    },
    stylist::Stylist,
};

use crate::document_runtime::DomHandle;

use super::{
    active_stylesheets::{
        ActiveStylesheet, ActiveStylesheetCollection, ActiveStylesheetReconciliation,
        StylesheetSetUpdate, note_exact_rule_change_notifications,
        note_full_cascade_update_fallback, stylesheet_set_updates,
    },
    source::store::{LiveStylesheetCascadeUpdate, StyloStylesheetSource},
    state::RetainedStyleSystem,
};

/// Persistent author-style state for one connected ShadowRoot TreeScope.
/// Empty scopes deliberately retain an empty `AuthorStyles`, matching Blink's
/// stable TreeScope universe and Stylo's shared empty CascadeData semantics.
pub(super) struct ShadowScopeStyles {
    root: DomHandle,
    active_stylesheets: ActiveStylesheetCollection,
    author_styles: AuthorStyles<DocumentStyleSheet>,
    implicit_scope_roots: Rc<[Option<ImplicitScopeRoot>]>,
    flush_count: u64,
}

impl ShadowScopeStyles {
    pub(super) fn new(
        root: DomHandle,
        active_stylesheets: ActiveStylesheetCollection,
        author_styles: AuthorStyles<DocumentStyleSheet>,
    ) -> Self {
        Self {
            root,
            active_stylesheets,
            implicit_scope_roots: implicit_scope_roots(&author_styles),
            author_styles,
            flush_count: 1,
        }
    }

    pub(super) fn root(&self) -> DomHandle {
        self.root
    }

    pub(super) fn active_stylesheets(&self) -> &ActiveStylesheetCollection {
        &self.active_stylesheets
    }

    pub(super) fn active_stylesheets_mut(&mut self) -> &mut ActiveStylesheetCollection {
        &mut self.active_stylesheets
    }

    pub(super) fn author_styles(&self) -> &AuthorStyles<DocumentStyleSheet> {
        &self.author_styles
    }

    pub(super) fn author_styles_mut(&mut self) -> &mut AuthorStyles<DocumentStyleSheet> {
        &mut self.author_styles
    }

    pub(super) fn implicit_scope_roots(&self) -> Rc<[Option<ImplicitScopeRoot>]> {
        Rc::clone(&self.implicit_scope_roots)
    }

    pub(super) fn flush(
        &mut self,
        stylist: &mut Stylist,
        guard: &SharedRwLockReadGuard<'_>,
    ) -> StylesheetInvalidationSet {
        self.flush_count = self.flush_count.saturating_add(1);
        self.implicit_scope_roots = implicit_scope_roots(&self.author_styles);
        self.author_styles.flush(stylist, guard)
    }

    #[cfg(test)]
    pub(super) fn flush_count_for_test(&self) -> u64 {
        self.flush_count
    }
}

fn implicit_scope_roots(
    author_styles: &AuthorStyles<DocumentStyleSheet>,
) -> Rc<[Option<ImplicitScopeRoot>]> {
    // Keep the indices used by Stylo's stylesheet set, including sheets
    // whose media conditions currently do not match.
    let sheets = &author_styles.stylesheets;
    (0..sheets.len())
        .map(|index| {
            sheets
                .get(index)
                .and_then(StylesheetInDocument::implicit_scope_root)
        })
        .collect()
}

pub(super) struct ShadowScopeReconciliation {
    pub(super) invalidations: Vec<(DomHandle, StylesheetInvalidationSet)>,
    pub(super) removed_roots: Vec<DomHandle>,
    pub(super) scope_fallbacks: Vec<DomHandle>,
    pub(super) collections_changed: bool,
}

/// Reconciles exactly the dirty ShadowRoot collections plus an optional
/// connected-root membership snapshot. Clean `AuthorStyles` stay in place and
/// are not flushed.
pub(super) fn reconcile_dirty_shadow_scopes(
    retained: &mut RetainedStyleSystem,
    shared_lock: &SharedRwLock,
    dirty_scopes: &[(DomHandle, Vec<StyloStylesheetSource>)],
    connected_roots: Option<&[DomHandle]>,
    device_changed: bool,
    mut install: impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) -> ShadowScopeReconciliation {
    let mut invalidations = Vec::new();
    let mut removed_roots = Vec::new();
    let mut scope_fallbacks = Vec::new();
    let mut device_affected_roots = Vec::new();
    let mut collections_changed = false;
    let mut newly_built_roots = HashSet::new();

    if let Some(connected_roots) = connected_roots {
        let mut previous_scopes = std::mem::take(&mut retained.shadow_scopes);
        let mut next_scopes = Vec::with_capacity(connected_roots.len());
        for root in connected_roots {
            if let Some(index) = previous_scopes
                .iter()
                .position(|scope| scope.root() == *root)
            {
                next_scopes.push(previous_scopes.remove(index));
                continue;
            }
            let sources = dirty_scopes
                .iter()
                .find_map(|(candidate, sources)| (candidate == root).then_some(sources.as_slice()))
                .unwrap_or_default();
            let (scope, scope_invalidations) = build_shadow_scope(
                &mut retained.stylist,
                shared_lock,
                *root,
                sources,
                &mut install,
            );
            newly_built_roots.insert(*root);
            invalidations.push((*root, scope_invalidations));
            next_scopes.push(scope);
            collections_changed = true;
        }
        removed_roots.extend(previous_scopes.into_iter().map(|scope| scope.root()));
        collections_changed |= !removed_roots.is_empty();
        retained.shadow_scopes = next_scopes;
    }

    if device_changed {
        let guard = shared_lock.read();
        device_affected_roots.extend(
            retained
                .shadow_scopes
                .iter()
                .filter(|scope| !newly_built_roots.contains(&scope.root()))
                .filter(|scope| shadow_scope_media_changed(scope, &retained.stylist, &guard))
                .map(ShadowScopeStyles::root),
        );
    }

    for (root, sources) in dirty_scopes {
        if newly_built_roots.contains(root) {
            continue;
        }
        let Some(scope) = retained
            .shadow_scopes
            .iter_mut()
            .find(|scope| scope.root() == *root)
        else {
            continue;
        };
        let reconciliation = scope
            .active_stylesheets_mut()
            .reconcile(sources, &mut install);
        if let Some(reconciliation) = reconciliation.as_ref() {
            collections_changed = true;
            if apply_stylesheet_set_reconciliation(
                scope,
                &retained.stylist,
                shared_lock,
                reconciliation,
            ) {
                scope_fallbacks.push(*root);
            }
        }
    }

    let dirty_roots = dirty_scopes
        .iter()
        .map(|(root, _)| *root)
        .collect::<HashSet<_>>();
    for scope in &mut retained.shadow_scopes {
        if newly_built_roots.contains(&scope.root()) {
            continue;
        }
        let device_affected = device_affected_roots.contains(&scope.root());
        let must_flush = dirty_roots.contains(&scope.root()) || device_affected;
        if !must_flush {
            continue;
        }
        if device_affected {
            scope.author_styles_mut().stylesheets.force_dirty();
        }
        let guard = shared_lock.read();
        invalidations.push((scope.root(), scope.flush(&mut retained.stylist, &guard)));
    }

    publish_shadow_cascade_data(retained);
    ShadowScopeReconciliation {
        invalidations,
        removed_roots,
        scope_fallbacks,
        collections_changed,
    }
}

/// Full-snapshot compatibility path used by initial construction tests and
/// exceptional callers. It still preserves existing scopes by identity.
pub(super) fn reconcile_shadow_scopes(
    retained: &mut RetainedStyleSystem,
    shared_lock: &SharedRwLock,
    desired_scopes: &[(DomHandle, Vec<StyloStylesheetSource>)],
    device_changed: bool,
    mut install: impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) -> ShadowScopeReconciliation {
    let mut previous_scopes = std::mem::take(&mut retained.shadow_scopes);
    let mut next_scopes = Vec::with_capacity(desired_scopes.len());
    let mut invalidations = Vec::new();
    let mut scope_fallbacks = Vec::new();
    let mut collections_changed = false;
    for (root, sources) in desired_scopes {
        let (mut scope, retained_scope) = if let Some(index) = previous_scopes
            .iter()
            .position(|scope| scope.root() == *root)
        {
            (previous_scopes.remove(index), true)
        } else {
            collections_changed = true;
            let (scope, scope_invalidations) = build_shadow_scope(
                &mut retained.stylist,
                shared_lock,
                *root,
                sources,
                &mut install,
            );
            invalidations.push((*root, scope_invalidations));
            (scope, false)
        };

        let device_affected = device_changed && retained_scope && {
            let guard = shared_lock.read();
            shadow_scope_media_changed(&scope, &retained.stylist, &guard)
        };

        let reconciliation = scope
            .active_stylesheets_mut()
            .reconcile(sources, &mut install);
        if let Some(reconciliation) = reconciliation.as_ref() {
            collections_changed = true;
            if apply_stylesheet_set_reconciliation(
                &mut scope,
                &retained.stylist,
                shared_lock,
                reconciliation,
            ) {
                scope_fallbacks.push(*root);
            }
        }
        if device_affected {
            scope.author_styles_mut().stylesheets.force_dirty();
        }
        if reconciliation.is_some() || device_affected {
            let guard = shared_lock.read();
            invalidations.push((*root, scope.flush(&mut retained.stylist, &guard)));
        }
        next_scopes.push(scope);
    }
    retained.shadow_scopes = next_scopes;
    publish_shadow_cascade_data(retained);
    let removed_roots = previous_scopes
        .into_iter()
        .map(|scope| scope.root())
        .collect::<Vec<_>>();
    collections_changed |= !removed_roots.is_empty();
    ShadowScopeReconciliation {
        invalidations,
        removed_roots,
        scope_fallbacks,
        collections_changed,
    }
}

fn shadow_scope_media_changed(
    scope: &ShadowScopeStyles,
    stylist: &Stylist,
    guard: &SharedRwLockReadGuard<'_>,
) -> bool {
    scope.active_stylesheets().entries().iter().any(|entry| {
        !scope.author_styles().data.media_feature_affected_matches(
            entry.stylesheet(),
            guard,
            stylist.device(),
            stylist.quirks_mode(),
        )
    })
}

fn build_shadow_scope(
    stylist: &mut Stylist,
    shared_lock: &SharedRwLock,
    root: DomHandle,
    sources: &[StyloStylesheetSource],
    install: &mut impl FnMut(&StyloStylesheetSource) -> ActiveStylesheet,
) -> (ShadowScopeStyles, StylesheetInvalidationSet) {
    let active_stylesheets = ActiveStylesheetCollection::new(sources.iter().map(install).collect());
    let mut author_styles = AuthorStyles::<DocumentStyleSheet>::new();
    let custom_media = CustomMediaMap::default();
    let guard = shared_lock.read();
    for stylesheet in active_stylesheets.cascade_stylesheets() {
        author_styles.stylesheets.append_stylesheet(
            Some(stylist.device()),
            &custom_media,
            stylesheet,
            &guard,
        );
    }
    let invalidations = author_styles.flush(stylist, &guard);
    (
        ShadowScopeStyles::new(root, active_stylesheets, author_styles),
        invalidations,
    )
}

fn apply_stylesheet_set_reconciliation(
    scope: &mut ShadowScopeStyles,
    stylist: &Stylist,
    shared_lock: &SharedRwLock,
    reconciliation: &ActiveStylesheetReconciliation,
) -> bool {
    let custom_media = CustomMediaMap::default();
    let guard = shared_lock.read();
    if reconciliation.stylesheet_set_changed() {
        let next_stylesheets = scope.active_stylesheets().cascade_stylesheets();
        update_shadow_stylesheet_set(
            scope.author_styles_mut(),
            stylist.device(),
            &custom_media,
            reconciliation.previous_stylesheets(),
            &next_stylesheets,
            &guard,
        );
    }
    let full_cascade_fallback = notify_shadow_stylesheet_rule_changes(
        scope.author_styles_mut(),
        stylist.device(),
        &custom_media,
        reconciliation,
        &guard,
    );
    if full_cascade_fallback {
        scope.author_styles_mut().stylesheets.force_dirty();
    }
    reconciliation.stylesheet_removed() || full_cascade_fallback
}

fn update_shadow_stylesheet_set(
    author_styles: &mut AuthorStyles<DocumentStyleSheet>,
    device: &style::device::Device,
    custom_media: &CustomMediaMap,
    previous: &[DocumentStyleSheet],
    next: &[DocumentStyleSheet],
    guard: &SharedRwLockReadGuard<'_>,
) {
    for update in stylesheet_set_updates(previous, next) {
        match update {
            StylesheetSetUpdate::Remove(stylesheet) => author_styles.stylesheets.remove_stylesheet(
                Some(device),
                custom_media,
                stylesheet,
                guard,
            ),
            StylesheetSetUpdate::InsertBefore { stylesheet, before } => author_styles
                .stylesheets
                .insert_stylesheet_before(Some(device), custom_media, stylesheet, before, guard),
            StylesheetSetUpdate::Append(stylesheet) => author_styles.stylesheets.append_stylesheet(
                Some(device),
                custom_media,
                stylesheet,
                guard,
            ),
        }
    }
}

fn notify_shadow_stylesheet_rule_changes(
    author_styles: &mut AuthorStyles<DocumentStyleSheet>,
    device: &style::device::Device,
    custom_media: &CustomMediaMap,
    reconciliation: &ActiveStylesheetReconciliation,
    guard: &SharedRwLockReadGuard<'_>,
) -> bool {
    if reconciliation
        .in_place_updates()
        .iter()
        .any(|update| matches!(update.cascade_update(), LiveStylesheetCascadeUpdate::Full))
    {
        note_full_cascade_update_fallback();
        return true;
    }
    for update in reconciliation.in_place_updates() {
        let LiveStylesheetCascadeUpdate::Rules(changes) = update.cascade_update() else {
            unreachable!("full updates were handled above");
        };
        note_exact_rule_change_notifications(changes.len());
        for change in changes {
            let ancestors = change
                .ancestors()
                .iter()
                .map(style::stylesheets::CssRuleRef::from)
                .collect::<Vec<_>>();
            author_styles.stylesheets.rule_changed(
                Some(device),
                custom_media,
                update.stylesheet(),
                change.rule(),
                guard,
                change.change_kind(),
                &ancestors,
            );
        }
    }
    false
}

fn publish_shadow_cascade_data(retained: &mut RetainedStyleSystem) {
    retained.shadow_cascade_data = retained
        .shadow_scopes
        .iter()
        .map(|scope| (scope.root(), scope.author_styles().data.clone()))
        .collect();
}
