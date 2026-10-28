//! Normalize authored frames before grouping them into property tracks. Values
//! remain specified CSS, so em/var()/currentColor still resolve at sample time.
use moli_animation::{KeyframeValue, PropertyKeyframes};

pub(crate) struct KeyframeDeclarations(Vec<(String, String)>);

impl KeyframeDeclarations {
    pub(crate) fn from_script(mut properties: Vec<(String, String)>) -> Self {
        use std::cmp::Reverse;
        use style::properties::PropertyId;
        // JS keyframes give longhands precedence over shorthands, independent
        // of object enumeration order. CSS declarations keep their own order.
        properties.sort_by_cached_key(|(name, _)| {
            let count = PropertyId::parse_enabled_for_all_content(name)
                .ok()
                .and_then(|id| id.as_shorthand().ok())
                .map_or(0, |id| id.longhands().count());
            (Reverse(count), name.clone())
        });
        Self::from_css(properties)
    }

    pub(crate) fn from_css(properties: Vec<(String, String)>) -> Self {
        let mut block = moli_css_parse::CssDeclarationBlock::default();
        for (name, value) in properties {
            block.set_property(&name, &value, false);
        }
        Self(
            block
                .into_inner()
                .declaration_importance_iter()
                .filter(|(declaration, _)| declaration.id().is_animatable())
                .map(|(declaration, _)| {
                    let mut value = style_traits::CssString::new();
                    let _ = declaration.to_css(&mut value);
                    (declaration.id().name().into_owned(), value)
                })
                .collect(),
        )
    }
}

impl IntoIterator for KeyframeDeclarations {
    type Item = (String, String);
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Default)]
pub(crate) struct KeyframeTrackBuilder(Vec<PropertyKeyframes>);
impl KeyframeTrackBuilder {
    pub(crate) fn push_frame(
        &mut self,
        offset: f64,
        easing: Option<String>,
        declarations: KeyframeDeclarations,
    ) {
        for (property, value) in declarations {
            let index = self
                .0
                .iter()
                .position(|track| track.property == property)
                .unwrap_or_else(|| {
                    self.0.push(PropertyKeyframes {
                        property,
                        values: Vec::new(),
                    });
                    self.0.len() - 1
                });
            self.0[index].values.push(KeyframeValue {
                offset,
                value: Some(value),
                easing: easing.clone(),
            });
        }
    }

    pub(crate) fn finish(mut self) -> Vec<PropertyKeyframes> {
        for property in &mut self.0 {
            // Stable sorting preserves distinct authored frames at equal offsets.
            property
                .values
                .sort_by(|a, b| a.offset.total_cmp(&b.offset));
        }
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn declarations(css: bool, entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        let entries = entries
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        (if css {
            KeyframeDeclarations::from_css(entries)
        } else {
            KeyframeDeclarations::from_script(entries)
        })
        .into_iter()
        .collect()
    }

    #[test]
    fn script_longhands_win_in_both_property_orders_while_css_keeps_declaration_order() {
        let entries = [("margin-left", "20px"), ("margin", "0px")];
        assert_eq!(declarations(false, &entries)["margin-left"], "20px");
        assert_eq!(
            declarations(false, &[entries[1], entries[0]])["margin-left"],
            "20px"
        );
        assert_eq!(declarations(true, &entries)["margin-left"], "0px");
        assert_eq!(
            declarations(true, &[entries[1], entries[0]])["margin-left"],
            "20px"
        );
    }

    #[test]
    fn invalid_values_do_not_create_keyframes_or_erase_the_remaining_interval() {
        let mut tracks = KeyframeTrackBuilder::default();
        for (offset, value) in [(0.0, "0px"), (0.5, "bogus"), (1.0, "100px")] {
            tracks.push_frame(
                offset,
                None,
                KeyframeDeclarations::from_script(vec![("width".into(), value.into())]),
            );
        }
        let tracks = tracks.finish();
        assert_eq!(tracks.len(), 1);
        assert_eq!(
            tracks[0]
                .values
                .iter()
                .map(|v| v.offset)
                .collect::<Vec<_>>(),
            vec![0.0, 1.0]
        );
    }

    #[test]
    fn equal_offsets_preserve_authored_frame_identity() {
        let mut tracks = KeyframeTrackBuilder::default();
        for (offset, value) in [(0.0, "0"), (0.0, ".4"), (1.0, "1")] {
            tracks.push_frame(
                offset,
                None,
                KeyframeDeclarations::from_script(vec![("opacity".into(), value.into())]),
            );
        }
        let tracks = tracks.finish();
        assert_eq!(tracks[0].values.len(), 3);
        assert_eq!(tracks[0].values[0].value.as_deref(), Some("0"));
        assert_eq!(tracks[0].values[1].value.as_deref(), Some("0.4"));
    }

    #[test]
    fn normalization_retains_environment_dependent_specified_values() {
        let values = declarations(false, &[("width", "2em"), ("height", "var(--height)")]);
        assert_eq!(values["width"], "2em");
        assert_eq!(values["height"], "var(--height)");
    }
}
