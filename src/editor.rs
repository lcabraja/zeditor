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
        SubmitAndPaste,
        SelectHome,
        SelectEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        ToggleWordWrap,
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
    pub word_wrap: bool,
    // Layout cache for IME/mouse
    pub last_shaped_lines: Vec<ShapedLine>,
    pub last_wrapped_lines: Vec<WrappedLine>,
    pub last_bounds: Option<Bounds<Pixels>>,
    pub last_line_height: Pixels,
    pub last_max_line_width: Pixels,
    /// Number of visual lines per logical line (1 when not wrapped)
    pub last_visual_line_counts: Vec<usize>,
    /// Set when cursor moves; cleared after paint applies scroll_to_cursor
    pub needs_scroll_to_cursor: bool,
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
            word_wrap: false,
            last_shaped_lines: Vec::new(),
            last_wrapped_lines: Vec::new(),
            last_bounds: None,
            last_line_height: px(24.),
            last_max_line_width: px(0.),
            last_visual_line_counts: Vec::new(),
            needs_scroll_to_cursor: false,
            cursor_opacity: 1.0,
            cursor_fading_in: true,
            blink_epoch: 0,
            fade_start: None,
        };
        editor.reset_cursor_blink(cx);
        editor
    }

    /// Reset editor contents with the given text, or empty if None.
    pub fn reset_with_text(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        if let Some(text) = text {
            let new_lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
            let last_line = new_lines.len() - 1;
            let last_col = new_lines[last_line].len();
            self.lines = new_lines;
            self.cursors = vec![Cursor {
                position: CursorPosition::new(last_line, last_col),
                anchor: Some(CursorPosition::new(0, 0)),
            }];
        } else {
            self.lines = vec![String::new()];
            self.cursors = vec![Cursor::new(0, 0)];
        }

        self.scroll_offset = point(px(0.), px(0.));
        self.preferred_col_x = None;
        self.marked_range = None;
        self.reset_cursor_blink(cx);
        cx.notify();
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
        self.needs_scroll_to_cursor = true;
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
        self.needs_scroll_to_cursor = true;
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
        self.needs_scroll_to_cursor = true;
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
        self.needs_scroll_to_cursor = true;
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
            self.needs_scroll_to_cursor = true;
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
            self.needs_scroll_to_cursor = true;
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

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.select_each_cursor(
            |pos, _lines| CursorPosition::new(pos.line, 0),
            cx,
        );
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        self.select_each_cursor(
            |pos, lines| CursorPosition::new(pos.line, lines[pos.line].len()),
            cx,
        );
    }

    fn select_document_start(&mut self, _: &SelectDocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.preferred_col_x = None;
        let pos = CursorPosition::new(0, 0);
        for c in &mut self.cursors {
            if c.anchor.is_none() {
                c.anchor = Some(c.position.clone());
            }
            c.position = pos.clone();
        }
        self.merge_overlapping_cursors();
        self.needs_scroll_to_cursor = true;
        cx.notify();
    }

    fn select_document_end(&mut self, _: &SelectDocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.lines.len() - 1;
        let last_col = self.lines[last].len();
        self.preferred_col_x = None;
        let pos = CursorPosition::new(last, last_col);
        for c in &mut self.cursors {
            if c.anchor.is_none() {
                c.anchor = Some(c.position.clone());
            }
            c.position = pos.clone();
        }
        self.merge_overlapping_cursors();
        self.needs_scroll_to_cursor = true;
        cx.notify();
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
        self.needs_scroll_to_cursor = true;
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
        self.needs_scroll_to_cursor = true;
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

    /// Get the text to submit/paste.
    /// - If any cursor has a selection, join all selected texts
    ///   (same line = space separator, different lines = newline separator)
    /// - If no selections, return all editor text
    pub fn get_submit_text(&self) -> String {
        // Check if any cursor has a selection
        let has_any_selection = self.cursors.iter().any(|c| c.has_selection());

        if !has_any_selection {
            // No selections - return entire editor content
            return self.lines.join("\n");
        }

        // Collect all selections sorted by position
        let mut selections: Vec<(CursorPosition, CursorPosition)> = self
            .cursors
            .iter()
            .filter_map(|c| c.selection_range())
            .collect();
        selections.sort_by(|a, b| a.0.cmp(&b.0));

        // Join selections: same line = space, different lines = newline
        let mut result = String::new();
        let mut last_line: Option<usize> = None;

        for (start, end) in selections {
            let text = self.text_in_range(&start, &end);

            if let Some(prev_line) = last_line {
                if start.line == prev_line {
                    // Same line as previous selection - join with space
                    result.push(' ');
                } else {
                    // Different line - join with newline
                    result.push('\n');
                }
            }

            result.push_str(&text);
            last_line = Some(end.line);
        }

        result
    }

    // --- Layout helpers (abstract over wrapped/unwrapped) ---

    fn x_for_index_in_line(&self, line: usize, col: usize) -> Pixels {
        if self.word_wrap {
            self.last_wrapped_lines.get(line)
                .map(|wl| wl.unwrapped_layout.x_for_index(col))
                .unwrap_or(px(0.))
        } else {
            self.last_shaped_lines.get(line)
                .map(|l| l.x_for_index(col))
                .unwrap_or(px(0.))
        }
    }

    fn closest_index_for_x_in_line(&self, line: usize, x: Pixels) -> usize {
        if self.word_wrap {
            self.last_wrapped_lines.get(line)
                .map(|wl| wl.unwrapped_layout.closest_index_for_x(x))
                .unwrap_or(0)
        } else {
            self.last_shaped_lines.get(line)
                .map(|l| l.closest_index_for_x(x))
                .unwrap_or(0)
        }
    }

    // --- Vertical movement ---

    fn move_vertically(&mut self, direction: i32, selecting: bool, cx: &mut Context<Self>) {
        // Ensure preferred_col_x is set from current position
        if self.preferred_col_x.is_none() {
            self.preferred_col_x = Some(self.x_for_index_in_line(
                self.cursors[0].position.line,
                self.cursors[0].position.col,
            ));
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
                if self.word_wrap {
                    self.last_wrapped_lines.get(new_line)
                        .map(|wl| wl.unwrapped_layout.closest_index_for_x(px_x))
                        .unwrap_or(0)
                } else {
                    self.last_shaped_lines.get(new_line)
                        .map(|l| l.closest_index_for_x(px_x))
                        .unwrap_or(c.position.col.min(self.lines[new_line].len()))
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
        self.needs_scroll_to_cursor = true;
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn col_for_preferred_x(&self, line: usize, _cx: &mut Context<Self>) -> usize {
        if let Some(px_x) = self.preferred_col_x {
            return self.closest_index_for_x_in_line(line, px_x);
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
        self.needs_scroll_to_cursor = true;
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

    fn toggle_word_wrap(&mut self, _: &ToggleWordWrap, _: &mut Window, cx: &mut Context<Self>) {
        self.word_wrap = !self.word_wrap;
        self.scroll_offset.x = px(0.);
        cx.notify();
    }

    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (delta_x, delta_y) = match event.delta {
            ScrollDelta::Pixels(d) => (-d.x, -d.y),
            ScrollDelta::Lines(d) => (-d.x * self.last_line_height, -d.y * self.last_line_height),
        };
        self.scroll_offset.y += delta_y;
        if !self.word_wrap {
            self.scroll_offset.x += delta_x;
        }
        self.clamp_scroll();
        cx.notify();
    }

    fn position_for_mouse(&self, point: Point<Pixels>) -> CursorPosition {
        let bounds = match &self.last_bounds {
            Some(b) => b,
            None => return CursorPosition::new(0, 0),
        };

        let y = point.y - bounds.top() + self.scroll_offset.y;

        if self.word_wrap {
            // Find which logical line this visual Y falls into
            let mut visual_y = px(0.);
            for (line_idx, &count) in self.last_visual_line_counts.iter().enumerate() {
                let line_visual_height = self.last_line_height * count;
                if y < visual_y + line_visual_height {
                    // Mouse is within this logical line's visual area
                    let local_y = y - visual_y;
                    let local_pos = Point::new(point.x - bounds.left(), local_y);
                    if let Some(wl) = self.last_wrapped_lines.get(line_idx) {
                        let col = match wl.closest_index_for_position(local_pos, self.last_line_height) {
                            Ok(idx) | Err(idx) => idx,
                        };
                        return CursorPosition::new(line_idx, col);
                    }
                    return CursorPosition::new(line_idx, 0);
                }
                visual_y += line_visual_height;
            }
            // Past the end
            let last = self.lines.len().saturating_sub(1);
            CursorPosition::new(last, self.lines[last].len())
        } else {
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
    }

    fn clamp_scroll(&mut self) {
        if self.scroll_offset.y < px(0.) {
            self.scroll_offset.y = px(0.);
        }
        if self.scroll_offset.x < px(0.) {
            self.scroll_offset.x = px(0.);
        }
        if let Some(bounds) = &self.last_bounds {
            // Vertical: total visual lines * line_height
            let total_visual_lines: usize = if self.word_wrap {
                self.last_visual_line_counts.iter().sum()
            } else {
                self.lines.len()
            };
            let total_y = self.last_line_height * total_visual_lines;
            let max_y = (total_y - bounds.size.height).max(px(0.));
            if self.scroll_offset.y > max_y {
                self.scroll_offset.y = max_y;
            }

            // Horizontal: only when not wrapping
            if self.word_wrap {
                self.scroll_offset.x = px(0.);
            } else {
                let max_x = (self.last_max_line_width - bounds.size.width).max(px(0.));
                if self.scroll_offset.x > max_x {
                    self.scroll_offset.x = max_x;
                }
            }
        }
    }

    fn scroll_to_cursor(&mut self) {
        let bounds = match &self.last_bounds {
            Some(b) => *b,
            None => return,
        };
        let cursor_line = self.cursors[0].position.line;
        let cursor_col = self.cursors[0].position.col;

        if self.word_wrap {
            // Compute visual Y by summing visual line counts for lines before cursor,
            // then add the wrapped sub-line offset for the cursor's line
            let visual_y_lines: usize = self.last_visual_line_counts.iter().take(cursor_line).sum();
            // Find which visual sub-line within this wrapped line the cursor is on
            let sub_line = if let Some(wrapped) = self.last_wrapped_lines.get(cursor_line) {
                if let Some(pos) = wrapped.position_for_index(cursor_col, self.last_line_height) {
                    (pos.y / self.last_line_height) as usize
                } else {
                    0
                }
            } else {
                0
            };
            let cursor_y = self.last_line_height * (visual_y_lines + sub_line);
            let visible_top = self.scroll_offset.y;
            let visible_bottom = visible_top + bounds.size.height - self.last_line_height;
            if cursor_y < visible_top {
                self.scroll_offset.y = cursor_y;
            } else if cursor_y > visible_bottom {
                self.scroll_offset.y = cursor_y - bounds.size.height + self.last_line_height;
            }
        } else {
            // Non-wrapped: simple line-based Y
            let cursor_y = self.last_line_height * cursor_line;
            let visible_top = self.scroll_offset.y;
            let visible_bottom = visible_top + bounds.size.height - self.last_line_height;
            if cursor_y < visible_top {
                self.scroll_offset.y = cursor_y;
            } else if cursor_y > visible_bottom {
                self.scroll_offset.y = cursor_y - bounds.size.height + self.last_line_height;
            }

            // Horizontal scroll to cursor
            let cursor_x = self.last_shaped_lines
                .get(cursor_line)
                .map(|l| l.x_for_index(cursor_col))
                .unwrap_or(px(0.));
            let visible_left = self.scroll_offset.x;
            let visible_right = visible_left + bounds.size.width - px(16.); // padding
            if cursor_x < visible_left {
                self.scroll_offset.x = cursor_x;
            } else if cursor_x > visible_right {
                self.scroll_offset.x = cursor_x - bounds.size.width + px(16.);
            }
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
        self.needs_scroll_to_cursor = true;
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

        self.needs_scroll_to_cursor = true;
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
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
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
            .on_action(cx.listener(Self::toggle_word_wrap))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .bg(theme.surface0)
            .size_full()
            .overflow_hidden()
            .font_family("JetBrains Mono")
            .line_height(px(24.))
            .text_size(px(14.))
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
    wrapped_lines: Vec<WrappedLine>,
    word_wrap: bool,
    visual_line_counts: Vec<usize>,
    max_line_width: Pixels,
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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
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
        let word_wrap = input.word_wrap;

        let mut shaped_lines = Vec::new();
        let mut wrapped_lines = Vec::new();
        let mut visual_line_counts = Vec::with_capacity(input.lines.len());
        let mut max_line_width = px(0.);

        if word_wrap {
            // Shape with wrapping
            let wrap_width = bounds.size.width;
            for line_text in &input.lines {
                let display_text: SharedString = if line_text.is_empty() {
                    " ".into()
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
                let result = window
                    .text_system()
                    .shape_text(display_text, font_size, &[run], Some(wrap_width), None);
                if let Ok(mut lines) = result {
                    if let Some(wl) = lines.pop() {
                        let count = wl.wrap_boundaries.len() + 1;
                        visual_line_counts.push(count);
                        wrapped_lines.push(wl);
                    } else {
                        visual_line_counts.push(1);
                        wrapped_lines.push(WrappedLine::default());
                    }
                } else {
                    visual_line_counts.push(1);
                    wrapped_lines.push(WrappedLine::default());
                }
            }
        } else {
            // Shape without wrapping
            for line_text in &input.lines {
                let display_text: SharedString = if line_text.is_empty() {
                    " ".into()
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
                if shaped.width > max_line_width {
                    max_line_width = shaped.width;
                }
                shaped_lines.push(shaped);
                visual_line_counts.push(1);
            }
        }

        // Build cursor rects and selection rects
        let mut cursor_rects = Vec::new();
        let mut selections = Vec::new();
        let is_focused = input.focus_handle.is_focused(window);

        // Helper: compute the visual Y offset for a logical line
        let visual_y_for_line = |line: usize| -> Pixels {
            let visual_lines_before: usize = visual_line_counts.iter().take(line).sum();
            line_height * visual_lines_before
        };

        if word_wrap {
            // Wrapped mode: use WrappedLineLayout position_for_index
            for c in &input.cursors {
                let base_y = visual_y_for_line(c.position.line);
                let (cx_offset, cy_offset) = if let Some(wl) = wrapped_lines.get(c.position.line) {
                    if let Some(pos) = wl.position_for_index(c.position.col, line_height) {
                        (pos.x, pos.y)
                    } else {
                        (px(0.), px(0.))
                    }
                } else {
                    (px(0.), px(0.))
                };

                let cursor_screen = point(
                    bounds.left() + cx_offset,
                    bounds.top() + base_y + cy_offset - scroll_offset.y,
                );

                if !c.has_selection() && is_focused {
                    cursor_rects.push((
                        Bounds::new(cursor_screen, size(px(2.), line_height)),
                        theme.accent,
                    ));
                }

                if let Some((start, end)) = c.selection_range() {
                    // For wrapped selections, paint per-visual-line segments
                    for line_idx in start.line..=end.line {
                        let col_start = if line_idx == start.line { start.col } else { 0 };
                        let col_end = if line_idx == end.line { end.col } else { input.lines[line_idx].len() };
                        let base = visual_y_for_line(line_idx);

                        if let Some(wl) = wrapped_lines.get(line_idx) {
                            let start_pos = wl.position_for_index(col_start, line_height).unwrap_or(point(px(0.), px(0.)));
                            let end_pos = wl.position_for_index(col_end, line_height).unwrap_or(point(px(0.), px(0.)));

                            if start_pos.y == end_pos.y {
                                // Same visual line
                                selections.push(fill(
                                    Bounds::from_corners(
                                        point(bounds.left() + start_pos.x, bounds.top() + base + start_pos.y - scroll_offset.y),
                                        point(bounds.left() + end_pos.x, bounds.top() + base + end_pos.y + line_height - scroll_offset.y),
                                    ),
                                    rgba(0x3311ff30),
                                ));
                            } else {
                                // Spans multiple visual lines — paint start to end of first line,
                                // full middle lines, and start of last line to end
                                let wrap_width = bounds.size.width;
                                // First visual line
                                selections.push(fill(
                                    Bounds::from_corners(
                                        point(bounds.left() + start_pos.x, bounds.top() + base + start_pos.y - scroll_offset.y),
                                        point(bounds.left() + wrap_width, bounds.top() + base + start_pos.y + line_height - scroll_offset.y),
                                    ),
                                    rgba(0x3311ff30),
                                ));
                                // Middle visual lines
                                let start_vline = (start_pos.y / line_height) as usize;
                                let end_vline = (end_pos.y / line_height) as usize;
                                for vl in (start_vline + 1)..end_vline {
                                    let vy = line_height * vl;
                                    selections.push(fill(
                                        Bounds::from_corners(
                                            point(bounds.left(), bounds.top() + base + vy - scroll_offset.y),
                                            point(bounds.left() + wrap_width, bounds.top() + base + vy + line_height - scroll_offset.y),
                                        ),
                                        rgba(0x3311ff30),
                                    ));
                                }
                                // Last visual line
                                selections.push(fill(
                                    Bounds::from_corners(
                                        point(bounds.left(), bounds.top() + base + end_pos.y - scroll_offset.y),
                                        point(bounds.left() + end_pos.x, bounds.top() + base + end_pos.y + line_height - scroll_offset.y),
                                    ),
                                    rgba(0x3311ff30),
                                ));
                            }
                        }
                    }

                    // Cursor at selection edge
                    if is_focused {
                        cursor_rects.push((
                            Bounds::new(cursor_screen, size(px(2.), line_height)),
                            theme.accent,
                        ));
                    }
                }
            }
        } else {
            // Non-wrapped mode: use ShapedLine x_for_index
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

            for c in &input.cursors {
                if let Some((start, end)) = c.selection_range() {
                    for line_idx in start.line..=end.line {
                        let col_start = if line_idx == start.line { start.col } else { 0 };
                        let col_end = if line_idx == end.line { end.col } else { input.lines[line_idx].len() };

                        let x_start = shaped_lines.get(line_idx).map(|l| l.x_for_index(col_start)).unwrap_or(px(0.));
                        let x_end = shaped_lines.get(line_idx).map(|l| l.x_for_index(col_end)).unwrap_or(px(0.));
                        let y = line_height * line_idx;

                        selections.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x_start - scroll_offset.x, bounds.top() + y - scroll_offset.y),
                                point(bounds.left() + x_end - scroll_offset.x, bounds.top() + y + line_height - scroll_offset.y),
                            ),
                            rgba(0x3311ff30),
                        ));
                    }

                    if is_focused {
                        let x = shaped_lines.get(c.position.line).map(|l| l.x_for_index(c.position.col)).unwrap_or(px(0.));
                        let y = line_height * c.position.line;
                        cursor_rects.push((
                            Bounds::new(
                                point(bounds.left() + x - scroll_offset.x, bounds.top() + y - scroll_offset.y),
                                size(px(2.), line_height),
                            ),
                            theme.accent,
                        ));
                    }
                }
            }
        }

        MultiLinePrepaintState {
            shaped_lines,
            wrapped_lines,
            word_wrap,
            visual_line_counts,
            max_line_width,
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

        let line_height = prepaint.line_height;
        let scroll_offset = prepaint.scroll_offset;

        if prepaint.word_wrap {
            // Paint wrapped lines
            let mut visual_y = px(0.);
            for (i, wrapped) in prepaint.wrapped_lines.iter().enumerate() {
                let visual_height = line_height * prepaint.visual_line_counts[i];
                let y = bounds.top() + visual_y - scroll_offset.y;
                // Skip lines outside visible bounds
                if y + visual_height >= bounds.top() && y <= bounds.bottom() {
                    let origin = point(bounds.left(), y);
                    wrapped
                        .paint(origin, line_height, TextAlign::Left, None, window, cx)
                        .ok();
                }
                visual_y += visual_height;
            }
        } else {
            // Paint unwrapped lines
            for (i, shaped) in prepaint.shaped_lines.iter().enumerate() {
                let y = bounds.top() + line_height * i - scroll_offset.y;
                if y + line_height < bounds.top() || y > bounds.bottom() {
                    continue;
                }
                let origin = point(bounds.left() - scroll_offset.x, y);
                shaped
                    .paint(origin, line_height, TextAlign::Left, None, window, cx)
                    .ok();
            }
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
        let wrapped_lines: Vec<WrappedLine> = prepaint.wrapped_lines.drain(..).collect();
        let visual_line_counts = prepaint.visual_line_counts.clone();
        let max_line_width = prepaint.max_line_width;
        self.input.update(cx, |input, cx| {
            input.last_shaped_lines = shaped_lines;
            input.last_wrapped_lines = wrapped_lines;
            input.last_visual_line_counts = visual_line_counts;
            input.last_max_line_width = max_line_width;
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            // Apply scroll_to_cursor with fresh layout data when cursor moved
            if input.needs_scroll_to_cursor {
                input.needs_scroll_to_cursor = false;
                let old_scroll = input.scroll_offset;
                input.scroll_to_cursor();
                if input.scroll_offset != old_scroll {
                    cx.notify();
                }
            }
        });
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
}
