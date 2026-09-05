use moli_core::browser::WebContentsId;

/// Browser window state has the lifetime of its owning WebContents.
#[derive(Debug, Default)]
pub(in crate::conn) struct Window {
    pub(in crate::conn) name: Option<String>,
    pub(in crate::conn) opener: Option<WindowOpener>,
    pub(in crate::conn) surface: WindowSurface,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::conn) struct WindowOpener {
    pub(in crate::conn) web_contents_id: WebContentsId,
    // A noopener window still has creator attribution, but no script access.
    pub(in crate::conn) can_access: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum WindowSurfaceState {
    #[default]
    Normal,
    Maximized,
    Minimized,
    Fullscreen,
}

impl WindowSurfaceState {
    pub(crate) fn document_hidden(self) -> bool {
        matches!(self, Self::Minimized)
    }

    pub(crate) fn is_fullscreen(self) -> bool {
        matches!(self, Self::Fullscreen)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Maximized => "maximized",
            Self::Minimized => "minimized",
            Self::Fullscreen => "fullscreen",
        }
    }
}

/// A value snapshot; it confers no mutable access to Browser state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WindowSurface {
    pub(crate) state: WindowSurfaceState,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

impl WindowSurface {
    pub(in crate::conn) fn set_geometry(
        &mut self,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) {
        if let Some(width) = width {
            self.width = width;
        }
        if let Some(height) = height {
            self.height = height;
        }
        if let Some(x) = x {
            self.x = x;
        }
        if let Some(y) = y {
            self.y = y;
        }
    }
}
