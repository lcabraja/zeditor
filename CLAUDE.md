# Zeditor

macOS popup editor built with GPUI.

## Development Workflow

After making any changes, run `./update.sh` to build, install, and launch. If the build fails, fix the errors and retry until it succeeds.

## Key Files

- `src/main.rs` — App entry, window setup, keybindings
- `src/editor.rs` — Multi-line editor with multi-cursor support
- `src/hotkey.rs` — Global Cmd+Shift+E hotkey, menu bar icon
- `src/theme.rs` — Catppuccin Mocha theme
- `Info.plist` — App bundle config (LSUIElement for no Dock icon)

## Keybindings

- **Cmd+Shift+E** — Toggle popup (global, requires Accessibility permissions)
- **Escape** — Collapse multi-cursors, then hide popup
- **Alt+Up/Down** — Move line up/down
- **Alt+Shift+Up/Down** — Add cursor above/below
