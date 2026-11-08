#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSurfaceProfile {
    pub user_agent: &'static str,
    pub platform: &'static str,
    pub language: &'static str,
    pub hardware_concurrency: f64,
    pub max_touch_points: f64,
    pub inner_width: f64,
    pub inner_height: f64,
    pub device_pixel_ratio: f64,
    pub screen_width: f64,
    pub screen_height: f64,
    pub screen_avail_width: f64,
    pub screen_avail_height: f64,
    pub color_depth: f64,
    pub pixel_depth: f64,
    pub orientation_angle: f64,
    pub orientation_type: &'static str,
    pub visual_viewport_scale: f64,
}

/// Chromium-compatible product token exposed through `Browser.getVersion`.
///
/// Keep this token in the default user agent as well. CDP clients commonly use
/// `product` for Chromium feature detection, while Moli's own product
/// identity remains available through its binary/package metadata.
pub const DEFAULT_CDP_PRODUCT: &str = "Chrome/156.0.8078.74";
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.8078.74 Safari/537.36";
pub const DEFAULT_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
pub const DEFAULT_SEC_CH_UA_PLATFORM: &str = "\"Windows\"";
pub const DEFAULT_SEC_CH_UA_PLATFORM_VERSION: &str = "\"19.0.0\"";
pub const DEFAULT_SEC_CH_UA_ARCH: &str = "\"x86\"";
pub const DEFAULT_SEC_CH_UA_BITNESS: &str = "\"64\"";
pub const DEFAULT_SEC_CH_UA_MODEL: &str = "\"\"";
pub const DEFAULT_SEC_CH_UA_WOW64: &str = "?0";
pub const DEFAULT_SEC_CH_UA_FORM_FACTORS: &str = "\"Desktop\"";
pub const DEFAULT_NAVIGATOR_PLATFORM: &str = "Win32";
pub const DEFAULT_NAVIGATOR_APP_CODE_NAME: &str = "Mozilla";
pub const DEFAULT_NAVIGATOR_APP_NAME: &str = "Netscape";
pub const DEFAULT_NAVIGATOR_VENDOR: &str = "Google Inc.";
pub const DEFAULT_NAVIGATOR_VENDOR_SUB: &str = "";
pub const DEFAULT_NAVIGATOR_PRODUCT: &str = "Gecko";
pub const DEFAULT_NAVIGATOR_PRODUCT_SUB: &str = "20030107";
pub const DEFAULT_NAVIGATOR_ONLINE: bool = true;
pub const DEFAULT_NAVIGATOR_WEBDRIVER: bool = false;
pub const DEFAULT_NAVIGATOR_PDF_VIEWER_ENABLED: bool = true;
pub const DEFAULT_NAVIGATOR_DEVICE_MEMORY: f64 = 8.0;
pub const DEFAULT_CONNECTION_TYPE: &str = "unknown";
pub const DEFAULT_CONNECTION_DOWNLINK_MAX: f64 = f64::INFINITY;
pub const DEFAULT_CONNECTION_EFFECTIVE_TYPE: &str = "4g";
pub const DEFAULT_CONNECTION_DOWNLINK: f64 = 10.0;
pub const DEFAULT_CONNECTION_RTT: f64 = 50.0;
pub const DEFAULT_CONNECTION_SAVE_DATA: bool = false;

pub const DEFAULT_WINDOW_SURFACE_PROFILE: WindowSurfaceProfile = WindowSurfaceProfile {
    user_agent: DEFAULT_USER_AGENT,
    platform: DEFAULT_NAVIGATOR_PLATFORM,
    language: "en-US",
    hardware_concurrency: 4.0,
    max_touch_points: 0.0,
    inner_width: 1920.0,
    inner_height: 1080.0,
    device_pixel_ratio: 1.0,
    // Keep the stable desktop profile internally consistent: inner viewport
    // should not exceed the reported screen bounds. This stays close to the
    // Chromium-derived Zhihu-clearing profile without carrying a textbook
    // `innerWidth > screen.width` fingerprint mismatch.
    screen_width: 1920.0,
    screen_height: 1080.0,
    screen_avail_width: 1920.0,
    screen_avail_height: 1080.0,
    color_depth: 24.0,
    pixel_depth: 24.0,
    orientation_angle: 0.0,
    orientation_type: "landscape-primary",
    visual_viewport_scale: 1.0,
};

pub fn navigator_app_version(user_agent: &str) -> &str {
    user_agent.strip_prefix("Mozilla/").unwrap_or(user_agent)
}

pub fn chromium_major_version(user_agent: &str) -> Option<&str> {
    let (_, tail) = user_agent
        .split_once("HeadlessChrome/")
        .or_else(|| user_agent.split_once("Chrome/"))?;
    let major_end = tail.find('.').unwrap_or(tail.len());
    let major = &tail[..major_end];
    (!major.is_empty() && major.chars().all(|ch| ch.is_ascii_digit())).then_some(major)
}

pub fn chromium_full_version(user_agent: &str) -> Option<&str> {
    let (_, tail) = user_agent
        .split_once("HeadlessChrome/")
        .or_else(|| user_agent.split_once("Chrome/"))?;
    let version_end = tail
        .find(|character: char| character.is_ascii_whitespace())
        .unwrap_or(tail.len());
    let version = &tail[..version_end];
    (!version.is_empty()
        && version.split('.').all(|component| {
            !component.is_empty() && component.chars().all(|ch| ch.is_ascii_digit())
        }))
    .then_some(version)
}

pub fn chromium_product_brand(user_agent: &str) -> Option<&'static str> {
    if user_agent.contains("HeadlessChrome/") {
        Some("HeadlessChrome")
    } else if user_agent.contains("Chrome/") {
        Some("Google Chrome")
    } else {
        None
    }
}

pub fn chromium_greased_brand_version(seed: usize) -> (String, String) {
    const GREASE_CHARS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];
    const GREASE_VERSIONS: [&str; 3] = ["8", "99", "24"];
    (
        format!(
            "Not{}A{}Brand",
            GREASE_CHARS[seed % GREASE_CHARS.len()],
            GREASE_CHARS[(seed + 1) % GREASE_CHARS.len()],
        ),
        GREASE_VERSIONS[seed % GREASE_VERSIONS.len()].to_owned(),
    )
}

pub fn chromium_brand_list_order(seed: usize, size: usize) -> Option<Vec<usize>> {
    match size {
        2 => Some(vec![seed % size, (seed + 1) % size]),
        3 => {
            const ORDERS: [[usize; 3]; 6] = [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ];
            Some(ORDERS[seed % ORDERS.len()].to_vec())
        }
        4 => {
            const ORDERS: [[usize; 4]; 24] = [
                [0, 1, 2, 3],
                [0, 1, 3, 2],
                [0, 2, 1, 3],
                [0, 2, 3, 1],
                [0, 3, 1, 2],
                [0, 3, 2, 1],
                [1, 0, 2, 3],
                [1, 0, 3, 2],
                [1, 2, 0, 3],
                [1, 2, 3, 0],
                [1, 3, 0, 2],
                [1, 3, 2, 0],
                [2, 0, 1, 3],
                [2, 0, 3, 1],
                [2, 1, 0, 3],
                [2, 1, 3, 0],
                [2, 3, 0, 1],
                [2, 3, 1, 0],
                [3, 0, 1, 2],
                [3, 0, 2, 1],
                [3, 1, 0, 2],
                [3, 1, 2, 0],
                [3, 2, 0, 1],
                [3, 2, 1, 0],
            ];
            Some(ORDERS[seed % ORDERS.len()].to_vec())
        }
        _ => None,
    }
}

pub fn chromium_sec_ch_ua_value(user_agent: &str) -> Option<String> {
    chromium_ua_brand_versions(user_agent, false).map(format_sec_ch_ua_brand_versions)
}

pub fn chromium_sec_ch_ua_full_version_list_value(user_agent: &str) -> Option<String> {
    chromium_ua_brand_versions(user_agent, true).map(format_sec_ch_ua_brand_versions)
}

pub fn chromium_ua_brand_versions(
    user_agent: &str,
    full_versions: bool,
) -> Option<Vec<(String, String)>> {
    let ua_major = chromium_major_version(user_agent)?;
    let ua_full = chromium_full_version(user_agent)?;
    let seed = ua_major.parse::<usize>().ok()?;
    let (greased_brand, greased_version) = chromium_greased_brand_version(seed);
    let greased_version = if full_versions {
        format!("{greased_version}.0.0.0")
    } else {
        greased_version
    };
    let product_version = if full_versions { ua_full } else { ua_major };

    let mut brand_version_list = vec![
        (greased_brand, greased_version),
        ("Chromium".to_owned(), product_version.to_owned()),
    ];
    if let Some(brand) = chromium_product_brand(user_agent) {
        brand_version_list.push((brand.to_owned(), product_version.to_owned()));
    }

    let order = chromium_brand_list_order(seed, brand_version_list.len())?;
    let mut shuffled = vec![("".to_owned(), "".to_owned()); brand_version_list.len()];
    for (index, shuffled_index) in order.iter().copied().enumerate() {
        shuffled[shuffled_index] = brand_version_list[index].clone();
    }
    Some(shuffled)
}

fn format_sec_ch_ua_brand_versions(brand_versions: Vec<(String, String)>) -> String {
    brand_versions
        .into_iter()
        .map(|(brand, version)| format!("\"{brand}\";v=\"{version}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_window_surface_profile_matches_stable_moli_baseline() {
        assert_eq!(
            DEFAULT_WINDOW_SURFACE_PROFILE.user_agent,
            DEFAULT_USER_AGENT
        );
        assert_eq!(
            DEFAULT_WINDOW_SURFACE_PROFILE.platform,
            DEFAULT_NAVIGATOR_PLATFORM
        );
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.language, "en-US");
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.hardware_concurrency, 4.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.max_touch_points, 0.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width, 1920.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height, 1080.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.screen_width, 1920.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.screen_height, 1080.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.screen_avail_width, 1920.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.screen_avail_height, 1080.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.color_depth, 24.0);
        assert_eq!(DEFAULT_WINDOW_SURFACE_PROFILE.pixel_depth, 24.0);
        assert_eq!(
            DEFAULT_WINDOW_SURFACE_PROFILE.orientation_type,
            "landscape-primary"
        );
    }

    #[test]
    fn navigator_app_version_strips_mozilla_prefix() {
        assert_eq!(
            navigator_app_version(DEFAULT_USER_AGENT),
            DEFAULT_USER_AGENT.strip_prefix("Mozilla/").unwrap()
        );
        assert_eq!(navigator_app_version("CustomAgent/1.0"), "CustomAgent/1.0");
    }

    #[test]
    fn sec_ch_ua_value_matches_chromium_seeded_order_for_chrome() {
        assert_eq!(
            chromium_sec_ch_ua_value(DEFAULT_USER_AGENT).as_deref(),
            Some("\"Not:A-Brand\";v=\"8\", \"Chromium\";v=\"156\", \"Google Chrome\";v=\"156\"")
        );
    }

    #[test]
    fn sec_ch_ua_full_version_list_uses_the_same_brand_order() {
        assert_eq!(
            chromium_sec_ch_ua_full_version_list_value(DEFAULT_USER_AGENT).as_deref(),
            Some(
                "\"Not:A-Brand\";v=\"8.0.0.0\", \"Chromium\";v=\"156.0.8078.74\", \"Google Chrome\";v=\"156.0.8078.74\""
            )
        );
        assert_eq!(
            chromium_full_version(DEFAULT_USER_AGENT),
            Some("156.0.8078.74")
        );
    }

    #[test]
    fn sec_ch_ua_value_matches_chromium_seeded_order_for_headless() {
        let user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/145.0.0.0 Safari/537.36";
        assert_eq!(
            chromium_sec_ch_ua_value(user_agent).as_deref(),
            Some("\"Not:A-Brand\";v=\"99\", \"HeadlessChrome\";v=\"145\", \"Chromium\";v=\"145\"")
        );
    }

    #[test]
    fn chromium_major_version_prefers_headless_prefix_without_dead_fallback() {
        assert_eq!(
            chromium_major_version(
                "Mozilla/5.0 HeadlessChrome/145.0.0.0 Safari/537.36 Chrome/999.0.0.0"
            ),
            Some("145")
        );
        assert_eq!(chromium_major_version(DEFAULT_USER_AGENT), Some("156"));
        assert!(
            DEFAULT_USER_AGENT.contains(DEFAULT_CDP_PRODUCT),
            "CDP product and default user agent must advertise the same Chromium compatibility version"
        );
        assert_eq!(chromium_major_version("Mozilla/5.0 Safari/537.36"), None);
    }
}
