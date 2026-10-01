use std::{
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use gpui::*;
use serde::Deserialize;

use crate::{editor::MultiLineEditor, theme::Theme};

const LIST_WIDTH_FRACTION: f32 = 0.54;
const ROW_HEIGHT: f32 = 36.;
const HISTORY_CHUNK_SIZE: usize = 200;

pub fn initial_history_count(window_height: f32) -> usize {
    // The popup header and search field occupy 74 logical pixels.
    ((window_height - 74.).max(ROW_HEIGHT) / ROW_HEIGHT).ceil() as usize
}

#[derive(Clone, Copy)]
struct HistoryCursor {
    timestamp: f64,
    row_id: i64,
}

#[derive(Deserialize)]
pub struct HistoryItem {
    item: String,
    row_id: i64,
    cursor_ts: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    apppath: String,
    #[serde(default)]
    ts: f64,
    #[serde(rename = "dataType")]
    data_type: i32,
    #[serde(rename = "dataHash")]
    data_hash: Option<String>,
    #[serde(skip)]
    file_paths: Option<String>,
    #[serde(skip)]
    image_path: Option<PathBuf>,
    #[serde(skip)]
    icon: Option<Arc<Image>>,
    #[serde(skip)]
    searchable: String,
}

impl HistoryItem {
    fn editable(&self) -> bool {
        self.data_type == 0 || (self.data_type == 2 && self.file_paths.is_some())
    }

    fn preview(&self) -> String {
        if self.data_type == 2 && self.file_paths.is_none() {
            return format!("File list unavailable · {}", self.item);
        }
        self.item
            .chars()
            .take(180)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    }

    fn full_text(&self) -> &str {
        self.file_paths.as_deref().unwrap_or(&self.item)
    }

    fn statistics(&self) -> String {
        if self.data_type == 1 {
            return "Image · cannot insert into a text editor".into();
        }
        format!(
            "{} {}; {} chars",
            self.full_text().split_whitespace().count(),
            if self.full_text().split_whitespace().count() == 1 {
                "word"
            } else {
                "words"
            },
            self.full_text().chars().count()
        )
    }
}

// Map lowercase search bytes back to original UTF-8 boundaries. Unicode lower-
// casing can expand a character, so offsets in the lowercase text are unsafe.
fn search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    let mut original_bytes = Vec::new();
    for (start, character) in text.char_indices() {
        let lowercase = character.to_lowercase().to_string();
        original_bytes.extend(std::iter::repeat_n(
            start..start + character.len_utf8(),
            lowercase.len(),
        ));
        folded.push_str(&lowercase);
    }
    let mut ranges = Vec::new();
    let terms = if folded.contains(&query) {
        vec![query.as_str()]
    } else {
        query.split_whitespace().collect()
    };
    for term in terms {
        for (start, _) in folded.match_indices(term) {
            ranges.push(original_bytes[start].start..original_bytes[start + term.len() - 1].end);
        }
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn highlighted(text: String, query: &str, highlight: HighlightStyle) -> StyledText {
    let ranges = search_ranges(&text, query);
    StyledText::new(text).with_highlights(ranges.into_iter().map(|range| (range, highlight)))
}

fn shortcut_index(digit: usize, count: usize) -> Option<usize> {
    if !(1..=9).contains(&digit) {
        return None;
    }
    let index = digit - 1;
    (index < count).then_some(index)
}

fn sufficiently_visible(index: usize, offset: f32, viewport_height: f32) -> bool {
    let top = index as f32 * ROW_HEIGHT + offset;
    let visible = (top + ROW_HEIGHT).min(viewport_height) - top.max(0.);
    // Allow for subpixel rounding at the exact 85% boundary.
    visible - ROW_HEIGHT * 0.85 > 0.001
}

fn data_file_path(directory: &Path, name: &str) -> Option<PathBuf> {
    // Alfred uses hashes with extensions. Accept one filename, never a path.
    (!name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    .then(|| directory.join(name))
}

fn read_history_chunk(
    path: &Path,
    cursor: Option<HistoryCursor>,
    count: usize,
) -> Result<Vec<HistoryItem>> {
    if !path.is_file() {
        bail!("Alfred clipboard history was not found. Enable history in Alfred first.");
    }
    // Keyset pagination uses Alfred's timestamp index, including rowid for ties.
    // New clips arriving above the cursor cannot shift or duplicate later pages.
    let after = cursor.map_or_else(String::new, |cursor| {
        format!(
            "AND (ts, rowid) < ({}, {})",
            cursor.timestamp, cursor.row_id
        )
    });
    let query = format!(
        "SELECT rowid AS row_id, printf('%!.17g', ts) AS cursor_ts, CAST(item AS TEXT) AS item, COALESCE(app, '') AS app, COALESCE(apppath, '') AS apppath, CAST(ts AS REAL) AS ts, dataType, dataHash FROM clipboard WHERE item IS NOT NULL {after} ORDER BY ts DESC, rowid DESC LIMIT {count};"
    );
    let output = Command::new("/usr/bin/sqlite3")
        .args(["-readonly", "-json", "-cmd", ".timeout 2000"])
        .arg(path)
        .arg(query)
        .output()
        .context("Could not read Alfred clipboard history")?;
    if !output.status.success() {
        bail!("Alfred clipboard history could not be read. Try opening the picker again.");
    }
    if output.stdout.is_empty() {
        return Ok(Vec::new());
    }
    let mut items: Vec<HistoryItem> = serde_json::from_slice(&output.stdout)
        .context("Alfred clipboard history has an unsupported format")?;
    let mut data_path = path.as_os_str().to_os_string();
    data_path.push(".data");
    let data_path = std::path::PathBuf::from(data_path);
    for item in &mut items {
        let data_file = item
            .data_hash
            .as_deref()
            .and_then(|hash| data_file_path(&data_path, hash));
        if item.data_type == 1
            && let Some(path) = &data_file
            && path.is_file()
        {
            item.image_path = Some(path.clone());
        }
        if item.data_type == 2
            && let Some(path) = &data_file
            && let Ok(output) = Command::new("/usr/bin/plutil")
                .args(["-convert", "json", "-o", "-", "--"])
                .arg(path)
                .output()
            && output.status.success()
            && let Ok(paths) = serde_json::from_slice::<Vec<String>>(&output.stdout)
        {
            item.file_paths = Some(paths.join("\n"));
        }
        item.searchable = format!(
            "{} {} {}",
            item.item,
            item.app,
            item.file_paths.as_deref().unwrap_or_default()
        )
        .to_lowercase();
    }
    Ok(items)
}

pub enum HistoryEvent {
    Selected(String),
    Cancelled,
}

pub struct ClipboardHistory {
    pub search: Entity<MultiLineEditor>,
    items: Vec<HistoryItem>,
    matches: Vec<usize>,
    query: String,
    selected: usize,
    loading: bool,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    preview_scroll: ScrollHandle,
    icons: HashMap<String, Option<Arc<Image>>>,
    spinner_frame: usize,
    _search_subscription: Subscription,
}

impl EventEmitter<HistoryEvent> for ClipboardHistory {}

impl ClipboardHistory {
    pub fn new(initial_count: usize, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            let mut input = MultiLineEditor::new(cx);
            input.compact_input = true;
            input
        });
        let subscription = cx.observe(&search, |this, search, cx| {
            let query = search.read(cx).lines.join(" ").to_lowercase();
            if query != this.query {
                this.query = query;
                this.filter(true);
                cx.notify();
            }
        });
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let mut cursor = None;
            let mut count = initial_count.max(1);
            loop {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let home = dirs::home_dir().context("Could not locate your home folder")?;
                        read_history_chunk(
                            &home.join(
                                "Library/Application Support/Alfred/Databases/clipboard.alfdb",
                            ),
                            cursor,
                            count,
                        )
                    })
                    .await;
                let (next_cursor, finished) = match &result {
                    Ok(items) => (
                        items.last().map(|item| HistoryCursor {
                            timestamp: item.cursor_ts.parse().expect("SQLite numeric timestamp"),
                            row_id: item.row_id,
                        }),
                        items.len() < count,
                    ),
                    Err(_) => (None, true),
                };
                if this
                    .update(cx, |this, cx| {
                        this.loading = !finished;
                        match result {
                            Ok(items) => this.items.extend(items),
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        this.filter(false);
                        cx.notify();
                        if cursor.is_none() {
                            crate::logging::event(
                                "history.first_chunk",
                                format!(
                                    "items={} elapsed_ms={}",
                                    this.items.len(),
                                    started.elapsed().as_millis()
                                ),
                            );
                        }
                        if finished {
                            crate::logging::event(
                                "history.complete",
                                format!(
                                    "items={} elapsed_ms={}",
                                    this.items.len(),
                                    started.elapsed().as_millis()
                                ),
                            );
                        }
                    })
                    .is_err()
                    || finished
                {
                    break;
                }
                cursor = next_cursor;
                count = HISTORY_CHUNK_SIZE;
                // Let the first screen paint before requesting the next batch.
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let keep_animating = this
                    .update(cx, |this, cx| {
                        if this.loading {
                            this.spinner_frame = (this.spinner_frame + 1) % 10;
                            cx.notify();
                        }
                        this.loading
                    })
                    .unwrap_or(false);
                if !keep_animating {
                    break;
                }
            }
        })
        .detach();
        Self {
            search,
            items: Vec::new(),
            matches: Vec::new(),
            query: String::new(),
            selected: 0,
            loading: true,
            error: None,
            scroll: UniformListScrollHandle::new(),
            preview_scroll: ScrollHandle::new(),
            icons: HashMap::new(),
            spinner_frame: 0,
            _search_subscription: subscription,
        }
    }

    fn filter(&mut self, reset: bool) {
        let selected_item = self.matches.get(self.selected).copied();
        let words: Vec<_> = self.query.split_whitespace().collect();
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                words
                    .iter()
                    .all(|word| item.searchable.contains(word))
                    .then_some(index)
            })
            .collect();
        self.selected = selected_item
            .and_then(|item| self.matches.iter().position(|index| *index == item))
            .unwrap_or(0);
        if reset {
            self.selected = 0;
            self.preview_scroll.set_offset(point(px(0.), px(0.)));
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = self
            .matches
            .get(self.selected)
            .map(|index| &self.items[*index])
            && item.editable()
        {
            cx.emit(HistoryEvent::Selected(
                item.file_paths.as_ref().unwrap_or(&item.item).clone(),
            ));
        }
    }

    fn move_selection(&mut self, down: bool, cx: &mut Context<Self>) {
        self.selected = if down {
            (self.selected + 1).min(self.matches.len().saturating_sub(1))
        } else {
            self.selected.saturating_sub(1)
        };
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        self.preview_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn number_shortcut(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mods = &event.keystroke.modifiers;
        if !mods.platform || mods.shift || mods.alt || mods.control {
            return;
        }
        if let Ok(digit) = event.keystroke.key.parse::<usize>()
            && let Some(index) = shortcut_index(digit, self.matches.len())
        {
            cx.stop_propagation();
            if event.is_held {
                return;
            }
            self.selected = index;
            self.preview_scroll.set_offset(point(px(0.), px(0.)));
            let visible = {
                let scroll = self.scroll.0.borrow();
                sufficiently_visible(
                    index,
                    scroll.base_handle.offset().y.into(),
                    scroll.base_handle.bounds().size.height.into(),
                )
            };
            if visible {
                self.accept(cx);
            } else {
                self.scroll
                    .scroll_to_item_strict(index, ScrollStrategy::Center);
            }
            cx.stop_propagation();
            cx.notify();
        }
    }
}

impl Render for ClipboardHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let text = theme.text;
        let muted = theme.subtext0;
        let selected_bg = theme.accent;
        let base = theme.base;
        let border = theme.surface0;
        let highlight = HighlightStyle {
            background_color: Some(rgba(0xe5c558bb).into()),
            color: Some(rgb(0x292516).into()),
            ..Default::default()
        };
        let entity = cx.entity();
        // Icons are fetched only for rows around the actual viewport, not all history.
        let first = (-f32::from(self.scroll.0.borrow().base_handle.offset().y) / ROW_HEIGHT).max(0.)
            as usize;
        let visible_count = initial_history_count(
            f32::from(self.scroll.0.borrow().base_handle.bounds().size.height) + 74.,
        ) + 2;
        for index in self.matches.iter().skip(first).take(visible_count) {
            let item = &mut self.items[*index];
            item.icon = self
                .icons
                .entry(item.apppath.clone())
                .or_insert_with(|| crate::history_platform::app_icon(&item.apppath))
                .clone();
        }
        let message = if self.matches.is_empty() && !self.loading && self.error.is_none() {
            Some("No clipboard items found".to_string())
        } else {
            None
        };
        let selection = self
            .matches
            .get(self.selected)
            .map(|index| &self.items[*index]);
        let preview = if let Some(item) = selection {
            let content = if let Some(path) = &item.image_path {
                div()
                    .w_full()
                    .child(img(path.clone()).w_full())
                    .into_any_element()
            } else {
                div()
                    .w_full()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .child(highlighted(
                        item.full_text().to_string(),
                        &self.query,
                        highlight,
                    ))
                    .into_any_element()
            };
            div()
                .flex()
                .flex_col()
                .size_full()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(
                    div()
                        .id("clipboard-preview")
                        .flex_1()
                        .min_h(px(0.))
                        .w_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.preview_scroll)
                        .p(px(12.))
                        .child(content),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w_full()
                        .px(px(8.))
                        .py(px(9.))
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .text_color(muted)
                        .text_align(TextAlign::Center)
                        .child(div().w_full().child(item.statistics()))
                        .child(
                            div()
                                .w_full()
                                .child(crate::history_platform::copied_at(item.ts)),
                        ),
                )
                .into_any_element()
        } else {
            div().size_full().into_any_element()
        };
        div()
            .key_context("ClipboardHistory")
            .capture_key_down(cx.listener(Self::number_shortcut))
            .capture_action(cx.listener(|this, _: &crate::editor::Up, _, cx| {
                this.move_selection(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &crate::editor::Down, _, cx| {
                this.move_selection(true, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &crate::editor::Enter, _, cx| {
                this.accept(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &crate::SubmitAndPaste, _, cx| {
                this.accept(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|_, _: &crate::Escape, _, cx| {
                cx.emit(HistoryEvent::Cancelled);
                cx.stop_propagation();
            }))
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .bg(base)
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .h(px(42.))
                    .w_full()
                    .border_b_1()
                    .border_color(border)
                    .child(self.search.clone()),
            )
            .child(if let Some(message) = message {
                div()
                    .flex_1()
                    .p(px(16.))
                    .text_size(px(13.))
                    .child(message)
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .w(relative(LIST_WIDTH_FRACTION))
                            .h_full()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .border_r_1()
                            .border_color(border)
                            .child(
                                uniform_list(
                                    "alfred-history",
                                    self.matches.len()
                                        + usize::from(self.loading || self.error.is_some()),
                                    move |range, window, cx| {
                                        let this = entity.read(cx);
                                        // Virtualized rows are laid out as roots. Their percentage
                                        // widths otherwise resolve against the intrinsic text width.
                                        let row_width = window.viewport_size().width
                                            * LIST_WIDTH_FRACTION
                                            - px(1.);
                                        range
                                            .map(|index| {
                                                if index == this.matches.len() {
                                                    let spinner = [
                                                        "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧",
                                                        "⠇", "⠏",
                                                    ][this.spinner_frame];
                                                    return div()
                                                        .w(row_width)
                                                        .h(px(ROW_HEIGHT))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .text_color(muted)
                                                        .text_size(px(14.))
                                                        .child(
                                                            this.error
                                                                .clone()
                                                                .unwrap_or_else(|| spinner.into()),
                                                        )
                                                        .into_any_element();
                                                }
                                                let item = &this.items[this.matches[index]];
                                                let selected = this.selected == index;
                                                let foreground = if !item.editable() {
                                                    if selected {
                                                        rgba(0xffffff99)
                                                    } else {
                                                        Rgba {
                                                            a: muted.a * 0.65,
                                                            ..muted
                                                        }
                                                    }
                                                } else if selected {
                                                    rgb(0xffffff)
                                                } else {
                                                    text
                                                };
                                                let shortcut = if selected && item.editable() {
                                                    "↩".to_string()
                                                } else if index < 9 {
                                                    format!("⌘{}", index + 1)
                                                } else {
                                                    String::new()
                                                };
                                                let icon = if let Some(icon) = &item.icon {
                                                    img(icon.clone())
                                                        .size(px(24.))
                                                        .into_any_element()
                                                } else {
                                                    div()
                                                        .size(px(24.))
                                                        .rounded(px(5.))
                                                        .bg(border)
                                                        .text_size(px(12.))
                                                        .child("▧")
                                                        .into_any_element()
                                                };
                                                div()
                                                    .id(index)
                                                    .w(row_width)
                                                    .h(px(ROW_HEIGHT))
                                                    .flex_shrink_0()
                                                    .px(px(8.))
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap(px(8.))
                                                    .overflow_hidden()
                                                    .cursor_pointer()
                                                    .bg(if selected { selected_bg } else { base })
                                                    .text_color(foreground)
                                                    .child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .size(px(24.))
                                                            .child(icon),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w(px(0.))
                                                            .text_size(px(16.))
                                                            .line_height(px(22.))
                                                            .line_clamp(1)
                                                            .text_ellipsis()
                                                            .child(highlighted(
                                                                item.preview(),
                                                                &this.query,
                                                                highlight,
                                                            )),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .w(px(30.))
                                                            .text_size(px(14.))
                                                            .text_align(TextAlign::Right)
                                                            .child(shortcut),
                                                    )
                                                    .on_mouse_down(MouseButton::Left, {
                                                        let entity = entity.clone();
                                                        move |event, _, cx| {
                                                            entity.update(cx, |this, cx| {
                                                                this.selected = index;
                                                                this.preview_scroll.set_offset(
                                                                    point(px(0.), px(0.)),
                                                                );
                                                                if event.click_count > 1 {
                                                                    this.accept(cx);
                                                                }
                                                                cx.notify();
                                                            });
                                                            cx.stop_propagation();
                                                        }
                                                    })
                                                    .into_any_element()
                                            })
                                            .collect()
                                    },
                                )
                                .track_scroll(self.scroll.clone())
                                .w_full()
                                .h_full(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .overflow_hidden()
                            .child(preview),
                    )
                    .into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HistoryCursor, data_file_path, initial_history_count, read_history_chunk, search_ranges,
        shortcut_index, sufficiently_visible,
    };
    use std::process::Command;

    #[test]
    fn reads_all_rows_in_recency_order_without_modifying_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard.alfdb");
        let status = Command::new("/usr/bin/sqlite3").arg(&path).arg(
            "CREATE TABLE clipboard(item, ts, app, dataType, dataHash, apppath); INSERT INTO clipboard VALUES ('old',1,'Test',0,NULL,'/Applications/Test.app'),('čćž\nsecond line',3,NULL,0,NULL,''),('image',2,'Test',1,'def456.tiff',''),('file',4,'Finder',2,'abc123.plist','');"
        ).status().unwrap();
        assert!(status.success());
        let data_path = dir.path().join("clipboard.alfdb.data");
        std::fs::create_dir(&data_path).unwrap();
        std::fs::write(data_path.join("abc123.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><array><string>/tmp/file</string><string>/tmp/second file</string></array></plist>").unwrap();
        std::fs::write(data_path.join("def456.tiff"), b"fixture").unwrap();
        let before = std::fs::read(&path).unwrap();
        let items = read_history_chunk(&path, None, 10).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[0].file_paths.as_deref(),
            Some("/tmp/file\n/tmp/second file")
        );
        assert!(items[0].editable());
        assert_eq!(items[1].item, "čćž\nsecond line");
        assert!(items[1].editable());
        assert_eq!(items[1].statistics(), "3 words; 15 chars");
        assert!(crate::history_platform::copied_at(items[1].ts).contains("2001"));
        assert!(!items[2].editable());
        assert_eq!(
            items[2].image_path.as_ref(),
            Some(&data_path.join("def456.tiff"))
        );
        assert_eq!(items[3].item, "old");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn chunks_preserve_recency_ties_and_precision_when_new_clips_arrive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard.alfdb");
        let run = |sql: &str| {
            assert!(
                Command::new("/usr/bin/sqlite3")
                    .arg(&path)
                    .arg(sql)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        run(
            "CREATE TABLE clipboard(item, ts, app, dataType, dataHash, apppath); CREATE INDEX clipboard_ts ON clipboard(ts); INSERT INTO clipboard VALUES ('old',1,'Test',0,NULL,''),('tie first',812345678.1234567,'Test',0,NULL,''),('tie second',812345678.1234567,'Test',0,NULL,''),('recent',900000000,'Test',0,NULL,'');",
        );
        let first = read_history_chunk(&path, None, 2).unwrap();
        assert_eq!(
            first
                .iter()
                .map(|item| item.item.as_str())
                .collect::<Vec<_>>(),
            ["recent", "tie second"]
        );
        let last = first.last().unwrap();
        let cursor = HistoryCursor {
            timestamp: last.cursor_ts.parse().unwrap(),
            row_id: last.row_id,
        };
        run("INSERT INTO clipboard VALUES ('new clip',1000000000,'Test',0,NULL,'');");
        let second = read_history_chunk(&path, Some(cursor), 2).unwrap();
        assert_eq!(
            second
                .iter()
                .map(|item| item.item.as_str())
                .collect::<Vec<_>>(),
            ["tie first", "old"]
        );
        assert!(second[0].searchable.contains("tie first test"));
        let last = second.last().unwrap();
        let cursor = HistoryCursor {
            timestamp: last.cursor_ts.parse().unwrap(),
            row_id: last.row_id,
        };
        assert!(
            read_history_chunk(&path, Some(cursor), 2)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn missing_database_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.alfdb");
        assert!(read_history_chunk(&path, None, 10).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn highlights_complete_phrases_and_repeated_matches() {
        let text = "I also want this. I ALSO want that.";
        let ranges = search_ranges(text, "i also");
        let matches: Vec<_> = ranges.iter().map(|range| &text[range.clone()]).collect();
        assert_eq!(matches, ["I also", "I ALSO"]);
        assert!(search_ranges(text, " ").is_empty());
    }

    #[test]
    fn highlights_unicode_without_splitting_utf8_characters() {
        let text = "Čćž İstanbul 🦀";
        let ranges = search_ranges(text, "i čćž");
        let matches: Vec<_> = ranges.iter().map(|range| &text[range.clone()]).collect();
        assert_eq!(matches, ["Čćž", "İ"]);
        let ranges = search_ranges("banana", "ban nana");
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 0..6);
    }

    #[test]
    fn number_shortcuts_stay_on_first_nine_results() {
        assert_eq!(shortcut_index(1, 20), Some(0));
        assert_eq!(shortcut_index(9, 20), Some(8));
        assert_eq!(shortcut_index(9, 8), None);
        assert_eq!(shortcut_index(0, 20), None);
        assert_eq!(shortcut_index(10, 20), None);
    }

    #[test]
    fn shortcut_requires_more_than_85_percent_visible() {
        assert!(sufficiently_visible(0, 0., 320.));
        assert!(!sufficiently_visible(0, -36., 320.));
        assert!(!sufficiently_visible(0, -5.4, 320.));
        assert!(sufficiently_visible(0, -5.3, 320.));
        assert!(!sufficiently_visible(8, 0., 318.6));
        assert!(sufficiently_visible(8, 0., 318.7));
        assert!(!sufficiently_visible(8, 0., 0.));
        assert_eq!(initial_history_count(400.), 10);
        assert_eq!(initial_history_count(450.), 11);
    }

    #[test]
    fn data_filenames_allow_extensions_but_cannot_escape_directory() {
        let root = std::path::Path::new("/tmp/history");
        assert_eq!(
            data_file_path(root, "hash.clip.tiff"),
            Some(root.join("hash.clip.tiff"))
        );
        assert!(data_file_path(root, "../private").is_none());
        assert!(data_file_path(root, "/private").is_none());
        assert!(data_file_path(root, "..").is_none());
    }
}
