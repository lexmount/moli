// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(missing_docs, reason = "We have many as-yet undocumented items.")]
#![expect(
    missing_debug_implementations,
    unreachable_pub,
    clippy::cast_possible_truncation,
    reason = "Deferred"
)]

use accesskit::{Node, Role, TreeId, TreeInfo, TreeUpdate};
use anyhow::Result;
use std::num::NonZeroU32;
use std::sync::Arc;
use ui_events_winit::{WindowEventReducer, WindowEventTranslation};
use vello_cpu::peniko::Color;
use vello_cpu::peniko::color::PremulRgba8;
use vello_cpu::{Pixmap, RenderContext, Resources, kurbo::Rect};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::Window;

/// The window background color.
const BACKGROUND_COLOR: Color = Color::from_rgb8(30, 30, 30);

type SoftbufferSurface = softbuffer::Surface<Arc<Window>, Arc<Window>>;

mod access_ids;
mod accessibility;
use access_ids::{TEXT_INPUT_ID, WINDOW_ID};

mod text;

const WINDOW_TITLE: &str = "Text Editor";

// Simple struct to hold the state of the renderer
pub struct ActiveRenderState {
    // The fields MUST be in this order, so that the surface and AccessKit adapter are dropped before the window
    surface: SoftbufferSurface,
    access_adapter: accesskit_winit::Adapter,
    window: Arc<Window>,
    sent_initial_access_update: bool,
}

impl ActiveRenderState {
    fn access_update(&mut self, editor: &mut text::Editor) {
        self.access_adapter.update_if_active(|| {
            let mut update = TreeUpdate {
                tree_id: TreeId::ROOT,
                nodes: vec![],
                tree: (!self.sent_initial_access_update).then(|| TreeInfo::new(WINDOW_ID)),
                focus: TEXT_INPUT_ID,
            };
            if !self.sent_initial_access_update {
                let mut node = Node::new(Role::Window);
                node.set_label(WINDOW_TITLE);
                node.push_child(TEXT_INPUT_ID);
                update.nodes.push((WINDOW_ID, node));
                self.sent_initial_access_update = true;
            }
            let mut node = Node::new(Role::TextInput);
            let size = self.window.inner_size();
            node.set_bounds(accesskit::Rect {
                x0: 0.0,
                y0: 0.0,
                x1: size.width as _,
                y1: size.height as _,
            });
            let rgba = text::BACKGROUND_COLOR.to_rgba8();
            node.set_background_color(accesskit::Color {
                red: rgba.r,
                green: rgba.g,
                blue: rgba.b,
                alpha: rgba.a,
            });
            editor.accessibility(&mut update, &mut node);
            update.nodes.push((TEXT_INPUT_ID, node));
            update
        });
    }
}

enum RenderState {
    Active(ActiveRenderState),
    // Cache a window so that it can be reused when the app is resumed after being suspended
    Suspended(Option<Arc<Window>>),
}

struct SimpleVelloApp {
    /// The softbuffer context used to create surfaces for windows.
    context: Option<softbuffer::Context<Arc<Window>>>,

    /// State for our example where we store the winit Window and the softbuffer Surface.
    state: RenderState,

    /// The `vello_cpu` render context into which the editor layout is drawn.
    renderer: RenderContext,
    /// Persistent glyph and image caches used by `renderer`.
    resources: Resources,

    /// The pixmap that the `vello_cpu` renderer rasterizes into.
    pixmap: Pixmap,

    /// Our `Editor`, which owns a `parley::PlainEditor`.
    editor: text::Editor,

    /// The last generation of the editor layout that we drew.
    last_drawn_generation: text::Generation,

    /// The IME cursor area we last sent to the platform.
    last_sent_ime_cursor_area: parley::BoundingBox,

    /// The event loop proxy required by the AccessKit winit adapter.
    event_loop_proxy: EventLoopProxy<accesskit_winit::Event>,

    /// Translate winit events into ui-events events.
    event_reducer: WindowEventReducer,
}

impl SimpleVelloApp {
    /// Ensure the `vello_cpu` render context and pixmap match the given size, recreating
    /// them (and forcing a redraw) if the size has changed.
    fn handle_resize(&mut self, width: u16, height: u16) {
        if self.renderer.width() != width || self.renderer.height() != height {
            self.renderer = RenderContext::new(width, height);
            self.pixmap = Pixmap::new(width, height);
            // Force the editor to be re-rasterized into the new pixmap.
            self.last_drawn_generation = text::Generation::default();
        }
    }
}

impl ApplicationHandler<accesskit_winit::Event> for SimpleVelloApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let RenderState::Suspended(cached_window) = &mut self.state else {
            return;
        };

        // Get the winit window cached in a previous Suspended event or else create a new window
        let window = cached_window
            .take()
            .unwrap_or_else(|| create_winit_window(event_loop));
        let access_adapter = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.event_loop_proxy.clone(),
        );
        window.set_visible(true);
        window.set_ime_allowed(true);

        let size = window.inner_size();

        // Create (or reuse) the softbuffer context and create a surface for this window.
        let context = self
            .context
            .get_or_insert_with(|| softbuffer::Context::new(window.clone()).unwrap());
        let mut surface = softbuffer::Surface::new(context, window.clone()).unwrap();
        if let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            surface.resize(width, height).unwrap();
        }

        // Ensure the CPU renderer and pixmap match the window size.
        self.handle_resize(to_nonzero_u16(size.width), to_nonzero_u16(size.height));

        // Save the Window and Surface to a state variable
        self.state = RenderState::Active(ActiveRenderState {
            surface,
            access_adapter,
            window,
            sent_initial_access_update: false,
        });

        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        if let RenderState::Active(state) = &self.state {
            self.state = RenderState::Suspended(Some(state.window.clone()));
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        match cause {
            StartCause::Init => {
                self.editor.cursor_reset();
                if let Some(next_time) = self.editor.next_blink_time() {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(next_time));
                }
            }
            StartCause::ResumeTimeReached { .. } => {
                self.editor.cursor_blink();

                if let Some(next_time) = self.editor.next_blink_time() {
                    self.last_drawn_generation = text::Generation::default();
                    if let RenderState::Active(state) = &self.state {
                        state.window.request_redraw();
                    }
                    event_loop.set_control_flow(ControlFlow::WaitUntil(next_time));
                }
            }
            StartCause::WaitCancelled { .. } => {
                if let Some(next_time) = self.editor.next_blink_time() {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(next_time));
                }
            }
            _ => {}
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // Ignore the event (return from the function) if
        //   - we have no render_state
        //   - OR the window id of the event doesn't match the window id of our render_state
        //
        // Else extract a mutable reference to the render state from its containing option for use below
        let render_state = match &mut self.state {
            RenderState::Active(state) if state.window.id() == window_id => state,
            _ => return,
        };

        render_state
            .access_adapter
            .process_event(&render_state.window, &event);

        if !matches!(
            event,
            WindowEvent::KeyboardInput {
                is_synthetic: true,
                ..
            }
        ) {
            if let Some(wet) = self
                .event_reducer
                .reduce(render_state.window.scale_factor(), &event)
            {
                match wet {
                    WindowEventTranslation::Keyboard(k) => {
                        self.editor.handle_keyboard_event(&k);
                    }
                    WindowEventTranslation::Pointer(p) => {
                        self.editor.handle_pointer_event(&p);
                    }
                }
            } else {
                self.editor.handle_event(event.clone());
            }
        }

        if self.last_drawn_generation != self.editor.generation() {
            render_state.window.request_redraw();
            let area = self.editor.editor().ime_cursor_area();
            if self.last_sent_ime_cursor_area != area {
                self.last_sent_ime_cursor_area = area;
                // Note: on X11 `set_ime_cursor_area` may cause the exclusion area to be obscured
                // until https://github.com/rust-windowing/winit/pull/3966 is in the Winit release
                // used by this example.
                render_state.window.set_ime_cursor_area(
                    PhysicalPosition::new(
                        area.x0 + text::INSET as f64,
                        area.y0 + text::INSET as f64,
                    ),
                    PhysicalSize::new(area.width(), area.height()),
                );
            }
        }
        // render_state
        //     .window
        //     .set_cursor(winit::window::Cursor::Icon(winit::window::CursorIcon::Text));

        match event {
            // Exit the event loop when a close is requested (e.g. window's close button is pressed)
            WindowEvent::CloseRequested => event_loop.exit(),

            // Resize the surface when the window is resized
            WindowEvent::Resized(size) => {
                if let (Some(width), Some(height)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    render_state.surface.resize(width, height).unwrap();
                }
                self.handle_resize(to_nonzero_u16(size.width), to_nonzero_u16(size.height));
                let editor = self.editor.editor();
                editor.set_scale(1.0);
                editor.set_width(Some(size.width as f32 - 2_f32 * text::INSET));
                if let RenderState::Active(state) = &self.state {
                    state.window.request_redraw();
                }
            }

            // Don't blink the cursor when we're not focused.
            WindowEvent::Focused(false) => {
                self.editor.disable_blink();
                self.editor.cursor_blink();
                self.last_drawn_generation = text::Generation::default();
                if let RenderState::Active(state) = &self.state {
                    state.window.request_redraw();
                }
            }
            // Make sure cursor is visible when we regain focus.
            WindowEvent::Focused(true) => self.editor.cursor_reset(),

            // This is where all the rendering happens
            WindowEvent::RedrawRequested => {
                // Send an accessibility update if accessibility is active.
                render_state.access_update(&mut self.editor);

                let width = self.renderer.width();
                let height = self.renderer.height();

                // Sometimes the pixmap is stale and needs to be redrawn.
                if self.last_drawn_generation != self.editor.generation() {
                    // Clear the render context and paint the background.
                    self.renderer.reset();
                    self.renderer.set_paint(BACKGROUND_COLOR);
                    self.renderer
                        .fill_rect(&Rect::new(0.0, 0.0, width as f64, height as f64));

                    self.last_drawn_generation =
                        self.editor.draw(&mut self.renderer, &mut self.resources);

                    // Rasterize the scene into the pixmap.
                    self.renderer.flush();
                    self.renderer.render(&mut self.pixmap, &mut self.resources);
                }

                // Copy the pixmap into the softbuffer surface and present it.
                let mut buffer = render_state.surface.buffer_mut().unwrap();
                for (dst, src) in buffer.iter_mut().zip(self.pixmap.data()) {
                    *dst = premul_rgba8_to_softbuffer(*src);
                }
                render_state.window.pre_present_notify();
                buffer.present().unwrap();
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: accesskit_winit::Event) {
        let render_state = match &mut self.state {
            RenderState::Active(state) if state.window.id() == event.window_id => state,
            _ => return,
        };

        match event.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                render_state.access_update(&mut self.editor);
            }
            accesskit_winit::WindowEvent::ActionRequested(req) => {
                if req.target_node == TEXT_INPUT_ID {
                    self.editor.handle_accesskit_action_request(&req);
                    if self.last_drawn_generation != self.editor.generation() {
                        render_state.window.request_redraw();
                    }
                }
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                render_state.sent_initial_access_update = false;
            }
        }
    }
}

fn main() -> Result<()> {
    // Create a winit event loop:
    let event_loop = EventLoop::with_user_event().build()?;

    // Setup a bunch of state:
    let mut app = SimpleVelloApp {
        context: None,
        state: RenderState::Suspended(None),
        // These are placeholders; they are recreated to match the window size in `resumed`.
        renderer: RenderContext::new(1, 1),
        resources: Resources::new(),
        pixmap: Pixmap::new(1, 1),
        editor: text::Editor::new(text::LOREM),
        last_drawn_generation: text::Generation::default(),
        last_sent_ime_cursor_area: parley::BoundingBox::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN),
        event_loop_proxy: event_loop.create_proxy(),
        event_reducer: WindowEventReducer::default(),
    };

    // Run the winit event loop
    event_loop
        .run_app(&mut app)
        .expect("Couldn't run event loop");
    let text = app.editor.text();
    print!("{text}");
    Ok(())
}

/// Helper function that creates a Winit window and returns it (wrapped in an Arc for sharing between threads)
fn create_winit_window(event_loop: &ActiveEventLoop) -> Arc<Window> {
    let attr = Window::default_attributes()
        .with_inner_size(LogicalSize::new(1044, 800))
        .with_resizable(true)
        .with_title(WINDOW_TITLE)
        .with_visible(false);
    Arc::new(event_loop.create_window(attr).unwrap())
}

/// Clamp a physical pixel dimension to a non-zero `u16`, as required by `vello_cpu`.
fn to_nonzero_u16(value: u32) -> u16 {
    value.clamp(1, u16::MAX as u32) as u16
}

/// Convert a premultiplied RGBA8 pixel into the `0RGB` `u32` format expected by softbuffer.
fn premul_rgba8_to_softbuffer(pixel: PremulRgba8) -> u32 {
    (u32::from(pixel.r) << 16) | (u32::from(pixel.g) << 8) | u32::from(pixel.b)
}
