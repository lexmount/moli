//! Unloaded FontFaceSet members participate in demand matching without
//! registering fake font data or calling the script runtime from layout.

use read_fonts::{FontRef, TableProvider};

use super::*;

impl DocumentLayoutServices {
    /// Publishes a manual face's descriptors until its load succeeds or fails.
    /// Merely adding a face does not initiate a request.
    pub fn set_font_load_candidate(&mut self, slot: &str, face: Option<Arc<WebFontFace>>) -> bool {
        let changed = match face {
            Some(face) => {
                if self.pending_web_fonts.get(slot) == Some(&face) {
                    return false;
                }
                self.pending_web_fonts.insert(slot.to_owned(), face);
                true
            }
            None => self.pending_web_fonts.remove(slot).is_some(),
        };
        if changed {
            self.queued_font_loads.remove(slot);
            if let Some(parley) = self.parley.as_mut() {
                parley.font_load_demands.remove(slot);
            }
            self.sync_font_load_candidates();
        }
        changed
    }

    pub fn has_font_load_candidates(&self) -> bool {
        !self.pending_web_fonts.is_empty()
    }

    /// Drained only after the native pass has released its layout borrows.
    pub fn take_font_load_demands(&mut self) -> BTreeSet<String> {
        let mut demands = std::mem::take(&mut self.queued_font_loads);
        if let Some(parley) = self.parley.as_mut() {
            demands.append(&mut parley.font_load_demands);
        }
        demands
    }

    pub(super) fn sync_font_load_candidates(&mut self) -> bool {
        let Some(parley) = self.parley.as_mut() else {
            return !self.pending_web_fonts.is_empty();
        };
        let candidates = self
            .pending_web_fonts
            .iter()
            .map(|(slot, face)| {
                let order = self
                    .web_font_order
                    .get(slot)
                    .copied()
                    .unwrap_or((true, usize::MAX));
                (slot.clone(), (face.clone(), order))
            })
            .collect();
        if parley.font_load_candidates == candidates {
            return false;
        }
        parley.font_load_candidates = candidates;
        true
    }
}

struct DemandFace<'a> {
    slot: &'a str,
    capabilities: WebFontCapabilities,
    priority: (bool, usize),
    ranges: &'a [WebFontUnicodeRange],
    loaded_family: Option<&'a str>,
}

impl ParleyDocumentServices {
    pub(crate) fn request_font_loads_for_text(
        &mut self,
        style: &TextStyle<'static, 'static, TextBrush>,
        text: &str,
    ) {
        if self.font_load_candidates.is_empty() || text.is_empty() {
            return;
        }
        let families = match &style.font_family {
            FontFamily::Single(family) => vec![family.clone()],
            FontFamily::List(families) => families.to_vec(),
            FontFamily::Source(source) => FontFamilyName::parse_css_list(source)
                .filter_map(Result::ok)
                .map(FontFamilyName::into_owned)
                .collect(),
        };
        if !families.iter().any(|family| {
            let FontFamilyName::Named(name) = family else {
                return false;
            };
            let key = font_family_name_key(name);
            self.font_load_candidates
                .values()
                .any(|(face, _)| face.family_name_key() == key)
        }) {
            return;
        }
        let attributes = Attributes::new(style.font_width, style.font_style, style.font_weight);
        let requested = font_selection_capabilities(attributes);
        let mut characters = text
            .chars()
            .filter(|c| !c.is_control())
            .collect::<BTreeSet<_>>();
        for family in families {
            if characters.is_empty() {
                break;
            }
            let mut faces = Vec::new();
            if let FontFamilyName::Named(name) = &family {
                let key = font_family_name_key(name);
                if let Some(family) = self.web_font_families.get(&key) {
                    faces.extend(family.groups.iter().flat_map(|group| {
                        group.faces.iter().map(|face| DemandFace {
                            slot: &face.source_slot,
                            capabilities: group.capabilities,
                            priority: face.source_priority,
                            ranges: &face.unicode_ranges,
                            loaded_family: Some(&face.internal_family_name),
                        })
                    }));
                }
                faces.extend(
                    self.font_load_candidates
                        .iter()
                        .filter(|(_, (face, _))| face.family_name_key() == key)
                        .map(|(slot, (face, priority))| DemandFace {
                            slot,
                            capabilities: face
                                .css_capabilities()
                                .expect("validated FontFace descriptors"),
                            priority: *priority,
                            ranges: face.unicode_ranges(),
                            loaded_family: None,
                        }),
                );
            }
            if let Some(best) = faces
                .iter()
                .min_by(|a, b| a.capabilities.compare(b.capabilities, requested))
                .map(|face| face.capabilities)
            {
                // Match capabilities across the entire family before applying
                // unicode-range, just as loaded segmented faces do.
                faces.retain(|face| face.capabilities.compare(best, requested).is_eq());
            }
            faces.sort_by_key(|face| (face.priority, face.slot));
            let platform_family = match &family {
                FontFamilyName::Named(name) => self
                    .system_font_family_resolver
                    .as_mut()
                    .and_then(|resolver| {
                        resolver.substitute_family(&mut self.font_context.collection, name)
                    })
                    .map(|name| FontFamilyName::Named(Cow::Owned(name)))
                    .unwrap_or_else(|| family.clone()),
                _ => family.clone(),
            };
            characters.retain(|&character| {
                for face in faces.iter().rev().filter(|face| {
                    face.ranges.is_empty()
                        || face.ranges.iter().any(|range| range.contains(character))
                }) {
                    if let Some(name) = face.loaded_family {
                        if family_has_glyph(
                            &mut self.font_context,
                            QueryFamily::Named(name),
                            attributes,
                            character,
                        ) {
                            return false;
                        }
                    } else {
                        self.font_load_demands.insert(face.slot.to_owned());
                        return false;
                    }
                }
                let family = match &platform_family {
                    FontFamilyName::Named(name) => QueryFamily::Named(name),
                    FontFamilyName::Generic(family) => QueryFamily::Generic(*family),
                };
                !family_has_glyph(&mut self.font_context, family, attributes, character)
            });
        }
    }
}

fn family_has_glyph(
    context: &mut FontContext,
    family: QueryFamily<'_>,
    attributes: Attributes,
    character: char,
) -> bool {
    let mut query = context.collection.query(&mut context.source_cache);
    query.set_families([family]);
    query.set_attributes(attributes);
    let mut found = false;
    query.matches_with(|font| {
        found = FontRef::from_index(font.blob.as_ref(), font.index)
            .ok()
            .and_then(|font| font.cmap().ok())
            .and_then(|cmap| cmap.map_codepoint(character))
            .is_some_and(|glyph| glyph.to_u32() != 0);
        if found {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    found
}
