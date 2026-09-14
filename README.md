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
| Escape | Collapse multiple cursors; with one cursor, hide the popup |
| Cmd+, | Open preferences |
| Option+Up / Down | Move the current line |
| Cmd+Option+Up / Down | Add a cursor above / below |
| Cmd+Q | Quit |

Record a new shortcut in preferences, then click Save. Registration or file
errors stay visible in the preferences window and leave the previous shortcut
active. Preference files are replaced atomically to avoid partial JSON writes.

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
