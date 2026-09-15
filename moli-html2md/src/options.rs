/// Conversion limits and the fallback language for fenced code blocks.
#[derive(Clone, Debug)]
pub struct Options {
    /// Visit nodes at depths `0..max_depth`, with the supplied root at depth 0.
    /// Deeper subtrees are omitted. Set to `usize::MAX` to disable this limit.
    pub max_depth: usize,
    /// Used when neither `<pre>` nor its `<code>` child declares a language.
    /// The hint ends at the first line ending, as Markdown info strings use one line.
    pub default_code_language: Option<String>,
    /// Preserve whitespace inside inline code, replacing line endings with spaces.
    /// When false, inline code follows ordinary HTML whitespace collapsing.
    pub preformatted_code: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_depth: 1000,
            default_code_language: None,
            preformatted_code: false,
        }
    }
}
