/// Exact renderer browsing-context source of a browser-owner handoff.
///
/// Root frames are identified by the enclosing Page/Document identity carried
/// by the handoff. Child frames and lightweight popups need their own stable
/// LocalWindow/Document identity as well: neither may silently degrade to the
/// opener Page's then-current root frame after the handoff leaves the
/// renderer.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RendererWindowDocumentSource {
    RootFrame,
    ChildFrame {
        frame_id: String,
        local_window_id: u64,
        document_id: u64,
        top_level_popup_id: Option<u64>,
    },
    LightweightPopup {
        popup_id: u64,
        popup_document_id: u64,
    },
}
