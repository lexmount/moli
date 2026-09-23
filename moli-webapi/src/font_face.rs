use moli_webidl as webidl;

// WebIDL reads dictionary members in lexicographic order, and must finish
// their conversion before the constructor starts parsing CSS or copying bytes.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FontFaceDescriptors")]
pub struct FontFaceDescriptors {
    pub ascent_override: Option<String>,
    pub descent_override: Option<String>,
    pub display: Option<String>,
    pub feature_settings: Option<String>,
    pub line_gap_override: Option<String>,
    pub size_adjust: Option<String>,
    pub stretch: Option<String>,
    pub style: Option<String>,
    pub unicode_range: Option<String>,
    pub variant: Option<String>,
    pub variation_settings: Option<String>,
    pub weight: Option<String>,
}
