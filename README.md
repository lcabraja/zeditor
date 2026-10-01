# Zeditor

A macOS menu bar popup editor built with Rust and GPUI. Open it with
Option+Cmd+V, edit clipboard text, then use Cmd+Enter to paste into the previous
application. The popup also accepts text as command-line arguments or piped input.

## Development

Install Rust with support for edition 2024 and the Xcode command-line tools.
Run these commands from the repository:

```sh
cargo clippy --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests -v
./update.sh
```

`bundle.sh` builds the locked release dependencies and creates
`target/Zeditor.app`, including its icon. Both build scripts work when invoked
from another directory and respect `CARGO_TARGET_DIR`.

`update.sh` signs and verifies a staged copy before stopping Zeditor and replacing
the installed application in `~/Applications`. It uses the `Zeditor` signing
certificate from your keychain. Keep this identity stable across rebuilds to
preserve Accessibility permissions. A signing failure leaves the installed app
and running process untouched.

Override the installation directory or signing identity when needed:

```sh
ZEDITOR_INSTALL_DIR="$HOME/Applications" \
ZEDITOR_SIGNING_IDENTITY="Zeditor" ./update.sh
```

Grant Zeditor Accessibility access in macOS System Settings when prompted so it
can send the paste keystroke to the previous application.

## Shortcuts

| Shortcut | Action |
| --- | --- |
| Option+Cmd+V | Toggle popup globally; configurable in preferences |
| Cmd+Enter | Submit text and paste into the previous application |
| Cmd+Shift+V | Search Alfred clipboard history while the popup is active |
| Escape | Collapse multiple cursors; with one cursor, hide the popup |
| Cmd+, | Open preferences |
| Option+Up / Down | Move the current line |
| Cmd+Option+Up / Down | Add a cursor above / below |
| Cmd+Q | Quit |

Record a new shortcut in preferences, then click Save. Registration or file
errors stay visible in the preferences window and leave the previous shortcut
active. Preference files are replaced atomically to avoid partial JSON writes.

## Alfred clipboard history

With the popup focused, press Cmd+Shift+V or click History in the header.
Type to search the full retained history, use Up/Down to choose a clip, then
press Enter or double-click it to insert its text at the editor's cursors. Selected
text is replaced. Multi-cursor insertion follows the same rules as Cmd+V.
Edit the inserted text, then press Cmd+Enter to paste into the previous app.
Escape cancels the picker and restores the existing editor state.

The left pane shows source-app icons and a full-width selection. Single-click
to preview an item in the right pane. Search matches are highlighted in the
list and full-text preview. Cmd+1 through Cmd+9 stay assigned to the first nine
search results. If the target row is more than 85% visible, the shortcut inserts
it. Otherwise it scrolls to that row; press the shortcut again to insert it.
The selected editable row shows a Return indicator. Word count, character
count, and the copy time appear below the preview. The search field has no line
number gutter.

The first screenful loads immediately, followed by batches of 200. Search
updates as each batch arrives, with a spinner below the last result while
loading. Image rows and their shortcuts appear dimmed.

The picker reads Alfred's local clipboard database without modifying it or
changing the system clipboard. File entries insert their full paths, one per
line. Images can be previewed but cannot be inserted into the text editor.
The database format is internal to Alfred and may change
in future versions. Missing history or read errors appear inside the picker.

Intercepting Alfred's global Cmd+Shift+V shortcut requires Zeditor's existing
Accessibility permission. Outside the focused popup, including in preferences,
the shortcut passes through to Alfred. The History button works even if macOS
prevents shortcut interception.

## Files and launchers

Configuration lives in `~/Library/Application Support/Zeditor/config.json`.
Diagnostic events go to `zeditor.log` in the same directory. Malformed
configuration is retained for recovery while the application uses defaults.

```sh
target/release/zeditor "Text to edit"
printf 'First line\nSecond line' | target/release/zeditor
```

`scripts/zeditor-alfred.sh` accepts an Alfred keyword argument. It checks PATH,
the repository release binary, `~/Applications/Zeditor.app`, and
`/Applications/Zeditor.app`, in that order.
