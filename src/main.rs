mod assets;
mod clipboard_history;
mod editor;
#[cfg(target_os = "macos")]
mod history_hotkey;
mod history_platform;
#[cfg(target_os = "macos")]
mod hotkey;
mod logging;
mod preferences;
mod preferences_window;
mod theme;

use assets::*;
use clipboard_history::{ClipboardHistory, HistoryEvent};
use editor::*;
use gpui::*;
use preferences::*;
use preferences_window::*;
use theme::*;

#[cfg(target_os = "macos")]
use raw_window_handle::HasWindowHandle;

actions!(
    popup_editor,
    [
        Quit,
        Escape,
        SubmitAndPaste,
        OpenPreferences,
        OpenClipboardHistory
    ]
);

pub struct PopupEditor {
    editor: Entity<MultiLineEditor>,
    last_clipboard_hash: u64,
    history: Option<Entity<ClipboardHistory>>,
    history_subscription: Option<Subscription>,
}

impl PopupEditor {
    fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(MultiLineEditor::new);
        Self {
            editor,
            last_clipboard_hash: 0,
            history: None,
            history_subscription: None,
        }
    }

    /// Called when the window is about to show. Reads clipboard, checks if it
    /// changed since last open. If changed, replaces editor contents. If same,
    /// keeps existing editor state.
    fn on_show(&mut self, cx: &mut Context<Self>) {
        self.history = None;
        self.history_subscription = None;
        // Check for CLI/pipe initial text first
        #[cfg(target_os = "macos")]
        if let Some(initial_text) = hotkey::take_pending_clipboard() {
            let hash = initial_text.as_deref().map(Self::hash_str).unwrap_or(0);
            self.last_clipboard_hash = hash;
            self.editor.update(cx, |editor, cx| {
                editor.reset_with_text(initial_text, cx);
            });
            return;
        }

        let clipboard_text = cx
            .read_from_clipboard()
            .and_then(|item| item.text().map(|t| t.to_string()));

        let current_hash = clipboard_text
            .as_ref()
            .map(|t| Self::hash_str(t))
            .unwrap_or(0);

        if current_hash != self.last_clipboard_hash {
            self.last_clipboard_hash = current_hash;
            self.editor.update(cx, |editor, cx| {
                editor.reset_with_text(clipboard_text, cx);
            });
        }
        // else: clipboard unchanged, keep editor contents
    }

    fn hash_str(s: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    fn open_clipboard_history(
        &mut self,
        _: &OpenClipboardHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() {
            return;
        }
        if let Some(history) = &self.history {
            window.focus(&history.read(cx).search.read(cx).focus_handle);
            return;
        }
        let history = cx.new(ClipboardHistory::new);
        window.focus(&history.read(cx).search.read(cx).focus_handle);
        self.history_subscription =
            Some(
                cx.subscribe_in(&history, window, |this, _, event, window, cx| {
                    if let HistoryEvent::Selected(text) = event {
                        this.editor.update(cx, |editor, cx| {
                            editor.insert_history_text(text, window, cx);
                        });
                    }
                    this.history = None;
                    this.history_subscription = None;
                    window.focus(&this.editor.read(cx).focus_handle);
                    cx.notify();
                }),
            );
        self.history = Some(history);
        cx.notify();
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

    #[cfg(target_os = "macos")]
    fn submit_and_paste(
        &mut self,
        _: &SubmitAndPaste,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.editor.read(cx).get_submit_text();
        let op_id = logging::next_operation_id();
        logging::event(
            "popup.submit_and_paste",
            format!(
                "op_id={} start text_bytes={} text_lines={}",
                op_id,
                text.len(),
                text.lines().count()
            ),
        );
        unsafe {
            hotkey::submit_and_paste(op_id, &text);
        }
        logging::event(
            "popup.submit_and_paste",
            format!("op_id={} submitted_to_hotkey", op_id),
        );
    }

    #[cfg(not(target_os = "macos"))]
    fn submit_and_paste(
        &mut self,
        _: &SubmitAndPaste,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // No-op on other platforms
    }

    #[cfg(target_os = "macos")]
    fn open_preferences(
        &mut self,
        _: &OpenPreferences,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        open_preferences_window(cx);
    }

    #[cfg(not(target_os = "macos"))]
    fn open_preferences(
        &mut self,
        _: &OpenPreferences,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }
}

impl Render for PopupEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();

        div()
            .key_context("PopupEditor")
            .track_focus(&self.editor.read(cx).focus_handle)
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::submit_and_paste))
            .on_action(cx.listener(Self::open_preferences))
            .on_action(cx.listener(Self::open_clipboard_history))
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
                    .relative()
                    .w_full()
                    .h(px(32.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(theme.surface0)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(13.))
                            .text_color(theme.subtext0)
                            .child("Zeditor"),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .id("clipboard-history-button")
                                    .cursor_pointer()
                                    .text_size(px(11.))
                                    .text_color(theme.subtext0)
                                    .child("History ⌘⇧V")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_clipboard_history(
                                            &OpenClipboardHistory,
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_align(TextAlign::Right)
                            .text_size(px(11.))
                            .text_color(theme.overlay0)
                            .child(self.editor.read(cx).status_text()),
                    ),
            )
            .child(
                // Editor area
                div().flex().flex_1().w_full().overflow_hidden().child(
                    if let Some(history) = &self.history {
                        history.clone().into_any_element()
                    } else {
                        self.editor.clone().into_any_element()
                    },
                ),
            )
    }
}

impl Focusable for PopupEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.read(cx).focus_handle.clone()
    }
}

fn main() {
    // Check for CLI text argument or piped stdin
    #[cfg(target_os = "macos")]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.len() > 1 {
            let text = args[1..].join(" ");
            hotkey::set_initial_text(text);
        } else {
            unsafe extern "C" {
                fn isatty(fd: i32) -> i32;
            }
            let is_tty = unsafe { isatty(0) != 0 };
            if !is_tty {
                use std::io::Read;
                let mut text = String::new();
                if std::io::stdin().read_to_string(&mut text).is_ok() && !text.is_empty() {
                    hotkey::set_initial_text(text);
                }
            }
        }
    }

    Application::new().with_assets(Assets).run(|cx: &mut App| {
        // Load embedded fonts
        {
            let font_paths = cx.asset_source().list("fonts").unwrap();
            let mut fonts = Vec::new();
            for path in font_paths {
                if path.ends_with(".ttf")
                    && let Ok(Some(data)) = cx.asset_source().load(&path)
                {
                    fonts.push(data);
                }
            }
            cx.text_system().add_fonts(fonts).unwrap();
        }

        // Bind keybindings
        cx.bind_keys([
            // App-level keybindings
            KeyBinding::new("escape", Escape, Some("PopupEditor")),
            KeyBinding::new("cmd-enter", SubmitAndPaste, Some("PopupEditor")),
            KeyBinding::new("cmd-shift-v", OpenClipboardHistory, Some("PopupEditor")),
            KeyBinding::new("cmd-,", OpenPreferences, Some("PopupEditor")),
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
            KeyBinding::new("cmd-shift-left", SelectHome, Some("MultiLineEditor")),
            KeyBinding::new("cmd-shift-right", SelectEnd, Some("MultiLineEditor")),
            KeyBinding::new("cmd-shift-up", SelectDocumentStart, Some("MultiLineEditor")),
            KeyBinding::new("cmd-shift-down", SelectDocumentEnd, Some("MultiLineEditor")),
            KeyBinding::new("alt-left", WordLeft, Some("MultiLineEditor")),
            KeyBinding::new("alt-right", WordRight, Some("MultiLineEditor")),
            KeyBinding::new("alt-shift-left", SelectWordLeft, Some("MultiLineEditor")),
            KeyBinding::new("alt-shift-right", SelectWordRight, Some("MultiLineEditor")),
            KeyBinding::new("enter", Enter, Some("MultiLineEditor")),
            KeyBinding::new("alt-up", MoveLineUp, Some("MultiLineEditor")),
            KeyBinding::new("alt-down", MoveLineDown, Some("MultiLineEditor")),
            KeyBinding::new("cmd-alt-up", AddCursorUp, Some("MultiLineEditor")),
            KeyBinding::new("cmd-alt-down", AddCursorDown, Some("MultiLineEditor")),
            KeyBinding::new("ctrl-alt-cmd-up", AddCursorsToTop, Some("MultiLineEditor")),
            KeyBinding::new(
                "ctrl-alt-cmd-down",
                AddCursorsToBottom,
                Some("MultiLineEditor"),
            ),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("MultiLineEditor")),
            KeyBinding::new("cmd-v", Paste, Some("MultiLineEditor")),
            KeyBinding::new("cmd-c", Copy, Some("MultiLineEditor")),
            KeyBinding::new("cmd-x", Cut, Some("MultiLineEditor")),
            KeyBinding::new("alt-z", ToggleWordWrap, Some("MultiLineEditor")),
            // Preferences window keybindings
            KeyBinding::new("escape", ClosePreferences, Some("PreferencesWindow")),
            KeyBinding::new("cmd-w", ClosePreferences, Some("PreferencesWindow")),
        ]);

        cx.on_action(quit);

        // Initialize preferences (before theme, so hotkey config is available)
        Preferences::init(cx);

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
                observe_system_theme(window);
                cx.new(|cx| {
                    let popup = PopupEditor::new(cx);
                    // Focus the editor
                    let focus = popup.editor.read(cx).focus_handle.clone();
                    window.focus(&focus);
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

            // Read hotkey config from preferences
            let prefs = cx.global::<Preferences>();
            let key_code = prefs.hotkey.key_code;
            let modifiers = prefs.hotkey.modifiers;

            // Get NSWindow from the GPUI window handle
            window_handle
                .update(cx, |_root, window, _cx| {
                    if let Ok(handle) = window.window_handle() {
                        let raw = handle.as_raw();
                        if let raw_window_handle::RawWindowHandle::AppKit(appkit) = raw {
                            let ns_view = appkit.ns_view.as_ptr() as *mut objc::runtime::Object;
                            unsafe {
                                let ns_window: *mut objc::runtime::Object =
                                    msg_send![ns_view, window];
                                let _: () = msg_send![ns_window, setLevel: 3i64];
                                hotkey::register_hotkey(ns_window, key_code, modifiers);
                            }
                        }
                    }
                })
                .ok();

            // Wait for hotkey/menu requests without busy polling.
            cx.spawn(async move |cx: &mut AsyncApp| {
                loop {
                    let requests = cx
                        .background_executor()
                        .spawn(async { hotkey::wait_for_requests() })
                        .await;

                    if requests.open_preferences {
                        let _ = cx.update(|cx| {
                            open_preferences_window(cx);
                        });
                    }

                    if requests.show_window {
                        window_handle
                            .update(cx, |root: &mut PopupEditor, window, cx| {
                                root.on_show(cx);
                                window.focus(&root.editor.read(cx).focus_handle);
                            })
                            .ok();
                        unsafe { hotkey::show_window_now() };
                    }
                    if requests.open_clipboard_history {
                        let _ = window_handle.update(cx, |root: &mut PopupEditor, window, cx| {
                            root.open_clipboard_history(&OpenClipboardHistory, window, cx);
                        });
                    }
                }
            })
            .detach();
        }
    });
}

#[cfg(target_os = "macos")]
fn open_preferences_window(cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(400.), px(250.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Preferences".into()),
            ..Default::default()
        }),
        show: true,
        focus: true,
        kind: WindowKind::Normal,
        ..Default::default()
    };

    let _ = cx.open_window(options, |window, cx| {
        observe_system_theme(window);
        cx.new(PreferencesWindow::new)
    });
}

fn observe_system_theme(window: &mut Window) {
    window
        .observe_window_appearance(|window, cx| {
            Theme::set_for_appearance(window.appearance(), cx);
        })
        .detach();
}

#[cfg(target_os = "macos")]
fn hide_window(_window: &mut Window) {
    unsafe { hotkey::hide_popup_window() };
}

#[cfg(not(target_os = "macos"))]
fn hide_window(_window: &mut Window) {
    // No-op on other platforms
}

fn quit(_: &Quit, app: &mut App) {
    app.quit();
}
