use std::{path::Path, process::Command};

use anyhow::{Context as _, Result, bail};
use gpui::*;
use serde::Deserialize;

use crate::{editor::MultiLineEditor, theme::Theme};

#[derive(Deserialize)]
pub struct HistoryItem {
    item: String,
    #[serde(default)]
    app: String,
    #[serde(rename = "dataType")]
    data_type: i32,
    #[serde(rename = "dataHash")]
    data_hash: Option<String>,
    #[serde(skip)]
    file_paths: Option<String>,
}

impl HistoryItem {
    fn editable(&self) -> bool {
        self.data_type == 0 || (self.data_type == 2 && self.file_paths.is_some())
    }

    fn preview(&self) -> String {
        if self.data_type == 1 {
            return "Image · cannot insert into a text editor".into();
        }
        if self.data_type == 2 && self.file_paths.is_none() {
            return format!("File list unavailable · {}", self.item);
        }
        self.item
            .chars()
            .take(180)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    }
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
        .arg("SELECT CAST(item AS TEXT) AS item, COALESCE(app, '') AS app, dataType, dataHash FROM clipboard WHERE item IS NOT NULL ORDER BY ts DESC, rowid DESC;")
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
        if item.data_type == 2
            && let Some(hash) = &item.data_hash
            && !hash.is_empty()
            && hash.chars().all(|c| c.is_ascii_alphanumeric())
            && let Ok(output) = Command::new("/usr/bin/plutil")
                .args(["-convert", "json", "-o", "-", "--"])
                .arg(data_path.join(hash))
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
    _search_subscription: Subscription,
}

impl EventEmitter<HistoryEvent> for ClipboardHistory {}

impl ClipboardHistory {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(MultiLineEditor::new);
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
                    Ok(items) => this.items = items,
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
        cx.notify();
    }
}

impl Render for ClipboardHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let text = theme.text;
        let muted = theme.subtext0;
        let selected_bg = theme.surface1;
        let base = theme.base;
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
        div()
            .key_context("ClipboardHistory")
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
            .bg(base)
            .child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(format!(
                        "Alfred clipboard history · {} items · type to search",
                        self.matches.len()
                    )),
            )
            .child(div().h(px(42.)).w_full().child(self.search.clone()))
            .child(if let Some(message) = message {
                div()
                    .flex_1()
                    .p(px(16.))
                    .text_size(px(13.))
                    .child(message)
                    .into_any_element()
            } else {
                uniform_list("alfred-history", self.matches.len(), move |range, _, cx| {
                    let this = entity.read(cx);
                    range
                        .map(|index| {
                            let item = &this.items[this.matches[index]];
                            div()
                                .id(index)
                                .h(px(52.))
                                .px(px(12.))
                                .py(px(6.))
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .bg(if this.selected == index {
                                    selected_bg
                                } else {
                                    base
                                })
                                .text_color(if item.editable() { text } else { muted })
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .overflow_hidden()
                                        .child(item.preview()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(muted)
                                        .child(item.app.clone()),
                                )
                                .on_mouse_down(MouseButton::Left, {
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.selected = index;
                                            this.accept(cx);
                                        })
                                    }
                                })
                        })
                        .collect()
                })
                .track_scroll(self.scroll.clone())
                .flex_1()
                .into_any_element()
            })
            .child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(11.))
                    .text_color(muted)
                    .child("↑ ↓ choose · Enter insert for editing · Escape cancel"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::read_history;
    use std::process::Command;

    #[test]
    fn reads_all_rows_in_recency_order_without_modifying_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard.alfdb");
        let status = Command::new("/usr/bin/sqlite3").arg(&path).arg(
            "CREATE TABLE clipboard(item, ts, app, dataType, dataHash); INSERT INTO clipboard VALUES ('old',1,'Test',0,NULL),('čćž\nsecond line',3,NULL,0,NULL),('image',2,'Test',1,NULL),('file',4,'Finder',2,'abc123');"
        ).status().unwrap();
        assert!(status.success());
        let data_path = dir.path().join("clipboard.alfdb.data");
        std::fs::create_dir(&data_path).unwrap();
        std::fs::write(data_path.join("abc123"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><array><string>/tmp/file</string><string>/tmp/second file</string></array></plist>").unwrap();
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
        assert!(!items[2].editable());
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
}
