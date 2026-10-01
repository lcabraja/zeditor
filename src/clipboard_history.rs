use std::{
    cell::Cell,
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use gpui::*;
use serde::Deserialize;

use crate::{editor::MultiLineEditor, theme::Theme};

const LIST_WIDTH_FRACTION: f32 = 0.54;

#[derive(Deserialize)]
pub struct HistoryItem {
    item: String,
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
    copied_at: String,
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

fn shortcut_index(first: usize, digit: usize, count: usize) -> Option<usize> {
    if !(1..=9).contains(&digit) {
        return None;
    }
    let index = first.checked_add(digit - 1)?;
    (index < count).then_some(index)
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

fn read_history(path: &Path) -> Result<Vec<HistoryItem>> {
    if !path.is_file() {
        bail!("Alfred clipboard history was not found. Enable history in Alfred first.");
    }
    // Query the live database read-only, including every retained row. SQLite
    // handles Alfred's locks; do not copy its database or print private clips.
    let output = Command::new("/usr/bin/sqlite3")
        .args(["-readonly", "-json", "-cmd", ".timeout 2000"])
        .arg(path)
        .arg("SELECT CAST(item AS TEXT) AS item, COALESCE(app, '') AS app, COALESCE(apppath, '') AS apppath, CAST(ts AS REAL) AS ts, dataType, dataHash FROM clipboard WHERE item IS NOT NULL ORDER BY ts DESC, rowid DESC;")
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
        item.copied_at = crate::history_platform::copied_at(item.ts);
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
    visible_start: Cell<usize>,
    _search_subscription: Subscription,
}

impl EventEmitter<HistoryEvent> for ClipboardHistory {}

impl ClipboardHistory {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            let mut input = MultiLineEditor::new(cx);
            input.compact_input = true;
            input
        });
        let subscription = cx.observe(&search, |this, search, cx| {
            let query = search.read(cx).lines.join(" ").to_lowercase();
            if query != this.query {
                this.query = query;
                this.filter();
                cx.notify();
            }
        });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let home = dirs::home_dir().context("Could not locate your home folder")?;
                    read_history(
                        &home.join("Library/Application Support/Alfred/Databases/clipboard.alfdb"),
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(mut items) => {
                        let mut icons = HashMap::new();
                        for item in &mut items {
                            item.icon = icons
                                .entry(item.apppath.clone())
                                .or_insert_with(|| crate::history_platform::app_icon(&item.apppath))
                                .clone();
                        }
                        this.items = items;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                this.filter();
                cx.notify();
            });
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
            visible_start: Cell::new(0),
            _search_subscription: subscription,
        }
    }

    fn filter(&mut self) {
        let words: Vec<_> = self.query.split_whitespace().collect();
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let searchable = format!(
                    "{} {} {}",
                    item.item,
                    item.app,
                    item.file_paths.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                words
                    .iter()
                    .all(|word| searchable.contains(word))
                    .then_some(index)
            })
            .collect();
        self.selected = 0;
        self.visible_start.set(0);
        self.preview_scroll.set_offset(point(px(0.), px(0.)));
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
            && let Some(index) = shortcut_index(self.visible_start.get(), digit, self.matches.len())
        {
            self.selected = index;
            self.preview_scroll.set_offset(point(px(0.), px(0.)));
            self.accept(cx);
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
        let message = if self.loading {
            Some("Loading Alfred history…".to_string())
        } else if let Some(error) = &self.error {
            Some(error.clone())
        } else if self.matches.is_empty() {
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
                        .child(div().w_full().child(item.copied_at.clone())),
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
                                    self.matches.len(),
                                    move |range, window, cx| {
                                        let this = entity.read(cx);
                                        let first = range.start;
                                        this.visible_start.set(first);
                                        // Virtualized rows are laid out as roots. Their percentage
                                        // widths otherwise resolve against the intrinsic text width.
                                        let row_width = window.viewport_size().width
                                            * LIST_WIDTH_FRACTION
                                            - px(1.);
                                        range
                                            .map(|index| {
                                                let item = &this.items[this.matches[index]];
                                                let selected = this.selected == index;
                                                let foreground =
                                                    if selected { rgb(0xffffff) } else { text };
                                                let shortcut = if selected && item.editable() {
                                                    "↩".to_string()
                                                } else if index >= first && index - first < 9 {
                                                    format!("⌘{}", index - first + 1)
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
                                                    .h(px(36.))
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
    use super::{data_file_path, read_history, search_ranges, shortcut_index};
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
        let items = read_history(&path).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[0].file_paths.as_deref(),
            Some("/tmp/file\n/tmp/second file")
        );
        assert!(items[0].editable());
        assert_eq!(items[1].item, "čćž\nsecond line");
        assert!(items[1].editable());
        assert_eq!(items[1].statistics(), "3 words; 15 chars");
        assert!(items[1].copied_at.contains("2001"));
        assert!(!items[2].editable());
        assert_eq!(
            items[2].image_path.as_ref(),
            Some(&data_path.join("def456.tiff"))
        );
        assert_eq!(items[3].item, "old");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn missing_database_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.alfdb");
        assert!(read_history(&path).is_err());
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
    fn number_shortcuts_follow_visible_rows_and_stop_at_end() {
        assert_eq!(shortcut_index(14, 1, 20), Some(14));
        assert_eq!(shortcut_index(14, 6, 20), Some(19));
        assert_eq!(shortcut_index(14, 7, 20), None);
        assert_eq!(shortcut_index(0, 0, 20), None);
        assert_eq!(shortcut_index(0, 10, 20), None);
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
