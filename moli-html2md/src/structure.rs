pub(crate) fn is_block(tag: &str) -> bool {
    is_structural_block(tag)
        || matches!(
            tag,
            "audio" | "canvas" | "frameset" | "isindex" | "noframes" | "output"
        )
}

/// These elements already own their document/list/choice boundaries.
pub(crate) fn owns_boundaries(tag: &str) -> bool {
    is_structural_block(tag) || matches!(tag, "select" | "optgroup" | "option" | "col" | "colgroup")
}

pub(crate) fn is_structural_block(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "html"
            | "legend"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}
