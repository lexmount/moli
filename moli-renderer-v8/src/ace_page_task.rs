//! Synchronous ACE page tasks supplied by an embedder and run on the page owner thread.
use crate::*;
use anyhow::Result;
use moli_dom::NodeId as DomHandle;
use moli_dom::native::{DocumentReadyState, DomHost};
use moli_layout::{FrozenLayoutTree, LayoutDisplay};
use std::{collections::HashMap, fmt::Debug};

/// Tasks own their data; host borrows must not escape this call.
pub trait AcePageTask: Debug + Send {
    /// Input tasks may complete after their action navigates the page.
    fn is_input(&self) -> bool {
        false
    }
    fn run(self: Box<Self>, host: &mut dyn PageExtensionHost) -> Result<AcePageTaskOutput>;
}

#[derive(Clone, Debug)]
pub struct AcePageTaskOutput {
    pub value: serde_json::Value,
    pub input_outcome: Option<RendererInputDispatchOutcome>,
}

#[derive(Clone, Debug)]
pub struct InspectionStyle {
    pub properties: HashMap<String, String>,
    pub authored_display: Option<LayoutDisplay>,
    pub background_alpha: f32,
    pub event_listeners: Vec<String>,
    pub focusable: bool,
    pub disabled: bool,
}

impl InspectionStyle {
    pub fn property(&self, name: &str) -> String {
        self.properties.get(name).cloned().unwrap_or_default()
    }
}

/// Raw loading facts. Readiness policy belongs to the provider.
pub struct PageLoadingFacts {
    pub ready_state: DocumentReadyState,
    pub dom_content_loaded: bool,
    pub pending_network: bool,
    pub pending_scripts: bool,
    pub pending_navigation: bool,
}

/// Owner-thread capabilities, with no access to V8 handles or renderer internals.
/// DOM borrows cannot survive an input operation; node identities must be checked
/// again after operations that can execute page JavaScript.
pub trait PageExtensionHost {
    fn layout_enabled(&self) -> bool;
    fn document_identity(&self) -> RendererDocumentLifecycleIdentity;
    fn dom(&self) -> &DomHost;
    fn url(&self) -> String;
    fn loading(&self) -> PageLoadingFacts;
    fn device_pixel_ratio(&self) -> f64;
    fn backend_node_ids(&mut self) -> HashMap<DomHandle, u32>;
    fn live_handle(&mut self, id: u32) -> Option<DomHandle>;
    fn backend_node_id(&mut self, handle: DomHandle) -> Option<u32>;
    fn focused_node(&self) -> Option<DomHandle>;
    fn disabled(&self, handle: DomHandle) -> bool;
    fn prepare_inspection(&mut self);
    /// Visits a temporary layout snapshot without publishing page geometry.
    fn with_layout(
        &mut self,
        visitor: &mut dyn FnMut(&FrozenLayoutTree<DomHandle>),
    ) -> Result<bool>;
    fn styles(
        &self,
        properties: &[&str],
        events: &[&str],
        nodes: &[DomHandle],
    ) -> HashMap<DomHandle, InspectionStyle>;
    fn scroll_into_view(&mut self, id: u32) -> Result<RendererScrollIntoViewResult>;
    fn client_rect(&mut self, id: u32) -> Result<Option<RendererDocumentNodeClientRect>>;
    fn hit_test(&mut self, x: f64, y: f64) -> Result<Option<u32>>;
    fn focus(&mut self, id: u32) -> Result<RendererDomFocusOutcome>;
    fn mouse(
        &mut self,
        x: f64,
        y: f64,
        event: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome>;
    fn key(
        &mut self,
        event: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        repeat: bool,
        insert_text: bool,
    ) -> Result<RendererInputDispatchOutcome>;
    fn activate(
        &mut self,
        handle: DomHandle,
        x: f64,
        y: f64,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome>;
    fn replace_text(&mut self, handle: DomHandle, text: &str) -> Result<bool>;
    fn submit(&mut self, handle: DomHandle) -> Result<Option<RendererInputDispatchOutcome>>;
    fn select_values(
        &mut self,
        handle: DomHandle,
        values: &[String],
        multiple: bool,
    ) -> Result<bool>;
    fn screenshot_clip(&mut self, handle: DomHandle) -> Result<RendererScreenshotClip>;
    fn capture(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<RendererCaptureScreenshotReply>;
    fn flush_screenshot(&mut self) -> Result<()>;
}
