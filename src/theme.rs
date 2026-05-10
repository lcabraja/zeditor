use gpui::*;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSColor;

#[allow(dead_code)]
pub struct Theme {
    pub mode: ThemeMode,
    pub text: Rgba,
    pub subtext1: Rgba,
    pub subtext0: Rgba,
    pub overlay2: Rgba,
    pub overlay1: Rgba,
    pub overlay0: Rgba,
    pub surface2: Rgba,
    pub surface1: Rgba,
    pub surface0: Rgba,
    pub base: Rgba,
    pub base_blur: Rgba,
    pub mantle: Rgba,
    pub crust: Rgba,
    pub crust_light: Rgba,
    pub accent: Rgba,
}

impl Global for Theme {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
}

/// Get the system accent color on macOS
#[cfg(target_os = "macos")]
fn get_system_accent_color() -> Rgba {
    let accent_color: Retained<NSColor> = NSColor::controlAccentColor();
    // Convert to sRGB color space
    if let Some(rgb_color) =
        accent_color.colorUsingColorSpace(objc2_app_kit::NSColorSpace::sRGBColorSpace().as_ref())
    {
        let r = rgb_color.redComponent() as f32;
        let g = rgb_color.greenComponent() as f32;
        let b = rgb_color.blueComponent() as f32;
        let a = rgb_color.alphaComponent() as f32;
        return rgba(
            ((r * 255.0) as u32) << 24
                | ((g * 255.0) as u32) << 16
                | ((b * 255.0) as u32) << 8
                | (a * 255.0) as u32,
        );
    }
    // Fallback to default blue
    gpui::blue().into()
}

#[cfg(not(target_os = "macos"))]
fn get_system_accent_color() -> Rgba {
    gpui::blue().into()
}

impl Theme {
    pub fn init(app: &mut App) {
        Self::set_for_appearance(app.window_appearance(), app);
    }

    pub fn set_for_appearance(appearance: WindowAppearance, app: &mut App) {
        let mode = ThemeMode::from(appearance);
        let current_mode = app.try_global::<Theme>().map(|theme| theme.mode);

        if current_mode == Some(mode) {
            return;
        }

        app.set_global(match mode {
            ThemeMode::Light => Theme::get_light(),
            ThemeMode::Dark => Theme::get_dark(),
        });
        app.refresh_windows();
    }

    // Catppuccin Latte
    pub fn get_light() -> Theme {
        Theme {
            mode: ThemeMode::Light,
            text: rgb(0x4c4f69),
            subtext1: rgb(0x5c5f77),
            subtext0: rgb(0x6c6f85),
            overlay2: rgb(0x7c7f93),
            overlay1: rgb(0x8c8fa1),
            overlay0: rgb(0x9ca0b0),
            surface2: rgb(0xacb0be),
            surface1: rgb(0xbcc0cc),
            surface0: rgb(0xccd0da),
            base: rgb(0xeff1f5),
            base_blur: rgba(0xeff1f5dd),
            mantle: rgb(0xe6e9ef),
            crust: rgb(0xdce0e8),
            crust_light: rgba(0x9ca0b066),
            accent: get_system_accent_color(),
        }
    }

    // Catppuccin Mocha
    // Text	#cdd6f4	rgb(205, 214, 244)	hsl(226, 64%, 88%)
    // Subtext1	#bac2de	rgb(186, 194, 222)	hsl(227, 35%, 80%)
    // Subtext0	#a6adc8	rgb(166, 173, 200)	hsl(228, 24%, 72%)
    // Overlay2	#9399b2	rgb(147, 153, 178)	hsl(228, 17%, 64%)
    // Overlay1	#7f849c	rgb(127, 132, 156)	hsl(230, 13%, 55%)
    // Overlay0	#6c7086	rgb(108, 112, 134)	hsl(231, 11%, 47%)
    // Surface2	#585b70	rgb(88, 91, 112)	hsl(233, 12%, 39%)
    // Surface1	#45475a	rgb(69, 71, 90)	hsl(234, 13%, 31%)
    // Surface0	#313244	rgb(49, 50, 68)	hsl(237, 16%, 23%)
    // Base	#1e1e2e	rgb(30, 30, 46)	hsl(240, 21%, 15%)
    // Mantle	#181825	rgb(24, 24, 37)	hsl(240, 21%, 12%)
    // Crust	#11111b	rgb(17, 17, 27)	hsl(240, 23%, 9%)
    pub fn get_dark() -> Theme {
        Theme {
            mode: ThemeMode::Dark,
            text: rgb(0xcdd6f4),
            subtext1: rgb(0xbac2de),
            subtext0: rgb(0xa6adc8),
            overlay2: rgb(0x9399b2),
            overlay1: rgb(0x7f849c),
            overlay0: rgb(0x6c7086),
            surface2: rgb(0x585b70),
            surface1: rgb(0x45475a),
            surface0: rgb(0x313244),
            base: rgb(0x1e1e2e),
            base_blur: rgba(0x1e1e2edd),
            mantle: rgb(0x181825),
            crust: rgb(0x11111b),
            crust_light: rgba(0x6c708666),
            accent: get_system_accent_color(),
        }
    }
}

impl From<WindowAppearance> for ThemeMode {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
        }
    }
}
