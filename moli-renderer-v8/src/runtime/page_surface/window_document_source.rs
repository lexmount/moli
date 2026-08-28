/// Exact renderer browsing-context source of a browser-owner handoff.
///
/// Root frames are identified by the enclosing Page/Document identity carried
/// by the handoff. Child frames need their own stable LocalWindow/Document
/// identity and may not silently degrade to the Page's then-current root frame
/// after the handoff leaves the renderer. An auxiliary top-level context owns
/// a real Page, so its source is that Page's `RootFrame` rather than a parallel
/// popup-only source kind.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RendererWindowDocumentSource {
    RootFrame,
    ChildFrame {
        frame_id: String,
        local_window_id: u64,
        document_id: u64,
    },
}
