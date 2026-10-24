use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use crate::{css_resource_urls::StylesheetLoadBlockingResource, document_runtime::DomHandle};

use style::{stylesheets::Origin, stylist::Stylist};

use super::{
    active_stylesheets::ActiveStylesheetCollection, shadow_scopes::ShadowScopeStyles,
    stylesheet::native_effective_font_face_rule_addresses,
};

#[cfg(test)]
thread_local! {
    static STYLESHEET_RESOURCE_MANIFEST_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_stylesheet_resource_manifest_build_count_for_test() {
    STYLESHEET_RESOURCE_MANIFEST_BUILD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn stylesheet_resource_manifest_build_count_for_test() -> usize {
    STYLESHEET_RESOURCE_MANIFEST_BUILD_COUNT.with(std::cell::Cell::get)
}

/// Identity of one typed stylesheet resource set published by a Document.
///
/// The revision advances only when the font/import manifest changes, not for
/// unrelated style-world flushes. Keeping the Document in the type prevents a
/// layout-owned sidecar from treating equal revisions from two iframe worlds
/// as interchangeable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StylesheetResourceGeneration {
    document: DomHandle,
    revision: u64,
}

impl StylesheetResourceGeneration {
    pub(super) const fn new(document: DomHandle, revision: u64) -> Self {
        Self { document, revision }
    }
}

/// Owned, typed resource projection from one current retained style world.
#[derive(Clone, Debug)]
pub(crate) struct StylesheetResourceSnapshot {
    generation: StylesheetResourceGeneration,
    web_fonts: Arc<[StylesheetLoadBlockingResource]>,
    imports: Arc<[url::Url]>,
}

impl StylesheetResourceSnapshot {
    pub(super) fn new(
        generation: StylesheetResourceGeneration,
        web_fonts: Arc<[StylesheetLoadBlockingResource]>,
        imports: Arc<[url::Url]>,
    ) -> Self {
        Self {
            generation,
            web_fonts,
            imports,
        }
    }

    pub(crate) const fn generation(&self) -> StylesheetResourceGeneration {
        self.generation
    }

    pub(crate) fn web_fonts(&self) -> &[StylesheetLoadBlockingResource] {
        self.web_fonts.as_ref()
    }

    pub(crate) fn imports(&self) -> &[url::Url] {
        self.imports.as_ref()
    }
}

/// Resource metadata retained beside the exact parsed stylesheet collections.
/// It contains no layout, paint, or network state.
#[derive(PartialEq)]
pub(super) struct StylesheetResourceManifest {
    web_fonts: Arc<[StylesheetLoadBlockingResource]>,
    imports: Arc<[url::Url]>,
}

impl StylesheetResourceManifest {
    pub(super) fn from_active_stylesheets(
        stylist: &Stylist,
        document_stylesheets: &ActiveStylesheetCollection,
        shadow_scopes: &[ShadowScopeStyles],
    ) -> Self {
        #[cfg(test)]
        STYLESHEET_RESOURCE_MANIFEST_BUILD_COUNT.with(|count| count.set(count.get() + 1));
        let mut web_fonts = Vec::new();
        let mut imports = BTreeMap::new();
        // Stylo has already resolved effective rules and sorted them by the
        // document-wide layer order. Source order alone loses cross-sheet and
        // empty @layer statements, even though those can change font priority.
        let document_font_order = stylist
            .iter_extra_data_origins()
            .filter(|(_, origin)| *origin == Origin::Author)
            .flat_map(|(data, _)| data.font_faces.iter())
            .enumerate()
            .map(|(index, (rule, _))| (rule.raw_ptr().as_ptr() as usize, index))
            .collect::<HashMap<_, _>>();
        collect_resources(
            document_stylesheets,
            stylist.device(),
            stylist
                .cascade_data()
                .borrow_for_origin(Origin::Author)
                .custom_media_map(),
            Some(&document_font_order),
            &mut web_fonts,
            &mut imports,
        );
        for scope in shadow_scopes {
            collect_resources(
                scope.active_stylesheets(),
                stylist.device(),
                scope.author_styles().data.custom_media_map(),
                None,
                &mut web_fonts,
                &mut imports,
            );
        }
        // A declaration may be adopted by more than one scope. Keep its last
        // occurrence, while independent rules retain independent registrations.
        let mut seen = BTreeSet::new();
        web_fonts.reverse();
        web_fonts.retain(|resource| {
            seen.insert(
                resource
                    .web_font()
                    .expect("font resource")
                    .registration_slot()
                    .to_owned(),
            )
        });
        web_fonts.reverse();
        Self {
            web_fonts: web_fonts.into(),
            imports: imports.into_values().collect::<Vec<_>>().into(),
        }
    }

    pub(super) fn snapshot(
        &self,
        generation: StylesheetResourceGeneration,
    ) -> StylesheetResourceSnapshot {
        StylesheetResourceSnapshot::new(
            generation,
            Arc::clone(&self.web_fonts),
            Arc::clone(&self.imports),
        )
    }
}

fn collect_resources(
    stylesheets: &ActiveStylesheetCollection,
    device: &style::device::Device,
    custom_media: &style::stylesheets::CustomMediaMap,
    cascade_order: Option<&HashMap<usize, usize>>,
    web_fonts: &mut Vec<StylesheetLoadBlockingResource>,
    imports: &mut BTreeMap<String, url::Url>,
) {
    let mut fonts = Vec::new();
    for stylesheet in stylesheets.entries() {
        let effective_font_faces = native_effective_font_face_rule_addresses(
            stylesheet.stylesheet(),
            device,
            custom_media,
        );
        for resource in stylesheet.web_font_resources() {
            if !effective_font_faces.contains(&resource.rule_address()) {
                continue;
            }
            let order = cascade_order
                .and_then(|order| order.get(&resource.rule_address()))
                .copied()
                .unwrap_or(usize::MAX);
            let resource = resource.resource();
            if resource.web_font().is_some() {
                fonts.push((order, resource.clone()));
            }
        }
        for import in stylesheet.import_urls() {
            let mut identity = import.clone();
            identity.set_fragment(None);
            imports
                .entry(identity.as_str().to_owned())
                .or_insert_with(|| import.clone());
        }
    }
    if cascade_order.is_some() {
        fonts.sort_by_key(|(order, _)| *order);
    }
    web_fonts.extend(fonts.into_iter().map(|(_, resource)| resource));
}
