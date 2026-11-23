use gpui::{Rgba, WindowAppearance, rgb};

#[derive(Clone, Copy)]
pub struct Palette {
    pub frame: Rgba,
    pub toolbar: Rgba,
    pub field: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub hover: Rgba,
    pub selection: Rgba,
}

impl Palette {
    pub fn for_window(appearance: WindowAppearance) -> Self {
        if matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ) {
            Self {
                frame: rgb(0x182420),
                toolbar: rgb(0x22312c),
                field: rgb(0x15221e),
                border: rgb(0x3a4a43),
                text: rgb(0xe5eee9),
                muted: rgb(0xa5bbb0),
                accent: rgb(0x68d6b1),
                hover: rgb(0x31453d),
                selection: rgb(0x2d5849),
            }
        } else {
            Self {
                frame: rgb(0xe2ece7),
                toolbar: rgb(0xf5f8f6),
                field: rgb(0xe9f1ed),
                border: rgb(0xc5d7ce),
                text: rgb(0x182a24),
                muted: rgb(0x52675e),
                accent: rgb(0x137659),
                hover: rgb(0xd8e9e1),
                selection: rgb(0xb3e4d1),
            }
        }
    }
}
