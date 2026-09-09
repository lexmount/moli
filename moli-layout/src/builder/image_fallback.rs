// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::{Arc, OnceLock};

use super::BoxBuilder;
use crate::{
    LayoutAnonymousReason, LayoutBoxId, LayoutBoxKind, LayoutDisplay, LayoutElementCategory,
    LayoutElementSemantics, LayoutError, LayoutImageResource, LayoutNamespace, LayoutPseudo,
    LayoutReplacedKind, LayoutSource, LayoutStyleResolver, LayoutWorld, ReplacedMetrics,
    ResolvedLayoutPseudoStyle, ResolvedLayoutStyle, replaced::ReplacedContext,
    style::ImageFallbackStyles,
};

impl<S, R> BoxBuilder<'_, S, R>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    pub(super) fn prepare_image_fallback(
        &mut self,
        source: S::NodeId,
        semantics: &mut LayoutElementSemantics,
        style: &mut ResolvedLayoutStyle,
    ) -> Result<Option<ImageFallbackStyles>, LayoutError> {
        if !semantics.is_html_element("img")
            || matches!(
                style.display(),
                LayoutDisplay::None | LayoutDisplay::Contents
            )
        {
            return Ok(None);
        }
        let Some(content) = self.source.image_fallback(source) else {
            return Ok(None);
        };
        let container = self
            .styles
            .anonymous_style(source, style, LayoutDisplay::FlowRoot)?;
        let fallback = ImageFallbackStyles::new(style, content, container);
        semantics.replaced = None;
        Ok(Some(fallback))
    }

    pub(super) fn populate_image_fallback(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        host: LayoutBoxId,
        owner: S::NodeId,
        host_style: &ResolvedLayoutStyle,
        fallback: ImageFallbackStyles,
        pseudos: (
            Option<ResolvedLayoutPseudoStyle>,
            Option<ResolvedLayoutPseudoStyle>,
        ),
    ) -> Result<(), LayoutError> {
        let label = self.source.label(owner);
        let content_style = fallback.container.as_ref().unwrap_or(host_style);
        let mut content = Vec::new();
        if fallback.show_icon {
            let mut style =
                self.styles
                    .anonymous_style(owner, content_style, LayoutDisplay::FlowRoot)?;
            style.make_broken_image_icon();
            let semantics = LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "img",
                LayoutElementCategory::Generic,
                Some(LayoutReplacedKind::Image),
            );
            let mut icon = LayoutWorld::new_box(
                None,
                Some(owner),
                None,
                format!("image-fallback({label})::icon"),
                Some(label.clone()),
                Some(semantics),
                Some(LayoutAnonymousReason::ImageFallbackContent),
                LayoutBoxKind::Replaced,
                style,
                None,
            );
            icon.replaced_context = Some(ReplacedContext::for_element(
                LayoutReplacedKind::Image,
                Some(ReplacedMetrics {
                    intrinsic_width: Some(16.0),
                    intrinsic_height: Some(16.0),
                    intrinsic_ratio: Some(1.0),
                    ..ReplacedMetrics::default()
                }),
            ));
            icon.replaced_image = Some(broken_image_icon());
            content.push(world.allocate(icon));
        }
        if let Some(text) = fallback.content.alt_text.filter(|text| !text.is_empty()) {
            let text = LayoutWorld::new_box(
                None,
                Some(owner),
                None,
                format!("image-fallback({label})::text"),
                Some(label.clone()),
                None,
                Some(LayoutAnonymousReason::ImageFallbackContent),
                LayoutBoxKind::Text,
                ResolvedLayoutStyle::text_leaf_from(content_style),
                Some(text),
            );
            content.push(world.allocate(text));
        }
        if let Some(style) = fallback.container {
            let container = world.allocate(LayoutWorld::new_box(
                None,
                Some(owner),
                None,
                format!("image-fallback({label})::container"),
                Some(label),
                None,
                Some(LayoutAnonymousReason::ImageFallbackContent),
                LayoutBoxKind::AnonymousBlock,
                style.clone(),
                None,
            ));
            self.attach_children(world, container, owner, &style, content, false)?;
            content = vec![container];
        }
        let (before, after) = pseudos;
        let mut children = self.build_pseudo(world, owner, LayoutPseudo::Before, before)?;
        children.extend(content);
        children.extend(self.build_pseudo(world, owner, LayoutPseudo::After, after)?);
        self.attach_children(world, host, owner, host_style, children, false)?;
        Ok(())
    }
}

/// Browser-owned artwork, not decoded content of the failed request. Keeping
/// it immutable and shared avoids allocations proportional to bad images.
fn broken_image_icon() -> LayoutImageResource {
    static ICON: OnceLock<Arc<moli_image::SvgImage>> = OnceLock::new();
    let svg = ICON.get_or_init(|| Arc::new(moli_image::decode_svg_image(br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
<path fill="#fff" stroke="#999" d="M1.5 1.5h9l4 4v9h-13z"/>
<path fill="#ddd" stroke="#999" d="M10.5 1.5v4h4"/>
<path fill="#6d98bb" d="M3 7h9v6H3z"/><path fill="#548647" d="m3 12 3-4 2 2 2-1 2 4H3z"/>
<path fill="none" stroke="#fff" stroke-width="1.5" d="m8 6-2 3 3 1-2 4"/>
</svg>"##).expect("built-in broken image SVG must be valid")));
    LayoutImageResource {
        intrinsic_width: 16.0,
        intrinsic_height: 16.0,
        pixels: None,
        svg: Some(svg.clone()),
    }
}
