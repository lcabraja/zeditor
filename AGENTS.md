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

---

## GPUI Framework Guide

### Core Concepts

GPUI is Zed's GPU-accelerated UI framework. It uses:
- **Entity-Component architecture** — State lives in `Entity<T>`, accessed via `Context`
- **Trait-based rendering** — Components implement `Render` trait
- **Builder pattern** — UI built by chaining methods on element builders
- **Action system** — Keybindings route to named actions

### Key Types

| Type | Purpose |
|------|---------|
| `Entity<T>` | Type-safe reference to a stateful component |
| `Context<T>` | Mutable context for component operations |
| `Window` | Window state, text system, rendering operations |
| `FocusHandle` | Identifies which component receives keyboard input |
| `Rgba` | Color (use `rgb(0xRRGGBB)` or `rgba(0xRRGGBBAA)`) |
| `Bounds<Pixels>` | Rectangle with origin and size |
| `Point<Pixels>` | 2D coordinate |
| `px(f32)` | Convert float to Pixels |

### Creating a Component

```rust
pub struct MyComponent {
    state: String,
    focus_handle: FocusHandle,
}

impl MyComponent {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: String::new(),
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Render for MyComponent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .track_focus(&self.focus_handle)
            .key_context("MyComponent")  // For keybinding routing
            .bg(theme.base)
            .text_color(theme.text)
            .child("Hello")
    }
}

impl Focusable for MyComponent {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
```

### State Access Patterns

```rust
// Read-only access to another entity
let editor = self.editor.read(cx);
let text = &editor.lines;

// Mutable update to another entity
self.editor.update(cx, |editor, cx| {
    editor.insert_text("hello", cx);
});

// Get reference to self as entity (for passing to child elements)
let self_entity = cx.entity().clone();

// Trigger re-render after state change
cx.notify();
```

### Element Builder Methods

**Layout:**
- `flex()` / `flex_col()` / `flex_row()` — Flexbox layout
- `w_full()` / `h_full()` / `size_full()` — Fill parent
- `w(px(100.))` / `h(px(50.))` — Fixed size
- `flex_1()` — Flex grow
- `overflow_hidden()` / `overflow_scroll()` — Overflow behavior

**Spacing:**
- `p(px(8.))` / `px_2()` — Padding (all sides / horizontal)
- `m(px(4.))` — Margin

**Styling:**
- `bg(color)` — Background color
- `text_color(color)` — Text color
- `text_size(px(14.))` — Font size
- `border_1()` / `border_color(color)` — Border
- `rounded(px(4.))` — Border radius
- `cursor(CursorStyle::IBeam)` — Cursor style

**Content:**
- `child(element)` — Add single child
- `children(vec![...])` — Add multiple children

**Events:**
- `on_action(cx.listener(Self::handler))` — Action handler
- `on_mouse_down(button, cx.listener(Self::handler))` — Mouse events
- `on_click(cx.listener(Self::handler))` — Click handler

### Defining Actions and Keybindings

```rust
// In module, define actions
actions!(my_component, [Save, Cancel, DoThing]);

// In main.rs, bind keys
cx.bind_keys([
    KeyBinding::new("cmd-s", Save, Some("MyComponent")),
    KeyBinding::new("escape", Cancel, Some("MyComponent")),
    KeyBinding::new("cmd-shift-d", DoThing, Some("MyComponent")),
]);

// In component, handle actions
impl Render for MyComponent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("MyComponent")  // Must match keybinding context
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
    }
}

impl MyComponent {
    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        // Handle save
        cx.notify();
    }
}
```

**Modifier keys:** `cmd`, `alt`, `shift`, `ctrl`
**Key format:** `"modifier-key"` or `"mod1-mod2-key"` (e.g., `"cmd-shift-s"`)

### Global State (Theme)

```rust
// Define global
pub struct Theme {
    pub text: Rgba,
    pub base: Rgba,
}
impl Global for Theme {}

// Initialize in main
app.set_global(Theme { ... });

// Use in any component
let theme = cx.global::<Theme>();
```

### Clipboard

```rust
// Read
if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
    self.insert_text(&text, cx);
}

// Write
cx.write_to_clipboard(ClipboardItem::new_string(text));
```

### Window Operations

```rust
// In action handler with Window parameter:
fn handler(&mut self, _: &Action, window: &mut Window, cx: &mut Context<Self>) {
    // Show emoji picker
    window.show_character_palette();

    // Get text metrics
    let line_height = window.line_height();
    let style = window.text_style();

    // Shape text for custom rendering
    let shaped = window.text_system().shape_line(text, font_size, &runs, None);
}
```

### Custom Element (Advanced)

For custom rendering (text editors, canvas), implement `Element` trait:

```rust
struct CustomElement { /* ... */ }

impl Element for CustomElement {
    type RequestLayoutState = ();
    type PrepaintState = MyPrepaintData;

    // Phase 1: Request layout space
    fn request_layout(&mut self, ..., window: &mut Window, cx: &mut App)
        -> (LayoutId, Self::RequestLayoutState)
    {
        let style = Style::default();
        style.size.width = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    // Phase 2: Prepare paint data (shape text, compute positions)
    fn prepaint(&mut self, ..., bounds: Bounds<Pixels>, ...) -> Self::PrepaintState {
        // Calculate what to draw
    }

    // Phase 3: Actually paint
    fn paint(&mut self, ..., bounds: Bounds<Pixels>, prepaint: &mut Self::PrepaintState, ...) {
        // Draw rectangles, text, etc.
        window.paint_quad(PaintQuad {
            bounds: cursor_bounds,
            background: Some(color),
            ..Default::default()
        });
        shaped_line.paint(origin, line_height, TextAlign::Left, None, window, cx);
    }
}
```

### Common Patterns in This Codebase

**Multi-cursor editing:**
```rust
for cursor in &mut self.cursors {
    cursor.position = new_pos;
}
self.merge_overlapping_cursors();
cx.notify();
```

**Animation loop:**
```rust
cx.request_animation_frame();  // Request next frame

fn on_animation_frame(&mut self, _: &AnimationFrame, cx: &mut Context<Self>) {
    self.update_animation();
    cx.notify();
    if self.animating {
        cx.request_animation_frame();
    }
}
```

**Entity in child element:**
```rust
// Pass entity to custom element for state access
child(MultiLineTextElement {
    input: cx.entity().clone(),
})
```

### Adding a New Feature Checklist

1. **New action:** Add to `actions!()`, bind key in `main.rs`, add handler
2. **New UI element:** Use builder pattern in `render()`
3. **New state:** Add field to struct, update in handlers, call `cx.notify()`
4. **New component:** Implement `Render`, optionally `Focusable`
5. **Custom rendering:** Implement `Element` trait with 3 phases
