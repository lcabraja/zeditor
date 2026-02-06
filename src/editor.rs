use std::ops::Range;
use std::time::Duration;
use std::time::Instant;

use gpui::*;
use unicode_segmentation::*;

use crate::Theme;

const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(600);
const CURSOR_FADE_DURATION: Duration = Duration::from_millis(400);
const CURSOR_ANIMATION_STEP: Duration = Duration::from_millis(16);

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

actions!(
    multi_line_editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        DocumentStart,
        DocumentEnd,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteToStart,
        DeleteWordBackward,
        Enter,
        MoveLineUp,
        MoveLineDown,
        AddCursorUp,
        AddCursorDown,
    ]
);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CursorPosition {
    pub line: usize,
    pub col: usize,
}

impl CursorPosition {
    fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

#[derive(Clone, Debug)]
pub struct Cursor {
    pub position: CursorPosition,
    pub anchor: Option<CursorPosition>,
}

impl Cursor {
    fn new(line: usize, col: usize) -> Self {
        Self {
            position: CursorPosition::new(line, col),
            anchor: None,
        }
    }

    fn selection_range(&self) -> Option<(CursorPosition, CursorPosition)> {
        let anchor = self.anchor.as_ref()?;
        if *anchor < self.position {
            Some((anchor.clone(), self.position.clone()))
        } else if *anchor > self.position {
            Some((self.position.clone(), anchor.clone()))
        } else {
            None
        }
    }

    fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    fn selection_start(&self) -> CursorPosition {
        match &self.anchor {
            Some(a) if *a < self.position => a.clone(),
            _ => self.position.clone(),
        }
    }

    fn selection_end(&self) -> CursorPosition {
        match &self.anchor {
            Some(a) if *a > self.position => a.clone(),
            _ => self.position.clone(),
        }
    }
}

pub struct MultiLineEditor {
    pub focus_handle: FocusHandle,
    pub lines: Vec<String>,
    pub cursors: Vec<Cursor>,
    pub scroll_offset: Point<Pixels>,
    pub preferred_col_x: Option<Pixels>,
    pub marked_range: Option<Range<usize>>,
    pub is_selecting: bool,
    // Layout cache for IME/mouse
    pub last_shaped_lines: Vec<ShapedLine>,
    pub last_bounds: Option<Bounds<Pixels>>,
    pub last_line_height: Pixels,
    // Cursor blink state
    pub cursor_opacity: f32,
    pub cursor_fading_in: bool,
    pub blink_epoch: usize,
    pub fade_start: Option<Instant>,
}

impl MultiLineEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let mut editor = Self {
            focus_handle,
            lines: vec![String::new()],
            cursors: vec![Cursor::new(0, 0)],
            scroll_offset: point(px(0.), px(0.)),
            preferred_col_x: None,
            marked_range: None,
            is_selecting: false,
            last_shaped_lines: Vec::new(),
            last_bounds: None,
            last_line_height: px(24.),
            cursor_opacity: 1.0,
            cursor_fading_in: true,
            blink_epoch: 0,
            fade_start: None,
        };
        editor.reset_cursor_blink(cx);
        editor
    }

    // --- Flat offset ↔ CursorPosition conversions (for IME) ---

    fn flat_text(&self) -> String {
        self.lines.join("\n")
    }

    fn flat_offset(&self, pos: &CursorPosition) -> usize {
        let mut offset = 0;
        for i in 0..pos.line.min(self.lines.len()) {
            offset += self.lines[i].len() + 1; // +1 for newline
        }
        if pos.line < self.lines.len() {
            offset += pos.col.min(self.lines[pos.line].len());
        }
        offset
    }

    fn position_from_flat(&self, offset: usize) -> CursorPosition {
        let mut remaining = offset;
        for (i, line) in self.lines.iter().enumerate() {
            if remaining <= line.len() {
                return CursorPosition::new(i, remaining);
            }
            remaining -= line.len() + 1; // +1 for newline
        }
        let last = self.lines.len().saturating_sub(1);
        CursorPosition::new(last, self.lines[last].len())
    }

    fn flat_selected_range(&self) -> Range<usize> {
        let c = &self.cursors[0];
        let start = self.flat_offset(&c.selection_start());
        let end = self.flat_offset(&c.selection_end());
        start..end
    }

    // --- Public query methods ---

    pub fn has_multiple_cursors(&self) -> bool {
        self.cursors.len() > 1
    }

    pub fn collapse_to_primary_cursor(&mut self, cx: &mut Context<Self>) {
        self.cursors.truncate(1);
        self.cursors[0].anchor = None;
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    // --- Cursor manipulation ---

    fn clamp_position(&self, pos: &CursorPosition) -> CursorPosition {
        let line = pos.line.min(self.lines.len().saturating_sub(1));
        let col = pos.col.min(self.lines[line].len());
        CursorPosition::new(line, col)
    }

    fn move_cursors_to(&mut self, pos: CursorPosition, cx: &mut Context<Self>) {
        let pos = self.clamp_position(&pos);
        self.cursors = vec![Cursor::new(pos.line, pos.col)];
        self.preferred_col_x = None;
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn select_primary_to(&mut self, pos: CursorPosition, cx: &mut Context<Self>) {
        let pos = self.clamp_position(&pos);
        let c = &mut self.cursors[0];
        if c.anchor.is_none() {
            c.anchor = Some(c.position.clone());
        }
        c.position = pos;
        cx.notify();
    }

    fn move_each_cursor<F>(&mut self, f: F, cx: &mut Context<Self>)
    where
        F: Fn(&CursorPosition, &[String]) -> CursorPosition,
    {
        for c in &mut self.cursors {
            c.position = f(&c.position, &self.lines);
            c.anchor = None;
        }
        self.merge_overlapping_cursors();
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn select_each_cursor<F>(&mut self, f: F, cx: &mut Context<Self>)
    where
        F: Fn(&CursorPosition, &[String]) -> CursorPosition,
    {
        for c in &mut self.cursors {
            if c.anchor.is_none() {
                c.anchor = Some(c.position.clone());
            }
            c.position = f(&c.position, &self.lines);
        }
        self.merge_overlapping_cursors();
        cx.notify();
    }

    fn merge_overlapping_cursors(&mut self) {
        if self.cursors.len() <= 1 {
            return;
        }
        self.cursors
            .sort_by(|a, b| a.position.cmp(&b.position));
        self.cursors.dedup_by(|a, b| {
            // If two cursors are at the same position, merge them
            if a.position == b.position {
                // Keep the wider selection
                if a.anchor.is_some() && b.anchor.is_none() {
                    b.anchor = a.anchor.clone();
                }
                true
            } else {
                false
            }
        });
    }

    // --- Navigation helpers ---

    fn prev_grapheme_boundary(line: &str, col: usize) -> usize {
        line.grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| if idx < col { Some(idx) } else { None })
            .unwrap_or(0)
    }

    fn next_grapheme_boundary(line: &str, col: usize) -> usize {
        line.grapheme_indices(true)
            .find_map(|(idx, _)| if idx > col { Some(idx) } else { None })
            .unwrap_or(line.len())
    }

    fn prev_word_boundary(line: &str, col: usize) -> usize {
        let mut prev_offset = col;
        let mut found_word = false;
        for (idx, grapheme) in line.grapheme_indices(true).rev() {
            if idx >= col {
                continue;
            }
            let is_word = grapheme
                .chars()
                .next()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
            if is_word {
                found_word = true;
                prev_offset = idx;
            } else if found_word {
                break;
            } else {
                prev_offset = idx;
            }
        }
        if found_word {
            prev_offset
        } else {
            0
        }
    }

    fn next_word_boundary(line: &str, col: usize) -> usize {
        let mut in_word = false;
        for (idx, grapheme) in line.grapheme_indices(true) {
            if idx <= col {
                continue;
            }
            let is_word = grapheme
                .chars()
                .next()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
            if is_word {
                in_word = true;
            } else if in_word {
                return idx;
            }
        }
        line.len()
    }

    fn position_left(pos: &CursorPosition, lines: &[String]) -> CursorPosition {
        if pos.col > 0 {
            CursorPosition::new(pos.line, Self::prev_grapheme_boundary(&lines[pos.line], pos.col))
        } else if pos.line > 0 {
            CursorPosition::new(pos.line - 1, lines[pos.line - 1].len())
        } else {
            pos.clone()
        }
    }

    fn position_right(pos: &CursorPosition, lines: &[String]) -> CursorPosition {
        if pos.col < lines[pos.line].len() {
            CursorPosition::new(pos.line, Self::next_grapheme_boundary(&lines[pos.line], pos.col))
        } else if pos.line + 1 < lines.len() {
            CursorPosition::new(pos.line + 1, 0)
        } else {
            pos.clone()
        }
    }

    fn position_word_left(pos: &CursorPosition, lines: &[String]) -> CursorPosition {
        if pos.col > 0 {
            CursorPosition::new(pos.line, Self::prev_word_boundary(&lines[pos.line], pos.col))
        } else if pos.line > 0 {
            CursorPosition::new(pos.line - 1, lines[pos.line - 1].len())
        } else {
            pos.clone()
        }
    }

    fn position_word_right(pos: &CursorPosition, lines: &[String]) -> CursorPosition {
        if pos.col < lines[pos.line].len() {
            CursorPosition::new(pos.line, Self::next_word_boundary(&lines[pos.line], pos.col))
        } else if pos.line + 1 < lines.len() {
            CursorPosition::new(pos.line + 1, 0)
        } else {
            pos.clone()
        }
    }

    // --- Actions ---

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let has_selection = self.cursors.iter().any(|c| c.has_selection());
        if has_selection {
            // Collapse to selection start
            for c in &mut self.cursors {
                let start = c.selection_start();
                c.position = start;
                c.anchor = None;
            }
            self.merge_overlapping_cursors();
            self.preferred_col_x = None;
            self.reset_cursor_blink(cx);
            cx.notify();
        } else {
            self.preferred_col_x = None;
            self.move_each_cursor(Self::position_left, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let has_selection = self.cursors.iter().any(|c| c.has_selection());
        if has_selection {
            for c in &mut self.cursors {
                let end = c.selection_end();
                c.position = end;
                c.anchor = None;
            }
            self.merge_overlapping_cursors();
            self.preferred_col_x = None;
            self.reset_cursor_blink(cx);
            cx.notify();
        } else {
            self.preferred_col_x = None;
            self.move_each_cursor(Self::position_right, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.select_each_cursor(Self::position_left, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.select_each_cursor(Self::position_right, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let last_line = self.lines.len() - 1;
        let last_col = self.lines[last_line].len();
        self.cursors = vec![Cursor {
            position: CursorPosition::new(last_line, last_col),
            anchor: Some(CursorPosition::new(0, 0)),
        }];
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.move_each_cursor(
            |pos, _lines| CursorPosition::new(pos.line, 0),
            cx,
        );
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.move_each_cursor(
            |pos, lines| CursorPosition::new(pos.line, lines[pos.line].len()),
            cx,
        );
    }

    fn document_start(&mut self, _: &DocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.move_cursors_to(CursorPosition::new(0, 0), cx);
    }

    fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.lines.len() - 1;
        self.preferred_col_x = None;
        self.move_cursors_to(CursorPosition::new(last, self.lines[last].len()), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.move_each_cursor(Self::position_word_left, cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.move_each_cursor(Self::position_word_right, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.select_each_cursor(Self::position_word_left, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.select_each_cursor(Self::position_word_right, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with_cursors(
            |pos, lines| {
                // If at start of line, select back to end of previous line
                if pos.col == 0 {
                    if pos.line > 0 {
                        Some((
                            CursorPosition::new(pos.line - 1, lines[pos.line - 1].len()),
                            pos.clone(),
                        ))
                    } else {
                        None
                    }
                } else {
                    let prev = Self::prev_grapheme_boundary(&lines[pos.line], pos.col);
                    Some((CursorPosition::new(pos.line, prev), pos.clone()))
                }
            },
            "",
            window,
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with_cursors(
            |pos, lines| {
                if pos.col >= lines[pos.line].len() {
                    if pos.line + 1 < lines.len() {
                        Some((pos.clone(), CursorPosition::new(pos.line + 1, 0)))
                    } else {
                        None
                    }
                } else {
                    let next = Self::next_grapheme_boundary(&lines[pos.line], pos.col);
                    Some((pos.clone(), CursorPosition::new(pos.line, next)))
                }
            },
            "",
            window,
            cx,
        );
    }

    fn delete_to_start(&mut self, _: &DeleteToStart, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with_cursors(
            |pos, _lines| {
                if pos.col > 0 {
                    Some((CursorPosition::new(pos.line, 0), pos.clone()))
                } else {
                    None
                }
            },
            "",
            window,
            cx,
        );
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with_cursors(
            |pos, lines| {
                if pos.col > 0 {
                    let prev = Self::prev_word_boundary(&lines[pos.line], pos.col);
                    Some((CursorPosition::new(pos.line, prev), pos.clone()))
                } else if pos.line > 0 {
                    Some((
                        CursorPosition::new(pos.line - 1, lines[pos.line - 1].len()),
                        pos.clone(),
                    ))
                } else {
                    None
                }
            },
            "",
            window,
            cx,
        );
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        // Insert newline at each cursor
        self.insert_text_at_cursors("\n", window, cx);
    }

    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        // Collect affected line ranges for each cursor
        let mut moved = false;
        for c in &mut self.cursors {
            let start_line = c.selection_start().line;
            if start_line > 0 {
                moved = true;
            }
        }
        if !moved {
            return;
        }

        // For simplicity with single/primary cursor
        let start_line = self.cursors[0].selection_start().line;
        let end_line = self.cursors[0].selection_end().line;

        if start_line == 0 {
            return;
        }

        let removed = self.lines.remove(start_line - 1);
        let insert_at = (end_line).min(self.lines.len());
        self.lines.insert(insert_at, removed);

        for c in &mut self.cursors {
            if c.position.line >= start_line && c.position.line <= end_line {
                c.position.line -= 1;
            }
            if let Some(ref mut a) = c.anchor
                && a.line >= start_line
                && a.line <= end_line
            {
                a.line -= 1;
            }
        }
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        let start_line = self.cursors[0].selection_start().line;
        let end_line = self.cursors[0].selection_end().line;

        if end_line + 1 >= self.lines.len() {
            return;
        }

        let removed = self.lines.remove(end_line + 1);
        self.lines.insert(start_line, removed);

        for c in &mut self.cursors {
            if c.position.line >= start_line && c.position.line <= end_line {
                c.position.line += 1;
            }
            if let Some(ref mut a) = c.anchor
                && a.line >= start_line
                && a.line <= end_line
            {
                a.line += 1;
            }
        }
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn add_cursor_up(&mut self, _: &AddCursorUp, _: &mut Window, cx: &mut Context<Self>) {
        let first = self
            .cursors
            .iter()
            .min_by_key(|c| c.position.line)
            .unwrap();
        if first.position.line == 0 {
            return;
        }
        let new_line = first.position.line - 1;
        let col = self.col_for_preferred_x(new_line, cx);
        self.cursors.push(Cursor::new(new_line, col));
        self.merge_overlapping_cursors();
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn add_cursor_down(&mut self, _: &AddCursorDown, _: &mut Window, cx: &mut Context<Self>) {
        let last = self
            .cursors
            .iter()
            .max_by_key(|c| c.position.line)
            .unwrap();
        if last.position.line + 1 >= self.lines.len() {
            return;
        }
        let new_line = last.position.line + 1;
        let col = self.col_for_preferred_x(new_line, cx);
        self.cursors.push(Cursor::new(new_line, col));
        self.merge_overlapping_cursors();
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert_text_at_cursors(&text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let c = &self.cursors[0];
        if let Some((start, end)) = c.selection_range() {
            let text = self.text_in_range(&start, &end);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let c = &self.cursors[0];
        if let Some((start, end)) = c.selection_range() {
            let text = self.text_in_range(&start, &end);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.insert_text_at_cursors("", window, cx);
        }
    }

    // --- Vertical movement ---

    fn move_vertically(&mut self, direction: i32, selecting: bool, cx: &mut Context<Self>) {
        // Ensure preferred_col_x is set from current position
        if self.preferred_col_x.is_none()
            && let Some(shaped) = self.last_shaped_lines.get(self.cursors[0].position.line)
        {
            self.preferred_col_x = Some(shaped.x_for_index(self.cursors[0].position.col));
        }

        for c in &mut self.cursors {
            let new_line = if direction < 0 {
                if c.position.line == 0 {
                    if !selecting {
                        c.position = CursorPosition::new(0, 0);
                        c.anchor = None;
                    } else {
                        if c.anchor.is_none() {
                            c.anchor = Some(c.position.clone());
                        }
                        c.position = CursorPosition::new(0, 0);
                    }
                    continue;
                }
                c.position.line - 1
            } else {
                if c.position.line + 1 >= self.lines.len() {
                    let end_col = self.lines[c.position.line].len();
                    if !selecting {
                        c.position = CursorPosition::new(c.position.line, end_col);
                        c.anchor = None;
                    } else {
                        if c.anchor.is_none() {
                            c.anchor = Some(c.position.clone());
                        }
                        c.position = CursorPosition::new(c.position.line, end_col);
                    }
                    continue;
                }
                c.position.line + 1
            };

            // Find col from preferred_col_x
            let col = if let Some(px_x) = self.preferred_col_x {
                if let Some(shaped) = self.last_shaped_lines.get(new_line) {
                    shaped.closest_index_for_x(px_x)
                } else {
                    c.position.col.min(self.lines[new_line].len())
                }
            } else {
                c.position.col.min(self.lines[new_line].len())
            };

            if selecting {
                if c.anchor.is_none() {
                    c.anchor = Some(c.position.clone());
                }
            } else {
                c.anchor = None;
            }
            c.position = CursorPosition::new(new_line, col);
        }

        self.merge_overlapping_cursors();
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn col_for_preferred_x(&self, line: usize, _cx: &mut Context<Self>) -> usize {
        if let Some(px_x) = self.preferred_col_x
            && let Some(shaped) = self.last_shaped_lines.get(line)
        {
            return shaped.closest_index_for_x(px_x);
        }
        // Fallback: use primary cursor col clamped to line length
        self.cursors[0].position.col.min(self.lines[line].len())
    }

    // --- Text extraction ---

    fn text_in_range(&self, start: &CursorPosition, end: &CursorPosition) -> String {
        if start.line == end.line {
            return self.lines[start.line][start.col..end.col].to_string();
        }
        let mut result = String::new();
        // First line
        result.push_str(&self.lines[start.line][start.col..]);
        // Middle lines
        for i in (start.line + 1)..end.line {
            result.push('\n');
            result.push_str(&self.lines[i]);
        }
        // Last line
        result.push('\n');
        result.push_str(&self.lines[end.line][..end.col]);
        result
    }

    // --- Multi-cursor edit ---

    fn insert_text_at_cursors(
        &mut self,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Sort cursors in reverse document order (bottom-first)
        let mut indexed: Vec<(usize, Cursor)> =
            self.cursors.iter().cloned().enumerate().collect();
        indexed.sort_by(|a, b| b.1.position.cmp(&a.1.position));

        let mut new_positions: Vec<(usize, CursorPosition)> = Vec::new();

        for (orig_idx, c) in &indexed {
            let (del_start, del_end) = if let Some((s, e)) = c.selection_range() {
                (s, e)
            } else {
                (c.position.clone(), c.position.clone())
            };

            let after = self.delete_range(&del_start, &del_end);
            let inserted_pos = self.insert_at(&del_start, text);
            new_positions.push((*orig_idx, inserted_pos.clone()));

            // Adjust subsequent cursor positions for the offset change
            let _ = after; // line/col shift is handled implicitly by operating bottom-first
        }

        // Rebuild cursors in original order
        new_positions.sort_by_key(|(idx, _)| *idx);
        self.cursors = new_positions
            .into_iter()
            .map(|(_, pos)| Cursor::new(pos.line, pos.col))
            .collect();

        self.merge_overlapping_cursors();
        self.marked_range = None;
        self.preferred_col_x = None;
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn edit_with_cursors<F>(
        &mut self,
        expand_fn: F,
        replacement: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: Fn(&CursorPosition, &[String]) -> Option<(CursorPosition, CursorPosition)>,
    {
        // For cursors without selection, expand using expand_fn
        for c in &mut self.cursors {
            if !c.has_selection()
                && let Some((start, end)) = expand_fn(&c.position, &self.lines)
            {
                c.anchor = Some(start);
                c.position = end;
                // Normalize so anchor < position
                let s = c.selection_start();
                let e = c.selection_end();
                c.anchor = Some(s);
                c.position = e;
            }
        }
        self.insert_text_at_cursors(replacement, window, cx);
    }

    // --- Low-level text mutation ---

    /// Delete a range and return the deleted text
    fn delete_range(&mut self, start: &CursorPosition, end: &CursorPosition) -> String {
        if start == end {
            return String::new();
        }
        let deleted = self.text_in_range(start, end);

        if start.line == end.line {
            self.lines[start.line] = format!(
                "{}{}",
                &self.lines[start.line][..start.col],
                &self.lines[start.line][end.col..]
            );
        } else {
            let new_line = format!(
                "{}{}",
                &self.lines[start.line][..start.col],
                &self.lines[end.line][end.col..]
            );
            // Remove lines from start.line+1 to end.line (inclusive)
            for _ in start.line + 1..=end.line {
                self.lines.remove(start.line + 1);
            }
            self.lines[start.line] = new_line;
        }

        deleted
    }

    /// Insert text at position, return new cursor position after insert
    fn insert_at(&mut self, pos: &CursorPosition, text: &str) -> CursorPosition {
        if text.is_empty() {
            return pos.clone();
        }

        let insert_lines: Vec<&str> = text.split('\n').collect();

        if insert_lines.len() == 1 {
            // Single-line insert
            self.lines[pos.line].insert_str(pos.col, text);
            return CursorPosition::new(pos.line, pos.col + text.len());
        }

        // Multi-line insert
        let after_cursor = self.lines[pos.line][pos.col..].to_string();
        self.lines[pos.line] = format!("{}{}", &self.lines[pos.line][..pos.col], insert_lines[0]);

        for (i, segment) in insert_lines[1..].iter().enumerate() {
            if i == insert_lines.len() - 2 {
                // Last segment — append the text that was after the cursor
                self.lines
                    .insert(pos.line + 1 + i, format!("{}{}", segment, after_cursor));
            } else {
                self.lines
                    .insert(pos.line + 1 + i, segment.to_string());
            }
        }

        let new_line = pos.line + insert_lines.len() - 1;
        let new_col = insert_lines.last().unwrap().len();
        CursorPosition::new(new_line, new_col)
    }

    // --- Mouse ---

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        let pos = self.position_for_mouse(event.position);
        if event.modifiers.shift {
            self.select_primary_to(pos, cx);
        } else {
            self.move_cursors_to(pos, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let pos = self.position_for_mouse(event.position);
            self.select_primary_to(pos, cx);
        }
    }

    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta_y = match event.delta {
            ScrollDelta::Pixels(d) => -d.y,
            ScrollDelta::Lines(d) => -d.y * self.last_line_height,
        };
        self.scroll_offset.y += delta_y;
        self.clamp_scroll();
        cx.notify();
    }

    fn position_for_mouse(&self, point: Point<Pixels>) -> CursorPosition {
        let bounds = match &self.last_bounds {
            Some(b) => b,
            None => return CursorPosition::new(0, 0),
        };

        let y = point.y - bounds.top() + self.scroll_offset.y;
        let line = if y < px(0.) {
            0
        } else {
            let l = (y / self.last_line_height) as usize;
            l.min(self.lines.len().saturating_sub(1))
        };

        let col = if let Some(shaped) = self.last_shaped_lines.get(line) {
            shaped.closest_index_for_x(point.x - bounds.left() + self.scroll_offset.x)
        } else {
            0
        };

        CursorPosition::new(line, col)
    }

    fn clamp_scroll(&mut self) {
        if self.scroll_offset.y < px(0.) {
            self.scroll_offset.y = px(0.);
        }
        // Clamp to max when we know bounds
        if let Some(bounds) = &self.last_bounds {
            let total = self.last_line_height * self.lines.len();
            let max = (total - bounds.size.height).max(px(0.));
            if self.scroll_offset.y > max {
                self.scroll_offset.y = max;
            }
        }
    }

    fn scroll_to_cursor(&mut self) {
        let bounds = match &self.last_bounds {
            Some(b) => b,
            None => return,
        };
        let cursor_y = self.last_line_height * self.cursors[0].position.line;
        let visible_top = self.scroll_offset.y;
        let visible_bottom = visible_top + bounds.size.height - self.last_line_height;

        if cursor_y < visible_top {
            self.scroll_offset.y = cursor_y;
        } else if cursor_y > visible_bottom {
            self.scroll_offset.y = cursor_y - bounds.size.height + self.last_line_height;
        }
        self.clamp_scroll();
    }

    // --- Cursor blink ---

    fn reset_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_opacity = 1.0;
        self.cursor_fading_in = true;
        self.fade_start = None;
        self.blink_epoch += 1;
        let epoch = self.blink_epoch;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(CURSOR_BLINK_INTERVAL)
                .await;

            loop {
                let fading_in = this
                    .update(cx, |this, cx| {
                        if this.blink_epoch != epoch {
                            return None;
                        }
                        this.cursor_fading_in = !this.cursor_fading_in;
                        this.fade_start = Some(Instant::now());
                        cx.notify();
                        Some(this.cursor_fading_in)
                    })
                    .ok()
                    .flatten();

                let Some(fading_in) = fading_in else {
                    break;
                };

                let fade_steps = (CURSOR_FADE_DURATION.as_millis()
                    / CURSOR_ANIMATION_STEP.as_millis())
                    as usize;
                for _ in 0..fade_steps {
                    cx.background_executor()
                        .timer(CURSOR_ANIMATION_STEP)
                        .await;
                    let should_continue = this
                        .update(cx, |this, cx| {
                            if this.blink_epoch != epoch {
                                return false;
                            }
                            if let Some(start) = this.fade_start {
                                let elapsed = start.elapsed().as_secs_f32();
                                let progress =
                                    (elapsed / CURSOR_FADE_DURATION.as_secs_f32()).min(1.0);
                                let eased = ease_in_out_cubic(progress);
                                this.cursor_opacity =
                                    if fading_in { eased } else { 1.0 - eased };
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !should_continue {
                        return;
                    }
                }

                let should_continue = this
                    .update(cx, |this, cx| {
                        if this.blink_epoch != epoch {
                            return false;
                        }
                        this.cursor_opacity = if fading_in { 1.0 } else { 0.0 };
                        this.fade_start = None;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !should_continue {
                    break;
                }

                let remaining = CURSOR_BLINK_INTERVAL.saturating_sub(CURSOR_FADE_DURATION);
                if !remaining.is_zero() {
                    cx.background_executor().timer(remaining).await;
                }
            }
        })
        .detach();
    }

    // --- UTF-16 conversions for IME ---

    fn offset_to_utf16(text: &str, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in text.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn offset_from_utf16(text: &str, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in text.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
        Self::offset_to_utf16(text, range.start)..Self::offset_to_utf16(text, range.end)
    }

    fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
        Self::offset_from_utf16(text, range.start)..Self::offset_from_utf16(text, range.end)
    }
}

// --- EntityInputHandler for IME ---

impl EntityInputHandler for MultiLineEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let flat = self.flat_text();
        let range = Self::range_from_utf16(&flat, &range_utf16);
        actual_range.replace(Self::range_to_utf16(&flat, &range));
        Some(flat[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let flat = self.flat_text();
        let range = self.flat_selected_range();
        let c = &self.cursors[0];
        let reversed = c
            .anchor
            .as_ref()
            .map(|a| *a > c.position)
            .unwrap_or(false);
        Some(UTF16Selection {
            range: Self::range_to_utf16(&flat, &range),
            reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let flat = self.flat_text();
        self.marked_range
            .as_ref()
            .map(|range| Self::range_to_utf16(&flat, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flat = self.flat_text();
        let range = range_utf16
            .as_ref()
            .map(|r| Self::range_from_utf16(&flat, r))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.flat_selected_range());

        let start_pos = self.position_from_flat(range.start);
        let end_pos = self.position_from_flat(range.end);

        self.delete_range(&start_pos, &end_pos);
        let new_pos = self.insert_at(&start_pos, new_text);

        self.cursors = vec![Cursor::new(new_pos.line, new_pos.col)];
        self.marked_range = None;
        self.preferred_col_x = None;
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flat = self.flat_text();
        let range = range_utf16
            .as_ref()
            .map(|r| Self::range_from_utf16(&flat, r))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| self.flat_selected_range());

        let start_pos = self.position_from_flat(range.start);
        let end_pos = self.position_from_flat(range.end);

        self.delete_range(&start_pos, &end_pos);
        let new_end = self.insert_at(&start_pos, new_text);

        let mark_start = self.flat_offset(&start_pos);
        let mark_end = self.flat_offset(&new_end);
        self.marked_range = Some(mark_start..mark_end);

        if let Some(sel_utf16) = new_selected_range_utf16 {
            let new_flat = self.flat_text();
            let sel = Self::range_from_utf16(&new_flat, &sel_utf16);
            let sel_start = self.position_from_flat(sel.start + mark_start);
            let sel_end = self.position_from_flat(sel.end + mark_start);
            if sel_start == sel_end {
                self.cursors = vec![Cursor::new(sel_start.line, sel_start.col)];
            } else {
                self.cursors = vec![Cursor {
                    position: CursorPosition::new(sel_end.line, sel_end.col),
                    anchor: Some(CursorPosition::new(sel_start.line, sel_start.col)),
                }];
            }
        } else {
            self.cursors = vec![Cursor::new(new_end.line, new_end.col)];
        }

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let flat = self.flat_text();
        let range = Self::range_from_utf16(&flat, &range_utf16);
        let start_pos = self.position_from_flat(range.start);
        let end_pos = self.position_from_flat(range.end);

        let start_x = self
            .last_shaped_lines
            .get(start_pos.line)
            .map(|l| l.x_for_index(start_pos.col))
            .unwrap_or(px(0.));
        let end_x = self
            .last_shaped_lines
            .get(end_pos.line)
            .map(|l| l.x_for_index(end_pos.col))
            .unwrap_or(px(0.));

        let top = bounds.top() + self.last_line_height * start_pos.line - self.scroll_offset.y;
        let bottom = top + self.last_line_height * (end_pos.line - start_pos.line + 1);

        Some(Bounds::from_corners(
            point(bounds.left() + start_x, top),
            point(bounds.left() + end_x, bottom),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.last_bounds.as_ref()?;
        let pos = self.position_for_mouse(point);
        let flat = self.flat_text();
        let offset = self.flat_offset(&pos);
        Some(Self::offset_to_utf16(&flat, offset))
    }
}

// --- Render ---

impl Render for MultiLineEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .flex()
            .key_context("MultiLineEditor")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_to_start))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::add_cursor_up))
            .on_action(cx.listener(Self::add_cursor_down))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .bg(theme.surface0)
            .size_full()
            .overflow_hidden()
            .line_height(px(24.))
            .text_size(px(16.))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .overflow_hidden()
                    .p(px(8.))
                    .child(MultiLineTextElement {
                        input: cx.entity().clone(),
                    }),
            )
    }
}

impl Focusable for MultiLineEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// --- Element ---

struct MultiLineTextElement {
    input: Entity<MultiLineEditor>,
}

struct MultiLinePrepaintState {
    shaped_lines: Vec<ShapedLine>,
    cursors: Vec<(Bounds<Pixels>, Rgba)>,
    cursor_opacity: f32,
    selections: Vec<PaintQuad>,
    scroll_offset: Point<Pixels>,
    line_height: Pixels,
}

impl IntoElement for MultiLineTextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MultiLineTextElement {
    type RequestLayoutState = ();
    type PrepaintState = MultiLinePrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let line_count = input.lines.len().max(1);
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * line_count).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let theme = cx.global::<Theme>();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let scroll_offset = input.scroll_offset;
        let cursor_opacity = input.cursor_opacity;

        // Shape all lines
        let mut shaped_lines = Vec::with_capacity(input.lines.len());
        for line_text in &input.lines {
            let display_text: SharedString = if line_text.is_empty() {
                " ".into() // shape at least a space for correct height
            } else {
                line_text.clone().into()
            };
            let run = TextRun {
                len: display_text.len(),
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window
                .text_system()
                .shape_line(display_text, font_size, &[run], None);
            shaped_lines.push(shaped);
        }

        // Build cursor rects
        let mut cursor_rects = Vec::new();
        let is_focused = input.focus_handle.is_focused(window);
        if is_focused {
            for c in &input.cursors {
                if !c.has_selection() {
                    let x = shaped_lines
                        .get(c.position.line)
                        .map(|l| l.x_for_index(c.position.col))
                        .unwrap_or(px(0.));
                    let y = line_height * c.position.line;
                    cursor_rects.push((
                        Bounds::new(
                            point(
                                bounds.left() + x - scroll_offset.x,
                                bounds.top() + y - scroll_offset.y,
                            ),
                            size(px(2.), line_height),
                        ),
                        theme.accent,
                    ));
                }
            }
        }

        // Build selection rects
        let mut selections = Vec::new();
        for c in &input.cursors {
            if let Some((start, end)) = c.selection_range() {
                for line_idx in start.line..=end.line {
                    let col_start = if line_idx == start.line {
                        start.col
                    } else {
                        0
                    };
                    let col_end = if line_idx == end.line {
                        end.col
                    } else {
                        input.lines[line_idx].len()
                    };

                    let x_start = shaped_lines
                        .get(line_idx)
                        .map(|l| l.x_for_index(col_start))
                        .unwrap_or(px(0.));
                    let x_end = shaped_lines
                        .get(line_idx)
                        .map(|l| l.x_for_index(col_end))
                        .unwrap_or(px(0.));
                    let y = line_height * line_idx;

                    selections.push(fill(
                        Bounds::from_corners(
                            point(
                                bounds.left() + x_start - scroll_offset.x,
                                bounds.top() + y - scroll_offset.y,
                            ),
                            point(
                                bounds.left() + x_end - scroll_offset.x,
                                bounds.top() + y + line_height - scroll_offset.y,
                            ),
                        ),
                        rgba(0x3311ff30),
                    ));
                }

                // Also show cursor at end of selection
                if is_focused {
                    let x = shaped_lines
                        .get(c.position.line)
                        .map(|l| l.x_for_index(c.position.col))
                        .unwrap_or(px(0.));
                    let y = line_height * c.position.line;
                    cursor_rects.push((
                        Bounds::new(
                            point(
                                bounds.left() + x - scroll_offset.x,
                                bounds.top() + y - scroll_offset.y,
                            ),
                            size(px(2.), line_height),
                        ),
                        theme.accent,
                    ));
                }
            }
        }

        MultiLinePrepaintState {
            shaped_lines,
            cursors: cursor_rects,
            cursor_opacity,
            selections,
            scroll_offset,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        // Paint selections
        for sel in prepaint.selections.drain(..) {
            window.paint_quad(sel);
        }

        // Paint lines
        let line_height = prepaint.line_height;
        let scroll_offset = prepaint.scroll_offset;

        for (i, shaped) in prepaint.shaped_lines.iter().enumerate() {
            let y = bounds.top() + line_height * i - scroll_offset.y;
            // Skip lines outside visible bounds
            if y + line_height < bounds.top() || y > bounds.bottom() {
                continue;
            }
            let origin = point(bounds.left() - scroll_offset.x, y);
            shaped
                .paint(origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
        }

        // Paint cursors
        let opacity = prepaint.cursor_opacity;
        if opacity > 0.0 && focus_handle.is_focused(window) {
            for (cursor_bounds, cursor_color) in &prepaint.cursors {
                let hsla: Hsla = (*cursor_color).into();
                let color_with_opacity = Hsla {
                    h: hsla.h,
                    s: hsla.s,
                    l: hsla.l,
                    a: opacity,
                };
                window.paint_quad(fill(*cursor_bounds, color_with_opacity));
            }
        }

        // Update cached layout info
        let shaped_lines: Vec<ShapedLine> = prepaint.shaped_lines.drain(..).collect();
        self.input.update(cx, |input, _cx| {
            input.last_shaped_lines = shaped_lines;
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            input.scroll_to_cursor();
        });
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
}
