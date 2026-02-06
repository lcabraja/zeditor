mod assets;
mod editor;
#[cfg(target_os = "macos")]
mod hotkey;
mod theme;

use assets::*;
use editor::*;
use gpui::*;
use theme::*;

#[cfg(target_os = "macos")]
use raw_window_handle::HasWindowHandle;
#[cfg(target_os = "macos")]
use objc::{msg_send, sel, sel_impl};

actions!(popup_editor, [Quit, Escape]);

pub struct PopupEditor {
    editor: Entity<MultiLineEditor>,
}

impl PopupEditor {
    fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(MultiLineEditor::new);
        Self { editor }
    }

    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        if editor.has_multiple_cursors() {
            // Stage 1: collapse to single cursor
            self.editor.update(cx, |editor, cx| {
                editor.collapse_to_primary_cursor(cx);
            });
        } else {
            // Stage 2: hide the popup
            hide_window(window);
        }
    }
}

impl Render for PopupEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();

        div()
            .key_context("PopupEditor")
            .track_focus(&self.editor.read(cx).focus_handle)
            .on_action(cx.listener(Self::escape))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.base)
            .text_color(theme.text)
            .overflow_hidden()
            .child(
                // Header bar
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(32.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(theme.surface0)
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.subtext0)
                            .child("Zeditor"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.overlay0)
                            .child("Esc to close"),
                    ),
            )
            .child(
                // Editor area
                div()
                    .flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.editor.clone()),
            )
    }
}

impl Focusable for PopupEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.read(cx).focus_handle.clone()
    }
}

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        // Bind keybindings
        cx.bind_keys([
            // App-level keybindings
            KeyBinding::new("escape", Escape, Some("PopupEditor")),
            KeyBinding::new("cmd-q", Quit, None),
            // Editor keybindings
            KeyBinding::new("backspace", Backspace, Some("MultiLineEditor")),
            KeyBinding::new("delete", Delete, Some("MultiLineEditor")),
            KeyBinding::new("cmd-backspace", DeleteToStart, Some("MultiLineEditor")),
            KeyBinding::new("alt-backspace", DeleteWordBackward, Some("MultiLineEditor")),
            KeyBinding::new("left", Left, Some("MultiLineEditor")),
            KeyBinding::new("right", Right, Some("MultiLineEditor")),
            KeyBinding::new("up", Up, Some("MultiLineEditor")),
            KeyBinding::new("down", Down, Some("MultiLineEditor")),
            KeyBinding::new("shift-left", SelectLeft, Some("MultiLineEditor")),
            KeyBinding::new("shift-right", SelectRight, Some("MultiLineEditor")),
            KeyBinding::new("shift-up", SelectUp, Some("MultiLineEditor")),
            KeyBinding::new("shift-down", SelectDown, Some("MultiLineEditor")),
            KeyBinding::new("cmd-a", SelectAll, Some("MultiLineEditor")),
            KeyBinding::new("home", Home, Some("MultiLineEditor")),
            KeyBinding::new("end", End, Some("MultiLineEditor")),
            KeyBinding::new("cmd-left", Home, Some("MultiLineEditor")),
            KeyBinding::new("cmd-right", End, Some("MultiLineEditor")),
            KeyBinding::new("cmd-up", DocumentStart, Some("MultiLineEditor")),
            KeyBinding::new("cmd-down", DocumentEnd, Some("MultiLineEditor")),
            KeyBinding::new("alt-left", WordLeft, Some("MultiLineEditor")),
            KeyBinding::new("alt-right", WordRight, Some("MultiLineEditor")),
            KeyBinding::new("alt-shift-left", SelectWordLeft, Some("MultiLineEditor")),
            KeyBinding::new("alt-shift-right", SelectWordRight, Some("MultiLineEditor")),
            KeyBinding::new("enter", Enter, Some("MultiLineEditor")),
            KeyBinding::new("alt-up", MoveLineUp, Some("MultiLineEditor")),
            KeyBinding::new("alt-down", MoveLineDown, Some("MultiLineEditor")),
            KeyBinding::new("alt-shift-up", AddCursorUp, Some("MultiLineEditor")),
            KeyBinding::new("alt-shift-down", AddCursorDown, Some("MultiLineEditor")),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("MultiLineEditor")),
            KeyBinding::new("cmd-v", Paste, Some("MultiLineEditor")),
            KeyBinding::new("cmd-c", Copy, Some("MultiLineEditor")),
            KeyBinding::new("cmd-x", Cut, Some("MultiLineEditor")),
        ]);

        cx.on_action(quit);

        // Initialize theme
        Theme::init(cx);

        // Create popup window
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(600.), px(400.)),
                cx,
            ))),
            titlebar: None,
            show: false,
            focus: false,
            kind: WindowKind::PopUp,
            ..Default::default()
        };

        let window_handle = cx
            .open_window(options, |window, cx| {
                cx.new(|cx| {
                    let popup = PopupEditor::new(cx);
                    // Focus the editor
                    let focus = popup.editor.read(cx).focus_handle.clone();
                    window.focus(&focus, cx);
                    popup
                })
            })
            .unwrap();

        // macOS-specific: set accessory activation policy and adjust window level
        #[cfg(target_os = "macos")]
        {
            use cocoa::appkit::NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory;
            use objc::{class, msg_send, sel, sel_impl};

            unsafe {
                // Set activation policy to Accessory (no Dock icon)
                let ns_app: cocoa::base::id =
                    msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![
                    ns_app,
                    setActivationPolicy: NSApplicationActivationPolicyAccessory as i64
                ];
            }

            // Get NSWindow from the GPUI window handle
            window_handle
                .update(cx, |_root, window, _cx| {
                    if let Ok(handle) = window.window_handle() {
                        let raw = handle.as_raw();
                        if let raw_window_handle::RawWindowHandle::AppKit(appkit) = raw {
                            let ns_view = appkit.ns_view.as_ptr() as *mut objc::runtime::Object;
                            unsafe {
                                // Get NSWindow from NSView
                                let ns_window: *mut objc::runtime::Object =
                                    msg_send![ns_view, window];
                                // Lower window level from NSPopUpWindowLevel (101)
                                // to NSFloatingWindowLevel (3)
                                let _: () = msg_send![ns_window, setLevel: 3i64];
                                // Register global hotkey
                                hotkey::register_hotkey(ns_window);
                            }
                        }
                    }
                })
                .ok();
        }
    });
}

#[cfg(target_os = "macos")]
fn hide_window(window: &mut Window) {
    if let Ok(handle) = window.window_handle() {
        let raw = handle.as_raw();
        if let raw_window_handle::RawWindowHandle::AppKit(appkit) = raw {
            let ns_view = appkit.ns_view.as_ptr() as *mut objc::runtime::Object;
            unsafe {
                let ns_window: *mut objc::runtime::Object = msg_send![ns_view, window];
                let _: () = msg_send![ns_window, orderOut: cocoa::base::nil];
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn hide_window(_window: &mut Window) {
    // No-op on other platforms
}

fn quit(_: &Quit, app: &mut App) {
    app.quit();
}
