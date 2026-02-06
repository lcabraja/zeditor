# Zeditor

macOS popup editor built with GPUI.

## Development Workflow

1. Make changes
2. Run `cargo clippy` to lint
3. If clippy reports warnings/errors, fix them and go to step 2
4. Run `./update.sh` to build, install, and launch
5. If build fails with errors not caught by clippy, fix them and go to step 2
6. Add any application state files to `.gitignore` if created
7. Commit changes

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
