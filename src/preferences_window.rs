use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::preferences::{save_preferences, HotkeyConfig, Preferences};
use crate::theme::Theme;

#[cfg(target_os = "macos")]
use crate::hotkey;

actions!(preferences_window, [ClosePreferences, SavePreferences, ToggleRecording]);

pub struct PreferencesWindow {
    focus_handle: FocusHandle,
    recording: bool,
    current_hotkey: HotkeyConfig,
    recorded_key_code: Option<u32>,
    recorded_modifiers: u32,
    recorded_display: String,
}

impl PreferencesWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let prefs = cx.global::<Preferences>();
        Self {
            focus_handle: cx.focus_handle(),
            recording: false,
            current_hotkey: prefs.hotkey.clone(),
            recorded_key_code: None,
            recorded_modifiers: 0,
            recorded_display: String::new(),
        }
    }

    fn close(&mut self, _: &ClosePreferences, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn toggle_recording(&mut self, _: &ToggleRecording, _window: &mut Window, cx: &mut Context<Self>) {
        if self.recording {
            self.recording = false;
            self.recorded_key_code = None;
            self.recorded_modifiers = 0;
            self.recorded_display.clear();
        } else {
            self.recording = true;
            self.recorded_key_code = None;
            self.recorded_modifiers = 0;
            self.recorded_display.clear();
        }
        cx.notify();
    }

    fn save(&mut self, _: &SavePreferences, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(key_code) = self.recorded_key_code else {
            return;
        };
        let modifiers = self.recorded_modifiers;
        let display = self.recorded_display.clone();

        let new_config = HotkeyConfig {
            key_code,
            modifiers,
            display_string: display,
        };

        let mut prefs = cx.global::<Preferences>().clone();
        prefs.hotkey = new_config.clone();
        cx.set_global(prefs.clone());
        save_preferences(&prefs);

        #[cfg(target_os = "macos")]
        unsafe {
            hotkey::re_register_hotkey(key_code, modifiers);
        }

        self.current_hotkey = new_config;
        self.recording = false;
        self.recorded_key_code = None;
        self.recorded_modifiers = 0;
        self.recorded_display.clear();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.recording {
            return;
        }

        let keystroke = &event.keystroke;

        if !keystroke.modifiers.platform
            && !keystroke.modifiers.alt
            && !keystroke.modifiers.control
        {
            return;
        }

        let Some(vk) = gpui_key_to_vk(&keystroke.key) else {
            return;
        };

        let mut carbon_mods: u32 = 0;
        if keystroke.modifiers.platform {
            carbon_mods |= 1 << 8;
        }
        if keystroke.modifiers.shift {
            carbon_mods |= 1 << 9;
        }
        if keystroke.modifiers.alt {
            carbon_mods |= 1 << 11;
        }
        if keystroke.modifiers.control {
            carbon_mods |= 1 << 12;
        }

        let mut display = String::new();
        if keystroke.modifiers.control {
            display.push_str("Ctrl+");
        }
        if keystroke.modifiers.alt {
            display.push_str("Alt+");
        }
        if keystroke.modifiers.shift {
            display.push_str("Shift+");
        }
        if keystroke.modifiers.platform {
            display.push_str("Cmd+");
        }
        display.push_str(&keystroke.key.to_uppercase());

        self.recorded_key_code = Some(vk);
        self.recorded_modifiers = carbon_mods;
        self.recorded_display = display;
        self.recording = false;
        cx.notify();
    }
}

impl Render for PreferencesWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let has_recorded = self.recorded_key_code.is_some();
        let recording = self.recording;

        let hotkey_display = if recording {
            "Waiting for input...".to_string()
        } else if has_recorded {
            self.recorded_display.clone()
        } else {
            self.current_hotkey.display_string.clone()
        };

        div()
            .key_context("PreferencesWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::toggle_recording))
            .on_action(cx.listener(Self::save))
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.mantle)
            .text_color(theme.text)
            .child(
                // Main content area with generous padding
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .p(px(24.))
                    .gap(px(20.))
                    // Section: Global Hotkey
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            // Section header
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.overlay0)
                                            .child("GLOBAL HOTKEY"),
                                    ),
                            )
                            // Hotkey row: display + button
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(10.))
                                    .p(px(12.))
                                    .rounded(px(8.))
                                    .bg(theme.base)
                                    .border_1()
                                    .border_color(if recording {
                                        theme.accent
                                    } else {
                                        theme.surface0
                                    })
                                    // Hotkey badge
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .items_center()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .h(px(28.))
                                                    .px(px(10.))
                                                    .rounded(px(5.))
                                                    .bg(if recording {
                                                        theme.surface0
                                                    } else {
                                                        theme.surface1
                                                    })
                                                    .text_size(px(12.))
                                                    .text_color(if recording {
                                                        theme.overlay1
                                                    } else {
                                                        theme.text
                                                    })
                                                    .child(hotkey_display),
                                            ),
                                    )
                                    // Action button
                                    .child(
                                        div()
                                            .id("record-btn")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .h(px(28.))
                                            .px(px(12.))
                                            .rounded(px(5.))
                                            .bg(if recording {
                                                theme.surface2
                                            } else {
                                                theme.surface1
                                            })
                                            .hover(|s| s.bg(theme.surface2))
                                            .cursor(CursorStyle::PointingHand)
                                            .text_size(px(12.))
                                            .text_color(theme.subtext0)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_recording(
                                                    &ToggleRecording,
                                                    window,
                                                    cx,
                                                );
                                            }))
                                            .child(if recording { "Cancel" } else { "Record" }),
                                    ),
                            )
                            // Helper text
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.overlay0)
                                    .child(if recording {
                                        "Press a key combination with at least one modifier (Cmd, Alt, Ctrl)"
                                    } else if has_recorded {
                                        "New hotkey recorded. Save to apply."
                                    } else {
                                        "Click Record to change the hotkey"
                                    }),
                            ),
                    )
                    // Error display
                    .when_some(get_hotkey_error(), |el, err| {
                        el.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .p(px(10.))
                                .rounded(px(6.))
                                .bg(rgba(0xf3838320))
                                .border_1()
                                .border_color(rgba(0xf3838340))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(0xf38383))
                                        .child(err),
                                ),
                        )
                    }),
            )
            // Bottom bar
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .w_full()
                    .h(px(48.))
                    .px(px(24.))
                    .border_t_1()
                    .border_color(theme.surface0)
                    .bg(theme.base)
                    .gap(px(8.))
                    .when(has_recorded, |el| {
                        el.child(
                            div()
                                .id("save-btn")
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(px(28.))
                                .px(px(14.))
                                .rounded(px(5.))
                                .bg(theme.accent)
                                .hover(|s| s.opacity(0.85))
                                .cursor(CursorStyle::PointingHand)
                                .text_size(px(12.))
                                .text_color(gpui::white())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save(&SavePreferences, window, cx);
                                }))
                                .child("Save"),
                        )
                    }),
            )
    }
}

impl Focusable for PreferencesWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(target_os = "macos")]
fn get_hotkey_error() -> Option<String> {
    hotkey::get_error()
}

#[cfg(not(target_os = "macos"))]
fn get_hotkey_error() -> Option<String> {
    None
}

/// Convert a GPUI key name to a macOS Carbon virtual key code.
fn gpui_key_to_vk(key: &str) -> Option<u32> {
    match key {
        "a" => Some(0x00),
        "s" => Some(0x01),
        "d" => Some(0x02),
        "f" => Some(0x03),
        "h" => Some(0x04),
        "g" => Some(0x05),
        "z" => Some(0x06),
        "x" => Some(0x07),
        "c" => Some(0x08),
        "v" => Some(0x09),
        "b" => Some(0x0B),
        "q" => Some(0x0C),
        "w" => Some(0x0D),
        "e" => Some(0x0E),
        "r" => Some(0x0F),
        "y" => Some(0x10),
        "t" => Some(0x11),
        "1" => Some(0x12),
        "2" => Some(0x13),
        "3" => Some(0x14),
        "4" => Some(0x15),
        "6" => Some(0x16),
        "5" => Some(0x17),
        "9" => Some(0x19),
        "7" => Some(0x1A),
        "8" => Some(0x1C),
        "0" => Some(0x1D),
        "o" => Some(0x1F),
        "u" => Some(0x20),
        "i" => Some(0x22),
        "p" => Some(0x23),
        "l" => Some(0x25),
        "j" => Some(0x26),
        "k" => Some(0x28),
        "n" => Some(0x2D),
        "m" => Some(0x2E),
        "space" => Some(0x31),
        "escape" => Some(0x35),
        "f1" => Some(0x7A),
        "f2" => Some(0x78),
        "f3" => Some(0x63),
        "f4" => Some(0x76),
        "f5" => Some(0x60),
        "f6" => Some(0x61),
        "f7" => Some(0x62),
        "f8" => Some(0x64),
        "f9" => Some(0x65),
        "f10" => Some(0x6D),
        "f11" => Some(0x67),
        "f12" => Some(0x6F),
        "-" => Some(0x1B),
        "=" => Some(0x18),
        "[" => Some(0x21),
        "]" => Some(0x1E),
        "\\" => Some(0x2A),
        ";" => Some(0x29),
        "'" => Some(0x27),
        "," => Some(0x2B),
        "." => Some(0x2F),
        "/" => Some(0x2C),
        "`" => Some(0x32),
        _ => None,
    }
}
