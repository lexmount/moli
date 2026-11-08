use crate::window_surface::{
    DEFAULT_ACCEPT_LANGUAGE, DEFAULT_NAVIGATOR_PLATFORM, DEFAULT_SEC_CH_UA_ARCH,
    DEFAULT_SEC_CH_UA_BITNESS, DEFAULT_SEC_CH_UA_FORM_FACTORS, DEFAULT_SEC_CH_UA_MODEL,
    DEFAULT_SEC_CH_UA_PLATFORM, DEFAULT_SEC_CH_UA_PLATFORM_VERSION, DEFAULT_SEC_CH_UA_WOW64,
    DEFAULT_USER_AGENT, chromium_full_version, chromium_major_version, chromium_ua_brand_versions,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserBrandVersion {
    pub brand: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserUserAgentMetadataOverride {
    pub brands: Option<Vec<BrowserBrandVersion>>,
    pub full_version_list: Option<Vec<BrowserBrandVersion>>,
    pub full_version: Option<String>,
    pub platform: String,
    pub platform_version: String,
    pub architecture: String,
    pub model: String,
    pub mobile: bool,
    pub bitness: Option<String>,
    pub wow64: Option<bool>,
    pub form_factors: Option<Vec<String>>,
}

impl From<(String, String)> for BrowserBrandVersion {
    fn from((brand, version): (String, String)) -> Self {
        Self { brand, version }
    }
}

/// One coherent browser-context identity used by network and JavaScript APIs.
///
/// A user-agent override replaces the derived Chromium version and brand data
/// at the same boundary as the UA string. An override without a Chromium
/// product token intentionally yields empty brand lists rather than retaining
/// stale default Chrome metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserIdentityProfile {
    user_agent: String,
    accept_language: String,
    navigator_platform: String,
    major_version: String,
    full_version: String,
    brands: Vec<BrowserBrandVersion>,
    full_version_list: Vec<BrowserBrandVersion>,
    mobile: bool,
    platform: String,
    platform_version: String,
    architecture: String,
    bitness: String,
    model: String,
    wow64: bool,
    form_factors: Vec<String>,
    languages: Vec<String>,
}

impl BrowserIdentityProfile {
    pub fn new(user_agent: impl Into<String>, accept_language: impl Into<String>) -> Self {
        let user_agent = user_agent.into();
        let accept_language = accept_language.into();
        let brands: Vec<BrowserBrandVersion> = chromium_ua_brand_versions(&user_agent, false)
            .unwrap_or_default()
            .into_iter()
            .map(BrowserBrandVersion::from)
            .collect();
        let full_version_list = chromium_ua_brand_versions(&user_agent, true)
            .unwrap_or_default()
            .into_iter()
            .map(BrowserBrandVersion::from)
            .collect();
        let has_user_agent_metadata = !brands.is_empty();
        let (platform, platform_version, architecture, bitness, model, form_factors) =
            if has_user_agent_metadata {
                (
                    structured_header_string_value(DEFAULT_SEC_CH_UA_PLATFORM),
                    structured_header_string_value(DEFAULT_SEC_CH_UA_PLATFORM_VERSION),
                    structured_header_string_value(DEFAULT_SEC_CH_UA_ARCH),
                    structured_header_string_value(DEFAULT_SEC_CH_UA_BITNESS),
                    structured_header_string_value(DEFAULT_SEC_CH_UA_MODEL),
                    DEFAULT_SEC_CH_UA_FORM_FACTORS
                        .split(',')
                        .map(structured_header_string_value)
                        .filter(|value| !value.is_empty())
                        .collect(),
                )
            } else {
                Default::default()
            };

        Self {
            major_version: chromium_major_version(&user_agent)
                .unwrap_or_default()
                .to_owned(),
            full_version: chromium_full_version(&user_agent)
                .unwrap_or_default()
                .to_owned(),
            brands,
            full_version_list,
            mobile: false,
            platform,
            platform_version,
            architecture,
            bitness,
            model,
            wow64: has_user_agent_metadata && DEFAULT_SEC_CH_UA_WOW64 == "?1",
            form_factors,
            languages: parse_accept_language(&accept_language),
            user_agent,
            accept_language,
            navigator_platform: DEFAULT_NAVIGATOR_PLATFORM.to_owned(),
        }
    }

    /// Builds the coherent identity selected by CDP `setUserAgentOverride`.
    ///
    /// Chromium does not infer Client Hint metadata from the replacement UA.
    /// Without `userAgentMetadata`, `navigator.userAgentData` remains exposed
    /// with empty brands/platform and no `Sec-CH-UA-*` values are generated.
    /// Omitted values fall back to the target's normal, non-overridden profile.
    pub fn from_devtools_override(
        base: &Self,
        user_agent: impl Into<String>,
        accept_language: Option<String>,
        navigator_platform: Option<String>,
        metadata: Option<BrowserUserAgentMetadataOverride>,
    ) -> Self {
        let user_agent = user_agent.into();
        let user_agent_override_active = !user_agent.is_empty();
        let user_agent = if user_agent_override_active {
            user_agent
        } else {
            base.user_agent.clone()
        };
        let (accept_language, languages) = match accept_language {
            Some(accept_language) if !accept_language.is_empty() => {
                let languages = accept_language
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect();
                (accept_language, languages)
            }
            Some(_) | None => (base.accept_language.clone(), base.languages.clone()),
        };
        let navigator_platform = navigator_platform
            .filter(|platform| !platform.is_empty())
            .unwrap_or_else(|| base.navigator_platform.clone());
        let (
            brands,
            full_version_list,
            full_version,
            mobile,
            platform,
            platform_version,
            architecture,
            bitness,
            model,
            wow64,
            form_factors,
        ) = match (user_agent_override_active, metadata) {
            (true, Some(metadata)) => {
                let full_version = metadata
                    .full_version
                    .unwrap_or_else(|| base.full_version.clone());
                (
                    metadata.brands.unwrap_or_else(|| base.brands.clone()),
                    metadata
                        .full_version_list
                        .unwrap_or_else(|| base.full_version_list.clone()),
                    full_version,
                    metadata.mobile,
                    metadata.platform,
                    metadata.platform_version,
                    metadata.architecture,
                    metadata.bitness.unwrap_or_else(|| base.bitness.clone()),
                    metadata.model,
                    metadata.wow64.unwrap_or(base.wow64),
                    metadata
                        .form_factors
                        .unwrap_or_else(|| base.form_factors.clone()),
                )
            }
            (true, None) => (
                Vec::new(),
                Vec::new(),
                String::new(),
                false,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                false,
                Vec::new(),
            ),
            // An empty CDP userAgent clears the override. Chromium then
            // exposes the target's natural UA metadata again; the protocol
            // rejects metadata supplied together with an empty UA before this
            // constructor is reached.
            (false, _) => (
                base.brands.clone(),
                base.full_version_list.clone(),
                base.full_version.clone(),
                base.mobile,
                base.platform.clone(),
                base.platform_version.clone(),
                base.architecture.clone(),
                base.bitness.clone(),
                base.model.clone(),
                base.wow64,
                base.form_factors.clone(),
            ),
        };
        let major_version = preferred_brand_version(&brands)
            .or_else(|| full_version.split('.').next().map(str::to_owned))
            .unwrap_or_default();
        Self {
            user_agent,
            accept_language,
            navigator_platform,
            major_version,
            full_version,
            brands,
            full_version_list,
            mobile,
            platform,
            platform_version,
            architecture,
            bitness,
            model,
            wow64,
            form_factors,
            languages,
        }
    }

    pub fn with_accept_language(&self, accept_language: impl Into<String>) -> Self {
        let accept_language = accept_language.into();
        let mut identity = self.clone();
        identity.languages = parse_accept_language(&accept_language);
        identity.accept_language = accept_language;
        identity
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub fn accept_language(&self) -> &str {
        &self.accept_language
    }

    pub fn navigator_platform(&self) -> &str {
        &self.navigator_platform
    }

    pub fn major_version(&self) -> &str {
        &self.major_version
    }

    pub fn full_version(&self) -> &str {
        &self.full_version
    }

    pub fn brands(&self) -> &[BrowserBrandVersion] {
        &self.brands
    }

    pub fn full_version_list(&self) -> &[BrowserBrandVersion] {
        &self.full_version_list
    }

    pub fn mobile(&self) -> bool {
        self.mobile
    }

    pub fn platform(&self) -> &str {
        &self.platform
    }

    pub fn platform_version(&self) -> &str {
        &self.platform_version
    }

    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    pub fn bitness(&self) -> &str {
        &self.bitness
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn wow64(&self) -> bool {
        self.wow64
    }

    pub fn form_factors(&self) -> &[String] {
        &self.form_factors
    }

    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    pub fn language(&self) -> &str {
        self.languages.first().map(String::as_str).unwrap_or("")
    }

    pub fn sec_ch_ua_value(&self) -> Option<String> {
        format_brand_list(&self.brands)
    }

    pub fn sec_ch_ua_full_version_list_value(&self) -> Option<String> {
        format_brand_list(&self.full_version_list)
    }

    pub fn has_user_agent_metadata(&self) -> bool {
        !self.brands.is_empty()
    }
}

impl Default for BrowserIdentityProfile {
    fn default() -> Self {
        Self::new(DEFAULT_USER_AGENT, DEFAULT_ACCEPT_LANGUAGE)
    }
}

pub fn parse_accept_language(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|entry| {
            let language = entry.split(';').next()?.trim();
            (!language.is_empty()).then(|| language.to_owned())
        })
        .collect()
}

fn structured_header_string_value(value: &str) -> String {
    value.trim().trim_matches('"').to_owned()
}

fn format_brand_list(brands: &[BrowserBrandVersion]) -> Option<String> {
    (!brands.is_empty()).then(|| {
        brands
            .iter()
            .map(|entry| {
                format!(
                    "{};v={}",
                    quoted_structured_header_string(&entry.brand),
                    quoted_structured_header_string(&entry.version)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn quoted_structured_header_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn preferred_brand_version(brands: &[BrowserBrandVersion]) -> Option<String> {
    brands
        .iter()
        .find(|entry| {
            let brand = entry.brand.to_ascii_lowercase();
            brand.contains("chromium") || brand.contains("chrome")
        })
        .or_else(|| brands.first())
        .map(|entry| entry.version.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_identity_keeps_network_and_js_brand_order_together() {
        let identity = BrowserIdentityProfile::default();

        assert_eq!(identity.major_version(), "156");
        assert_eq!(identity.full_version(), "156.0.8078.74");
        assert_eq!(identity.platform(), "Windows");
        assert_eq!(identity.navigator_platform(), "Win32");
        assert_eq!(identity.languages(), ["en-US", "en"]);
        assert_eq!(
            identity.sec_ch_ua_value().as_deref(),
            Some("\"Not:A-Brand\";v=\"8\", \"Chromium\";v=\"156\", \"Google Chrome\";v=\"156\"")
        );
        assert_eq!(
            identity
                .brands()
                .iter()
                .map(|entry| entry.brand.as_str())
                .collect::<Vec<_>>(),
            ["Not:A-Brand", "Chromium", "Google Chrome"]
        );
    }

    #[test]
    fn non_chromium_user_agent_override_does_not_retain_stale_brands() {
        let identity = BrowserIdentityProfile::new("CustomAgent/1.0", "fr-FR,fr;q=0.8");

        assert!(identity.brands().is_empty());
        assert!(identity.full_version_list().is_empty());
        assert_eq!(identity.sec_ch_ua_value(), None);
        assert_eq!(identity.platform(), "");
        assert!(identity.form_factors().is_empty());
        assert_eq!(identity.languages(), ["fr-FR", "fr"]);
    }

    #[test]
    fn devtools_override_without_metadata_does_not_infer_client_hints() {
        let identity = BrowserIdentityProfile::from_devtools_override(
            &BrowserIdentityProfile::default(),
            "CustomAgent/1.0",
            Some("fr-CA,fr;q=0.9".to_owned()),
            Some("Linux x86_64".to_owned()),
            None,
        );

        assert_eq!(identity.user_agent(), "CustomAgent/1.0");
        assert_eq!(identity.navigator_platform(), "Linux x86_64");
        assert_eq!(identity.languages(), ["fr-CA", "fr;q=0.9"]);
        assert!(identity.brands().is_empty());
        assert_eq!(identity.platform(), "");
        assert_eq!(identity.sec_ch_ua_value(), None);
    }

    #[test]
    fn empty_devtools_override_restores_the_natural_identity() {
        let base = BrowserIdentityProfile::default();
        let identity = BrowserIdentityProfile::from_devtools_override(
            &base,
            "",
            Some(String::new()),
            Some(String::new()),
            None,
        );

        assert_eq!(identity, base);
    }

    #[test]
    fn devtools_override_uses_explicit_client_hint_metadata() {
        let identity = BrowserIdentityProfile::from_devtools_override(
            &BrowserIdentityProfile::default(),
            "LinuxChrome/145",
            None,
            Some("Linux x86_64".to_owned()),
            Some(BrowserUserAgentMetadataOverride {
                brands: Some(vec![BrowserBrandVersion {
                    brand: "Chromium".to_owned(),
                    version: "145".to_owned(),
                }]),
                full_version_list: Some(vec![BrowserBrandVersion {
                    brand: "Chromium".to_owned(),
                    version: "145.0.7632.116".to_owned(),
                }]),
                full_version: Some("145.0.9000.1".to_owned()),
                platform: "Linux".to_owned(),
                platform_version: String::new(),
                architecture: "x86".to_owned(),
                model: String::new(),
                mobile: false,
                bitness: Some("64".to_owned()),
                wow64: Some(false),
                form_factors: Some(vec!["Desktop".to_owned()]),
            }),
        );

        assert_eq!(identity.navigator_platform(), "Linux x86_64");
        assert_eq!(identity.platform(), "Linux");
        assert_eq!(identity.full_version(), "145.0.9000.1");
        assert_eq!(identity.major_version(), "145");
        assert_eq!(identity.form_factors(), ["Desktop"]);
    }

    #[test]
    fn devtools_brand_metadata_is_escaped_as_a_structured_header_string() {
        let identity = BrowserIdentityProfile::from_devtools_override(
            &BrowserIdentityProfile::default(),
            "CustomAgent/1.0",
            None,
            None,
            Some(BrowserUserAgentMetadataOverride {
                brands: Some(vec![BrowserBrandVersion {
                    brand: "Quoted\"Brand\\Name".to_owned(),
                    version: "1\"2\\3".to_owned(),
                }]),
                full_version_list: None,
                full_version: None,
                platform: String::new(),
                platform_version: String::new(),
                architecture: String::new(),
                model: String::new(),
                mobile: false,
                bitness: None,
                wow64: None,
                form_factors: None,
            }),
        );

        assert_eq!(
            identity.sec_ch_ua_value().as_deref(),
            Some(r#""Quoted\"Brand\\Name";v="1\"2\\3""#)
        );
    }
}
