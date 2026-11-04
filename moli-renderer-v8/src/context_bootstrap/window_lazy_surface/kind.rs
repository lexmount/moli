use super::super::{
    WINDOW_CRYPTO_SLOT, WINDOW_CUSTOM_ELEMENTS_SLOT, WINDOW_EXTERNAL_SLOT, WINDOW_NAVIGATOR_SLOT,
    WINDOW_PERFORMANCE_SLOT, WINDOW_SCREEN_SLOT, WINDOW_SPEECH_SYNTHESIS_SLOT,
    WINDOW_VISUAL_VIEWPORT_SLOT,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowLazySurface {
    Navigator,
    External,
    Performance,
    CustomElements,
    Screen,
    Crypto,
    VisualViewport,
    SpeechSynthesis,
}

impl WindowLazySurface {
    pub(in crate::context_bootstrap) fn from_slot(slot: &str) -> Option<Self> {
        match slot {
            WINDOW_NAVIGATOR_SLOT => Some(Self::Navigator),
            WINDOW_EXTERNAL_SLOT => Some(Self::External),
            WINDOW_PERFORMANCE_SLOT => Some(Self::Performance),
            WINDOW_CUSTOM_ELEMENTS_SLOT => Some(Self::CustomElements),
            WINDOW_SCREEN_SLOT => Some(Self::Screen),
            WINDOW_CRYPTO_SLOT => Some(Self::Crypto),
            WINDOW_VISUAL_VIEWPORT_SLOT => Some(Self::VisualViewport),
            WINDOW_SPEECH_SYNTHESIS_SLOT => Some(Self::SpeechSynthesis),
            _ => None,
        }
    }

    pub(crate) const fn slot(self) -> &'static str {
        match self {
            Self::Navigator => WINDOW_NAVIGATOR_SLOT,
            Self::External => WINDOW_EXTERNAL_SLOT,
            Self::Performance => WINDOW_PERFORMANCE_SLOT,
            Self::CustomElements => WINDOW_CUSTOM_ELEMENTS_SLOT,
            Self::Screen => WINDOW_SCREEN_SLOT,
            Self::Crypto => WINDOW_CRYPTO_SLOT,
            Self::VisualViewport => WINDOW_VISUAL_VIEWPORT_SLOT,
            Self::SpeechSynthesis => WINDOW_SPEECH_SYNTHESIS_SLOT,
        }
    }

    pub(super) const fn materializing_slot(self) -> &'static str {
        match self {
            Self::Navigator => "__moliWindowNavigatorMaterializing",
            Self::External => "__moliWindowExternalMaterializing",
            Self::Performance => "__moliWindowPerformanceMaterializing",
            Self::CustomElements => "__moliWindowCustomElementsMaterializing",
            Self::Screen => "__moliWindowScreenMaterializing",
            Self::Crypto => "__moliWindowCryptoMaterializing",
            Self::VisualViewport => "__moliWindowVisualViewportMaterializing",
            Self::SpeechSynthesis => "__moliWindowSpeechSynthesisMaterializing",
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Navigator => "Navigator",
            Self::External => "External",
            Self::Performance => "Performance",
            Self::CustomElements => "CustomElementRegistry",
            Self::Screen => "Screen",
            Self::Crypto => "Crypto",
            Self::VisualViewport => "VisualViewport",
            Self::SpeechSynthesis => "SpeechSynthesis",
        }
    }
}
