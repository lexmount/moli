use std::path::{Path, PathBuf};

use moli_fetch::FetchConfig;
use moli_page_types::{LayoutPolicy, OptionalResourceFetchMask, SubresourceResourceType};

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    document_start_scripts: Vec<String>,
    fetch: FetchConfig,
    profile_dir: Option<PathBuf>,
    layout_policy: LayoutPolicy,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
    subframe_loading_enabled: bool,
    script_execution_disabled: bool,
    author_styles_disabled: bool,
    wpt_extensions_enabled: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            document_start_scripts: Vec::new(),
            fetch: FetchConfig::default(),
            profile_dir: None,
            layout_policy: LayoutPolicy::default(),
            optional_resource_fetch_mask: OptionalResourceFetchMask::NONE,
            subframe_loading_enabled: true,
            script_execution_disabled: false,
            author_styles_disabled: false,
            wpt_extensions_enabled: false,
        }
    }
}

impl BrowserConfig {
    pub fn document_start_scripts(&self) -> &[String] {
        &self.document_start_scripts
    }

    pub fn add_document_start_script(&mut self, source: impl Into<String>) {
        self.document_start_scripts.push(source.into());
    }

    pub fn with_document_start_script(mut self, source: impl Into<String>) -> Self {
        self.add_document_start_script(source);
        self
    }

    pub fn fetch(&self) -> &FetchConfig {
        &self.fetch
    }

    pub fn fetch_mut(&mut self) -> &mut FetchConfig {
        &mut self.fetch
    }

    pub fn profile_dir(&self) -> Option<&Path> {
        self.profile_dir.as_deref()
    }

    pub fn set_profile_dir(&mut self, profile_dir: Option<PathBuf>) {
        self.profile_dir = profile_dir;
    }

    pub fn layout_policy(&self) -> LayoutPolicy {
        self.layout_policy
    }

    pub fn set_layout_policy(&mut self, policy: LayoutPolicy) {
        self.layout_policy = policy;
    }

    pub fn with_layout_policy(mut self, policy: LayoutPolicy) -> Self {
        self.set_layout_policy(policy);
        self
    }

    pub fn image_fetch_enabled(&self) -> bool {
        self.optional_resource_fetch_enabled(SubresourceResourceType::Image)
    }

    pub fn set_image_fetch_enabled(&mut self, enabled: bool) {
        self.set_optional_resource_fetch_enabled(SubresourceResourceType::Image, enabled);
    }

    pub fn with_image_fetch_enabled(mut self, enabled: bool) -> Self {
        self.set_image_fetch_enabled(enabled);
        self
    }

    pub fn optional_resource_fetch_mask(&self) -> OptionalResourceFetchMask {
        self.optional_resource_fetch_mask
    }

    pub fn set_optional_resource_fetch_mask(&mut self, mask: OptionalResourceFetchMask) {
        self.optional_resource_fetch_mask = mask;
    }

    pub fn with_optional_resource_fetch_mask(mut self, mask: OptionalResourceFetchMask) -> Self {
        self.set_optional_resource_fetch_mask(mask);
        self
    }

    pub fn optional_resource_fetch_enabled(&self, resource_type: SubresourceResourceType) -> bool {
        self.optional_resource_fetch_mask.allows(resource_type)
    }

    pub fn set_optional_resource_fetch_enabled(
        &mut self,
        resource_type: SubresourceResourceType,
        enabled: bool,
    ) {
        let Some(resource) = OptionalResourceFetchMask::for_resource_type(resource_type) else {
            return;
        };
        self.optional_resource_fetch_mask.set(resource, enabled);
    }

    pub fn subframe_loading_enabled(&self) -> bool {
        self.subframe_loading_enabled
    }

    pub fn set_subframe_loading_enabled(&mut self, enabled: bool) {
        self.subframe_loading_enabled = enabled;
    }

    pub fn with_subframe_loading_enabled(mut self, enabled: bool) -> Self {
        self.set_subframe_loading_enabled(enabled);
        self
    }

    pub fn script_execution_disabled(&self) -> bool {
        self.script_execution_disabled
    }

    pub fn set_script_execution_disabled(&mut self, disabled: bool) {
        self.script_execution_disabled = disabled;
    }

    pub fn author_styles_disabled(&self) -> bool {
        self.author_styles_disabled
    }

    pub fn set_author_styles_disabled(&mut self, disabled: bool) {
        self.author_styles_disabled = disabled;
    }

    pub fn wpt_extensions_enabled(&self) -> bool {
        self.wpt_extensions_enabled
    }

    pub fn set_wpt_extensions_enabled(&mut self, enabled: bool) {
        self.wpt_extensions_enabled = enabled;
    }

    pub fn with_wpt_extensions_enabled(mut self, enabled: bool) -> Self {
        self.set_wpt_extensions_enabled(enabled);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONAL_RESOURCES: [(SubresourceResourceType, OptionalResourceFetchMask); 6] = [
        (
            SubresourceResourceType::Image,
            OptionalResourceFetchMask::IMAGE,
        ),
        (
            SubresourceResourceType::Font,
            OptionalResourceFetchMask::FONT,
        ),
        (
            SubresourceResourceType::Audio,
            OptionalResourceFetchMask::AUDIO,
        ),
        (
            SubresourceResourceType::Video,
            OptionalResourceFetchMask::VIDEO,
        ),
        (
            SubresourceResourceType::Media,
            OptionalResourceFetchMask::MEDIA,
        ),
        (
            SubresourceResourceType::TextTrack,
            OptionalResourceFetchMask::TEXT_TRACK,
        ),
    ];

    #[test]
    fn browser_config_defaults_layout_to_mock_and_can_select_on_demand() {
        let mut config = BrowserConfig::default();
        assert_eq!(config.layout_policy(), LayoutPolicy::Mock);

        config.set_layout_policy(LayoutPolicy::OnDemand);
        assert_eq!(config.layout_policy(), LayoutPolicy::OnDemand);

        let config = config.with_layout_policy(LayoutPolicy::Mock);
        assert_eq!(config.layout_policy(), LayoutPolicy::Mock);
    }

    #[test]
    fn browser_config_disables_every_optional_resource_family_by_default() {
        let config = BrowserConfig::default();

        assert_eq!(
            config.optional_resource_fetch_mask(),
            OptionalResourceFetchMask::NONE
        );
        for (resource_type, _) in OPTIONAL_RESOURCES {
            assert!(
                !config.optional_resource_fetch_enabled(resource_type),
                "{resource_type:?} must require an explicit opt-in"
            );
        }
        for resource_type in [
            SubresourceResourceType::Script,
            SubresourceResourceType::Stylesheet,
            SubresourceResourceType::Fetch,
            SubresourceResourceType::Xhr,
        ] {
            assert!(
                config.optional_resource_fetch_enabled(resource_type),
                "{resource_type:?} is outside the optional-resource policy"
            );
        }
    }

    #[test]
    fn browser_config_defaults_to_script_execution_and_can_disable_it() {
        let mut config = BrowserConfig::default();
        assert!(!config.script_execution_disabled());

        config.set_script_execution_disabled(true);
        assert!(config.script_execution_disabled());
    }

    #[test]
    fn browser_config_defaults_to_author_styles_and_can_disable_them() {
        let mut config = BrowserConfig::default();
        assert!(!config.author_styles_disabled());

        config.set_author_styles_disabled(true);
        assert!(config.author_styles_disabled());
    }

    #[test]
    fn browser_config_toggles_each_optional_resource_bit_independently() {
        for (enabled_type, enabled_bit) in OPTIONAL_RESOURCES {
            let mut config = BrowserConfig::default();
            config.set_optional_resource_fetch_enabled(enabled_type, true);

            assert_eq!(config.optional_resource_fetch_mask(), enabled_bit);
            for (observed_type, _) in OPTIONAL_RESOURCES {
                assert_eq!(
                    config.optional_resource_fetch_enabled(observed_type),
                    observed_type == enabled_type,
                    "enabling {enabled_type:?} changed {observed_type:?}"
                );
            }

            config.set_optional_resource_fetch_enabled(enabled_type, false);
            assert_eq!(
                config.optional_resource_fetch_mask(),
                OptionalResourceFetchMask::NONE
            );
        }
    }

    #[test]
    fn legacy_image_switch_changes_only_the_image_bit() {
        let preserved = OptionalResourceFetchMask::FONT | OptionalResourceFetchMask::TEXT_TRACK;
        let mut config = BrowserConfig::default().with_optional_resource_fetch_mask(preserved);

        config.set_image_fetch_enabled(true);
        assert_eq!(
            config.optional_resource_fetch_mask(),
            preserved | OptionalResourceFetchMask::IMAGE
        );
        assert!(config.image_fetch_enabled());

        config.set_image_fetch_enabled(false);
        assert_eq!(config.optional_resource_fetch_mask(), preserved);
        assert!(!config.image_fetch_enabled());
    }

    #[test]
    fn browser_config_ignores_non_optional_resource_mutations() {
        let mut config = BrowserConfig::default()
            .with_optional_resource_fetch_mask(OptionalResourceFetchMask::ALL);

        config.set_optional_resource_fetch_enabled(SubresourceResourceType::Script, false);

        assert_eq!(
            config.optional_resource_fetch_mask(),
            OptionalResourceFetchMask::ALL
        );
        assert!(
            config.optional_resource_fetch_enabled(SubresourceResourceType::Script),
            "required resource families cannot be disabled through the optional mask"
        );
    }
}
