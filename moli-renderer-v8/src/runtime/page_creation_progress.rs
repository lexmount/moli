use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// The renderer-side condition currently preventing an initial page from
/// reaching its requested lifecycle boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RendererPageCreationPhase {
    StreamingMainBody = 0,
    ProcessingMainDocument = 1,
    WaitingForParserBlockingScript = 2,
    WaitingForParserBlockingStylesheet = 3,
    WaitingForDomContentLoaded = 4,
    WaitingForLoad = 5,
}

impl RendererPageCreationPhase {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::StreamingMainBody,
            1 => Self::ProcessingMainDocument,
            2 => Self::WaitingForParserBlockingScript,
            3 => Self::WaitingForParserBlockingStylesheet,
            4 => Self::WaitingForDomContentLoaded,
            5 => Self::WaitingForLoad,
            _ => Self::ProcessingMainDocument,
        }
    }
}

/// Shared progress reported by the renderer while a streamed main Document is
/// being created.
///
/// The browser-side deadline owns a clone and reads it only if that deadline
/// expires. Renderer owner turns update the same value immediately before
/// parking on body input, parser-blocking resources, or a lifecycle target.
#[derive(Clone, Debug)]
pub struct RendererPageCreationProgress {
    phase: Arc<AtomicU8>,
}

impl RendererPageCreationProgress {
    pub fn new() -> Self {
        Self {
            phase: Arc::new(AtomicU8::new(
                RendererPageCreationPhase::StreamingMainBody as u8,
            )),
        }
    }

    pub fn phase(&self) -> RendererPageCreationPhase {
        RendererPageCreationPhase::from_u8(self.phase.load(Ordering::Relaxed))
    }

    pub(crate) fn set_phase(&self, phase: RendererPageCreationPhase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
    }
}

impl Default for RendererPageCreationProgress {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_observe_the_latest_phase() {
        let progress = RendererPageCreationProgress::new();
        let observer = progress.clone();

        progress.set_phase(RendererPageCreationPhase::WaitingForParserBlockingStylesheet);

        assert_eq!(
            observer.phase(),
            RendererPageCreationPhase::WaitingForParserBlockingStylesheet
        );
    }
}
