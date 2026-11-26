//! Owner: the Block page (`pages/block`). The node's JSON for the current object, collapsed
//! by default (`JSONSection` in `Components.swift`). Expanded, it is a tree with links,
//! names and dates, a cursor, copy and save (`JSONTreeView` in `JSONTree.swift`). Used by
//! the Block, Transaction, Identity, Currency and Address pages.
//!
//! The tree keeps the members in document order and the numbers as the node wrote them, so
//! a copied value is exactly what the node sent. Each line is a stop of the page's cursor:
//! Return follows an ID or opens and closes a node, y and Ctrl-C copy the value or the
//! subtree, s saves the document.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route};
use crate::display;
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, Page};
use crate::theme::{self, font, tone};
use crate::widgets::icons::{self, Icon};
use crate::widgets::lamp::Disclosure;
use crate::widgets::{link, loader};

// MARK: - The document

/// A JSON value with its members in document order and numbers kept as written.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonNode {
    Object(Vec<(String, JsonNode)>),
    Array(Vec<JsonNode>),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

impl JsonNode {
    pub fn is_container(&self) -> bool {
        matches!(self, JsonNode::Object(_) | JsonNode::Array(_))
    }

    /// The members of an object, or the items of an array with their index as the key.
    pub fn children(&self) -> Vec<(String, &JsonNode)> {
        match self {
            JsonNode::Object(members) => members.iter().map(|(k, v)| (k.clone(), v)).collect(),
            JsonNode::Array(items) => items.iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
            _ => Vec::new(),
        }
    }

    /// Compact JSON for a subtree, or the raw scalar: a string without quotes, as the
    /// terminal's `y` copies it.
    pub fn copy_text(&self) -> String {
        match self {
            JsonNode::String(text) | JsonNode::Number(text) => text.clone(),
            JsonNode::Bool(flag) => if *flag { "true" } else { "false" }.into(),
            JsonNode::Null => "null".into(),
            _ => self.compact(),
        }
    }

    pub fn compact(&self) -> String {
        match self {
            JsonNode::Object(members) => format!(
                "{{{}}}",
                members.iter().map(|(k, v)| format!("{}:{}", quote(k), v.compact())).collect::<Vec<_>>().join(",")
            ),
            JsonNode::Array(items) => format!("[{}]", items.iter().map(JsonNode::compact).collect::<Vec<_>>().join(",")),
            JsonNode::String(text) => quote(text),
            JsonNode::Number(text) => text.clone(),
            JsonNode::Bool(flag) => if *flag { "true" } else { "false" }.into(),
            JsonNode::Null => "null".into(),
        }
    }

    /// Indented JSON with the numbers as written.
    pub fn pretty(&self) -> String {
        self.pretty_at("")
    }

    fn pretty_at(&self, indent: &str) -> String {
        let inner = format!("{indent}  ");
        match self {
            JsonNode::Object(members) => {
                if members.is_empty() {
                    return "{}".into();
                }
                let body: Vec<String> =
                    members.iter().map(|(k, v)| format!("{inner}{}: {}", quote(k), v.pretty_at(&inner))).collect();
                format!("{{\n{}\n{indent}}}", body.join(",\n"))
            }
            JsonNode::Array(items) => {
                if items.is_empty() {
                    return "[]".into();
                }
                let body: Vec<String> = items.iter().map(|v| format!("{inner}{}", v.pretty_at(&inner))).collect();
                format!("[\n{}\n{indent}]", body.join(",\n"))
            }
            _ => self.compact(),
        }
    }
}

/// A string as JSON, with the escapes of the Swift app.
pub fn quote(text: &str) -> String {
    let mut result = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if (c as u32) < 0x20 => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

/// A small JSON reader that keeps member order and number text (`JSONReader`).
pub fn parse(text: &str) -> Option<JsonNode> {
    let mut reader = Reader { bytes: text.as_bytes(), index: 0 };
    reader.skip_space();
    let value = reader.value()?;
    reader.skip_space();
    (reader.index == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }

    fn value(&mut self) -> Option<JsonNode> {
        match self.peek()? {
            b'{' => {
                self.index += 1;
                let mut members = Vec::new();
                self.skip_space();
                if self.peek() == Some(b'}') {
                    self.index += 1;
                    return Some(JsonNode::Object(members));
                }
                loop {
                    self.skip_space();
                    let key = self.string()?;
                    self.skip_space();
                    self.expect(b':')?;
                    self.skip_space();
                    members.push((key, self.value()?));
                    self.skip_space();
                    if self.separator(b'}')? {
                        return Some(JsonNode::Object(members));
                    }
                }
            }
            b'[' => {
                self.index += 1;
                let mut items = Vec::new();
                self.skip_space();
                if self.peek() == Some(b']') {
                    self.index += 1;
                    return Some(JsonNode::Array(items));
                }
                loop {
                    self.skip_space();
                    items.push(self.value()?);
                    self.skip_space();
                    if self.separator(b']')? {
                        return Some(JsonNode::Array(items));
                    }
                }
            }
            b'"' => Some(JsonNode::String(self.string()?)),
            b't' => self.literal(b"true").map(|_| JsonNode::Bool(true)),
            b'f' => self.literal(b"false").map(|_| JsonNode::Bool(false)),
            b'n' => self.literal(b"null").map(|_| JsonNode::Null),
            _ => {
                let start = self.index;
                while matches!(self.peek(), Some(b'+' | b'-' | b'0'..=b'9' | b'.' | b'e' | b'E')) {
                    self.index += 1;
                }
                (self.index > start)
                    .then(|| JsonNode::Number(String::from_utf8_lossy(&self.bytes[start..self.index]).into_owned()))
            }
        }
    }

    /// True at the closing bracket, false after a comma.
    fn separator(&mut self, close: u8) -> Option<bool> {
        match self.peek()? {
            b',' => {
                self.index += 1;
                Some(false)
            }
            c if c == close => {
                self.index += 1;
                Some(true)
            }
            _ => None,
        }
    }

    fn expect(&mut self, ch: u8) -> Option<()> {
        (self.peek()? == ch).then(|| self.index += 1)
    }

    fn literal(&mut self, word: &[u8]) -> Option<()> {
        (self.bytes.get(self.index..self.index + word.len())? == word).then(|| self.index += word.len())
    }

    fn string(&mut self) -> Option<String> {
        self.expect(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let byte = self.peek()?;
            self.index += 1;
            match byte {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let escape = self.peek()?;
                    self.index += 1;
                    let ch = match escape {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{08}',
                        b'f' => '\u{0C}',
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..=0xDBFF).contains(&code)
                                && self.bytes.get(self.index) == Some(&b'\\')
                                && self.bytes.get(self.index + 1) == Some(&b'u')
                            {
                                self.index += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + low.wrapping_sub(0xDC00);
                            }
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        other => other as char,
                    };
                    let mut buffer = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
                }
                other => out.push(other),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = std::str::from_utf8(self.bytes.get(self.index..self.index + 4)?).ok()?;
        let code = u32::from_str_radix(digits, 16).ok()?;
        self.index += 4;
        Some(code)
    }
}

// MARK: - Lines of the tree

/// The view shows at most this many lines.
pub const ROW_LIMIT: usize = 5000;
/// A longer string shows its start and a key to show all of it.
pub const PREVIEW_LENGTH: usize = 120;

/// One visible line of the tree.
#[derive(Debug)]
pub struct JsonRow<'a> {
    /// A JSON pointer: `""` for the root, `/vout/0/value` below it.
    pub pointer: String,
    pub depth: usize,
    /// The member name or the index, empty for the root.
    pub label: String,
    pub node: &'a JsonNode,
}

/// A key in a JSON pointer, with `~` and `/` escaped.
pub fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// The lines that show: each node, and the children of the open ones. The walk stops
/// adding children once the list is past `limit`, as the Swift view does.
pub fn visible_rows<'a>(root: &'a JsonNode, expanded: &HashSet<String>, limit: usize) -> Vec<JsonRow<'a>> {
    fn walk<'a>(
        rows: &mut Vec<JsonRow<'a>>,
        node: &'a JsonNode,
        pointer: String,
        label: String,
        depth: usize,
        expanded: &HashSet<String>,
        limit: usize,
    ) {
        let open = node.is_container() && expanded.contains(&pointer);
        rows.push(JsonRow { pointer: pointer.clone(), depth, label, node });
        if !open || rows.len() > limit {
            return;
        }
        for (key, child) in node.children() {
            let below = format!("{pointer}/{}", escape(&key));
            walk(rows, child, below, key, depth + 1, expanded, limit);
        }
    }
    let mut rows = Vec::new();
    walk(&mut rows, root, String::new(), String::new(), 0, expanded, limit);
    rows
}

/// The pointers of all containers, for Expand All. At most 2,000.
pub fn all_containers(root: &JsonNode) -> HashSet<String> {
    fn walk(node: &JsonNode, pointer: String, found: &mut HashSet<String>) {
        if !node.is_container() || found.len() >= 2000 {
            return;
        }
        found.insert(pointer.clone());
        for (key, child) in node.children() {
            walk(child, format!("{pointer}/{}", escape(&key)), found);
        }
    }
    let mut found = HashSet::new();
    walk(root, String::new(), &mut found);
    found
}

/// Port of `friendly_identity_name` in `src/app.rs`: a friendly name omits the assumed
/// `.VRSC` root and keeps every other namespace, the casing and the `@`.
pub fn friendly(name: &str) -> String {
    let (body, marker) = match name.strip_suffix('@') {
        Some(body) => (body, "@"),
        None => (name, ""),
    };
    let chars: Vec<char> = body.chars().collect();
    if chars.len() <= 5 {
        return name.to_string();
    }
    let tail: String = chars[chars.len() - 5..].iter().collect();
    if tail.to_lowercase() != ".vrsc" {
        return name.to_string();
    }
    let head: String = chars[..chars.len() - 5].iter().collect();
    format!("{head}{marker}")
}

/// The colour role of a value that is not a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueTone {
    Label,
    Record,
    Value,
    Mined,
}

/// The text of a value as the line shows it, before links, names and dates: `{3}` for an
/// object, `[2]` for an array, a string quoted and cut at the preview length unless it is
/// open. `friendly_name` is the friendly form for a `friendlyname` member.
pub fn value_text(node: &JsonNode, open: bool, friendly_name: Option<&str>) -> (String, ValueTone) {
    match node {
        JsonNode::Object(members) => (format!("{{{}}}", members.len()), ValueTone::Label),
        JsonNode::Array(items) => (format!("[{}]", items.len()), ValueTone::Label),
        JsonNode::Number(text) => (text.clone(), ValueTone::Record),
        JsonNode::Bool(flag) => (if *flag { "true" } else { "false" }.into(), ValueTone::Mined),
        JsonNode::Null => ("null".into(), ValueTone::Mined),
        JsonNode::String(text) => {
            let long = text.chars().count() > PREVIEW_LENGTH;
            let shown = friendly_name.unwrap_or(text);
            let shown = if long && !open {
                format!("{}…", shown.chars().take(PREVIEW_LENGTH).collect::<String>())
            } else {
                shown.to_string()
            };
            (quote(&shown), ValueTone::Value)
        }
    }
}

/// The page that a mark's link opens.
pub fn route(mark: &api::JsonMark) -> Option<Route> {
    let target = mark.target.clone()?;
    Some(match mark.link? {
        api::JsonLinkKind::Transaction => Route::Transaction { txid: target },
        api::JsonLinkKind::Block => Route::Block { id: target },
        api::JsonLinkKind::Address => Route::Address { address: target },
        api::JsonLinkKind::Identity => Route::Identity { name: target },
        api::JsonLinkKind::Currency => Route::Currency { name: target },
    })
}

/// The text of a link: the name for a nameable ID when names are on, else the raw value.
pub fn link_text(text: &str, mark: &api::JsonMark, label: &str, names_on: bool, names: &HashMap<String, String>) -> String {
    if names_on && label.to_lowercase() == "friendlyname" {
        return friendly(text);
    }
    if !names_on || !mark.nameable {
        return text.to_string();
    }
    match names.get(text) {
        Some(name) if mark.link == Some(api::JsonLinkKind::Currency) => name.strip_suffix('@').unwrap_or(name).to_string(),
        Some(name) => name.clone(),
        None => text.to_string(),
    }
}

/// The note under a tree that is cut at the row limit.
pub fn limit_note(count: usize) -> Option<String> {
    (count > ROW_LIMIT).then(|| {
        format!("The view shows the first {ROW_LIMIT} lines. Collapse a node, or save the document to read all of it.")
    })
}

// MARK: - The card

const HINT_SHOW: &str = "Show the data from the node";
const HINT_HIDE: &str = "Hide";

/// The "Raw JSON" card. `filename` is the file name that Save offers, with the object in it.
pub fn json_section(page: &Rc<Page>, json: &str, filename: &str) -> gtk::Box {
    let card = page.card("Raw JSON", None);
    // The label of the disclosure is in the style of the disclosure: small capitals, here
    // in the secondary colour, with the copy key at the right.
    let hint = theme::label(font::SMALL, HINT_SHOW, "secondary");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.set_margin_top(8);
    holder.set_visible(false);
    // Opens or closes the part. The tree is made when it opens and is gone when it
    // closes, as in the Swift app, so it opens with only the root expanded.
    let set_open: Rc<dyn Fn(bool)> = {
        let (page, holder, hint) = (Rc::downgrade(page), holder.clone(), hint.clone());
        let (json, filename) = (json.to_string(), filename.to_string());
        Rc::new(move |open: bool| {
            let Some(page) = page.upgrade() else { return };
            theme::set_label(&hint, font::SMALL, if open { HINT_HIDE } else { HINT_SHOW });
            page::load::clear(&holder);
            if open {
                holder.append(&json_tree(&page, &json, &filename));
            }
            holder.set_visible(open);
        })
    };
    let disclosure = Disclosure::new("", false, {
        let set_open = set_open.clone();
        move |open| set_open(open)
    });
    disclosure.header.append(&hint);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&disclosure.root);
    row.append(&spacer);
    let copy = page::copy_button(json);
    copy.set_valign(gtk::Align::Start);
    row.append(&copy);
    card.append(&row);
    card.append(&holder);

    // Tab shows the JSON and goes to it. Tab again hides it and goes to the top. The hook
    // holds the disclosure, which keeps its click working while the page lives.
    let cursor = page.cursor.clone();
    page.hooks().toggle_json = Some(Rc::new(move || {
        let open = !disclosure.expanded();
        disclosure.set_expanded(open);
        set_open(open);
        if open {
            // The tree gets its height after it reads the JSON. Go to the card when it
            // opens, and again when the tree is there.
            for delay in [150, 700] {
                let (cursor, disclosure) = (cursor.clone(), Rc::downgrade(&disclosure));
                glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                    if disclosure.upgrade().is_some_and(|d| d.expanded()) {
                        cursor.scroll_to_card("Raw JSON");
                    }
                });
            }
        } else {
            cursor.scroll_to_top();
        }
    }));
    card.root
}

/// The tree of a JSON document, with its toolbar (Expand All, Collapse All, Copy All,
/// Save), for a page to put where it shows JSON (`JSONTreeView`). Each line is a stop of
/// the page's cursor. While the tree is on screen, s saves its document as `filename`.
pub fn json_tree(page: &Rc<Page>, json: &str, filename: &str) -> gtk::Box {
    let tree = JsonTree::new(page, json, filename);
    let root = tree.root.upgrade().expect("the tree's root is new");
    // The tree lives as long as its widgets.
    root.connect_destroy(move |_| {
        let _ = &tree;
    });
    root
}

// MARK: - The tree

/// The tree of one document (`JSONTreeView`).
struct JsonTree {
    page: Weak<Page>,
    model: Rc<AppModel>,
    json: String,
    filename: String,
    /// Weak: the root's own handler keeps the tree.
    root: glib::WeakRef<gtk::Box>,
    body: gtk::Box,
    document: RefCell<Option<JsonNode>>,
    expanded: RefCell<HashSet<String>>,
    open_strings: RefCell<HashSet<String>>,
    marks: RefCell<HashMap<String, api::JsonMark>>,
    selected: RefCell<Option<String>>,
    /// The line widgets by pointer, to move the selection without a new layout.
    lines: RefCell<HashMap<String, gtk::Box>>,
    path: gtk::Label,
    /// The dates setting and the connection of the marks that are shown, to ask again when
    /// one of them changes.
    marks_key: Cell<Option<(bool, u64)>>,
    marks_request: Cell<u64>,
}

impl JsonTree {
    fn new(page: &Rc<Page>, json: &str, filename: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let path = page::text("", &["mono", "secondary", "caption-text"]);
        path.set_selectable(true);
        path.set_visible(false);
        let tree = Rc::new(Self {
            page: Rc::downgrade(page),
            model: page.model.clone(),
            json: json.to_string(),
            filename: filename.to_string(),
            root: root.downgrade(),
            body: body.clone(),
            document: RefCell::new(None),
            expanded: RefCell::new(HashSet::from([String::new()])),
            open_strings: RefCell::default(),
            marks: RefCell::default(),
            selected: RefCell::new(None),
            lines: RefCell::default(),
            path: path.clone(),
            marks_key: Cell::new(None),
            marks_request: Cell::new(0),
        });
        root.append(&tree.toolbar());
        root.append(&body);
        let progress = loader::small();
        progress.set_halign(gtk::Align::Start);
        body.append(&progress);

        // s saves the document while the tree is on screen (`saveJSON` of the cursor).
        let weak = Rc::downgrade(&tree);
        let save: page::Action = Rc::new(move || {
            if let Some(tree) = weak.upgrade() {
                tree.save();
            }
        });
        let (weak_page, hook) = (Rc::downgrade(page), save.clone());
        root.connect_map(move |_| {
            if let Some(page) = weak_page.upgrade() {
                page.hooks().save_json = Some(hook.clone());
            }
        });
        let (weak_page, hook) = (Rc::downgrade(page), save);
        root.connect_unmap(move |_| {
            if let Some(page) = weak_page.upgrade() {
                let mut hooks = page.hooks();
                if hooks.save_json.as_ref().is_some_and(|h| Rc::ptr_eq(h, &hook)) {
                    hooks.save_json = None;
                }
            }
        });

        // Read the document off the main thread.
        let (weak, text) = (Rc::downgrade(&tree), json.to_string());
        glib::spawn_future_local(async move {
            let parsed = bridge::run_blocking(move || parse(&text)).await;
            let Some(tree) = weak.upgrade() else { return };
            let failed = parsed.is_none();
            *tree.document.borrow_mut() = parsed;
            if failed {
                tree.show_failed();
            } else {
                tree.render();
            }
        });
        tree.load_marks();

        // Names and dates follow the settings; a new connection asks for the marks again.
        let weak = Rc::downgrade(&tree);
        page.model.watch(&[Topic::Names, Topic::Display, Topic::Connection], &root, move |_| {
            let Some(tree) = weak.upgrade() else { return };
            if tree.marks_key.get() != Some((display::shows_dates(), tree.model.generation())) {
                tree.load_marks();
            }
            tree.render();
        });
        tree
    }

    fn toolbar(self: &Rc<Self>) -> gtk::Box {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let weak = Rc::downgrade(self);
        let expand = link::link("Expand All", false, move || {
            if let Some(tree) = weak.upgrade() {
                let all = tree.document.borrow().as_ref().map(all_containers);
                if let Some(all) = all {
                    *tree.expanded.borrow_mut() = all;
                    tree.render();
                }
            }
        });
        let weak = Rc::downgrade(self);
        let collapse = link::link("Collapse All", false, move || {
            if let Some(tree) = weak.upgrade() {
                *tree.expanded.borrow_mut() = HashSet::from([String::new()]);
                tree.render();
            }
        });
        for key in [&expand, &collapse] {
            if let Some(label) = link::label_of(key) {
                label.add_css_class("callout");
            }
            bar.append(key);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        bar.append(&spacer);
        let json = self.json.clone();
        let copy_all = icon_link(Icon::Copy, "Copy All", move |button| button.clipboard().set_text(&json));
        bar.append(&copy_all);
        let weak = Rc::downgrade(self);
        let save = icon_link(Icon::Save, "Save…", move |_| {
            if let Some(tree) = weak.upgrade() {
                tree.save();
            }
        });
        bar.append(&save);
        bar
    }

    /// Asks the bridge for the links and dates of the values, then for the names.
    fn load_marks(self: &Rc<Self>) {
        let Some(explorer) = self.model.explorer() else { return };
        let dates = display::shows_dates();
        self.marks_key.set(Some((dates, self.model.generation())));
        let request = self.marks_request.get() + 1;
        self.marks_request.set(request);
        let (weak, json, generation) = (Rc::downgrade(self), self.json.clone(), self.model.generation());
        glib::spawn_future_local(async move {
            let found = bridge::run(async move { explorer.json_marks(json, dates).await }).await;
            let Some(tree) = weak.upgrade() else { return };
            if tree.marks_request.get() != request || tree.model.generation() != generation {
                return;
            }
            let nameable: Vec<String> = found.iter().filter(|m| m.nameable).filter_map(|m| m.target.clone()).collect();
            *tree.marks.borrow_mut() = found.into_iter().map(|m| (m.pointer.clone(), m)).collect();
            tree.render();
            if display::shows_names() {
                tree.model.resolve_names(nameable);
            }
        });
    }

    fn show_failed(&self) {
        page::load::clear(&self.body);
        let text = page::text(&self.json, &["mono"]);
        text.set_selectable(true);
        text.set_valign(gtk::Align::Start);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&text));
        self.body.append(&scroller);
    }

    /// Lays out the lines again.
    fn render(self: &Rc<Self>) {
        let document = self.document.borrow();
        let Some(document) = document.as_ref() else { return };
        let Some(page) = self.page.upgrade() else { return };
        page::load::clear(&self.body);
        self.lines.borrow_mut().clear();
        let expanded = self.expanded.borrow().clone();
        let rows = visible_rows(document, &expanded, ROW_LIMIT);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for row in rows.iter().take(ROW_LIMIT) {
            let line = self.line(&page, row, expanded.contains(&row.pointer));
            column.append(&line);
            self.lines.borrow_mut().insert(row.pointer.clone(), line);
        }
        self.body.append(&column);
        if let Some(note) = limit_note(rows.len()) {
            self.body.append(&page::paragraph(&note, &["secondary", "caption-text"]));
        }
        self.body.append(&self.path);
        self.show_path();
    }

    fn show_path(&self) {
        match self.selected.borrow().as_deref() {
            Some(pointer) => {
                self.path.set_text(&format!("Path {}", if pointer.is_empty() { "/" } else { pointer }));
                self.path.set_visible(true);
            }
            None => self.path.set_visible(false),
        }
    }

    fn select(&self, pointer: &str) {
        let previous = self.selected.replace(Some(pointer.to_string()));
        let lines = self.lines.borrow();
        if let Some(line) = previous.and_then(|p| lines.get(&p).cloned()) {
            line.remove_css_class("json-selected");
        }
        if let Some(line) = lines.get(pointer) {
            line.add_css_class("json-selected");
        }
        drop(lines);
        self.show_path();
    }

    fn toggle(self: &Rc<Self>, pointer: &str) {
        {
            let mut expanded = self.expanded.borrow_mut();
            if !expanded.remove(pointer) {
                expanded.insert(pointer.to_string());
            }
        }
        // Change the layout after the key or click that asked for it is done.
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(tree) = weak.upgrade() {
                tree.render();
            }
        });
    }

    fn open_route(&self, route: Route) {
        self.model.open(route);
    }

    /// One line: the mark that opens a node, the member name and the value.
    fn line(self: &Rc<Self>, page: &Rc<Page>, row: &JsonRow, open: bool) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        line.add_css_class("json-row");
        line.set_margin_start(row.depth as i32 * 16);
        if self.selected.borrow().as_deref() == Some(row.pointer.as_str()) {
            line.add_css_class("json-selected");
        }
        let pointer = row.pointer.clone();
        if row.node.is_container() {
            let key = gtk::Button::new();
            key.add_css_class("json-chevron");
            key.set_child(Some(&icons::icon(if open { Icon::Down } else { Icon::Chevron }, 10, tone::LABEL)));
            key.set_valign(gtk::Align::Center);
            key.set_tooltip_text(Some(if open { "Collapse" } else { "Expand" }));
            crate::widgets::lamp::key_sounds(&key, false);
            let (weak, pointer) = (Rc::downgrade(self), pointer.clone());
            key.connect_clicked(move |_| {
                if let Some(tree) = weak.upgrade() {
                    tree.toggle(&pointer);
                }
            });
            line.append(&key);
        } else {
            let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            space.set_size_request(12, -1);
            line.append(&space);
        }
        if !row.label.is_empty() {
            let name = page::text(&format!("{}:", row.label), &["mono", "label"]);
            name.set_valign(gtk::Align::Baseline);
            line.append(&name);
        }
        let mark = self.marks.borrow().get(&row.pointer).cloned();
        let target_route = mark.as_ref().and_then(route);
        self.value(&line, row, mark.as_ref(), target_route.clone());
        if display::shows_dates()
            && let Some(date) = mark.as_ref().and_then(|m| m.date)
        {
            let estimated = mark.as_ref().is_some_and(|m| m.estimated);
            let text = format!("{}{}", if estimated { "est. " } else { "" }, format::date(date));
            let label = page::text(&text, &["label", "caption-text"]);
            label.set_valign(gtk::Align::Baseline);
            line.append(&label);
        }

        // A click selects the line. The right key opens the menu of the line.
        let click = gtk::GestureClick::new();
        let (weak, pointer) = (Rc::downgrade(self), row.pointer.clone());
        click.connect_released(move |_, _, _, _| {
            if let Some(tree) = weak.upgrade() {
                tree.select(&pointer);
            }
        });
        line.add_controller(click);
        let menu = gtk::GestureClick::new();
        menu.set_button(3);
        let (weak, pointer, node_copy, node_pretty, container) = (
            Rc::downgrade(self),
            row.pointer.clone(),
            row.node.copy_text(),
            row.node.is_container().then(|| row.node.pretty()),
            row.node.is_container(),
        );
        let menu_route = target_route.clone();
        menu.connect_pressed(move |gesture, _, x, y| {
            let Some(tree) = weak.upgrade() else { return };
            let Some(widget) = gesture.widget() else { return };
            tree.select(&pointer);
            tree.menu(&widget, x, y, &pointer, &node_copy, node_pretty.as_deref(), container, open, menu_route.clone());
        });
        line.add_controller(menu);

        // The cursor of the page: y copies the value, Return follows an ID or opens a node.
        let (weak, pointer, container) = (Rc::downgrade(self), row.pointer.clone(), row.node.is_container());
        page.stop(&line, Some(format!("json:{}", row.pointer)), Some(row.node.copy_text()), move || {
            let Some(tree) = weak.upgrade() else { return };
            if let Some(route) = target_route.clone() {
                tree.open_route(route);
            } else if container {
                tree.toggle(&pointer);
            }
        });
        line
    }

    fn value(self: &Rc<Self>, line: &gtk::Box, row: &JsonRow, mark: Option<&api::JsonMark>, target: Option<Route>) {
        let names_on = display::shows_names();
        let JsonNode::String(text) = row.node else {
            let (shown, tone) = value_text(row.node, false, None);
            let class = match tone {
                ValueTone::Label => "label",
                ValueTone::Record => "record",
                ValueTone::Value => "value",
                ValueTone::Mined => "mined",
            };
            let label = page::text(&shown, &["mono", class]);
            label.set_valign(gtk::Align::Baseline);
            if matches!(row.node, JsonNode::Number(_)) {
                label.set_selectable(true);
            }
            line.append(&label);
            return;
        };
        let long = text.chars().count() > PREVIEW_LENGTH;
        let open = self.open_strings.borrow().contains(&row.pointer);
        let friendly_name = (names_on && row.label.to_lowercase() == "friendlyname").then(|| friendly(text));
        match (mark, target) {
            (Some(mark), Some(route)) => {
                let shown = link_text(text, mark, &row.label, names_on, &self.model.state().names);
                let model = Rc::downgrade(&self.model);
                let button = link::link(&shown, true, move || {
                    if let Some(model) = model.upgrade() {
                        model.open(route.clone());
                    }
                });
                button.set_tooltip_text(mark.target.as_deref());
                line.append(&button);
            }
            _ => {
                let (shown, _) = value_text(row.node, open, friendly_name.as_deref());
                let label = page::text(&shown, &["mono", "value"]);
                label.set_selectable(true);
                label.set_valign(gtk::Align::Baseline);
                if open {
                    label.set_wrap(true);
                    label.set_wrap_mode(gtk::pango::WrapMode::Char);
                } else {
                    label.set_single_line_mode(true);
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                }
                label.set_hexpand(open);
                line.append(&label);
            }
        }
        if long {
            let title = if open { "Collapse".to_string() } else { format!("Show all {} characters", text.chars().count()) };
            let (weak, pointer) = (Rc::downgrade(self), row.pointer.clone());
            let key = link::link(&title, false, move || {
                let Some(tree) = weak.upgrade() else { return };
                {
                    let mut strings = tree.open_strings.borrow_mut();
                    if !strings.remove(&pointer) {
                        strings.insert(pointer.clone());
                    }
                }
                let weak = Rc::downgrade(&tree);
                glib::idle_add_local_once(move || {
                    if let Some(tree) = weak.upgrade() {
                        tree.render();
                    }
                });
            });
            if let Some(label) = link::label_of(&key) {
                label.add_css_class("caption-text");
            }
            line.append(&key);
        }
    }

    /// The menu of a line (`contextMenu`).
    #[allow(clippy::too_many_arguments)]
    fn menu(
        self: &Rc<Self>,
        widget: &gtk::Widget,
        x: f64,
        y: f64,
        pointer: &str,
        copy: &str,
        pretty: Option<&str>,
        container: bool,
        open: bool,
        target: Option<Route>,
    ) {
        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        popover.set_child(Some(&list));
        popover.set_parent(widget);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let add = |title: &str, action: Box<dyn Fn()>| {
            let item = gtk::Button::with_label(title);
            item.add_css_class("flat");
            item.add_css_class("menu-row");
            if let Some(label) = item.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let popover = popover.downgrade();
            item.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                action();
            });
            list.append(&item);
        };
        let clipboard = widget.clipboard();
        let copy_value = copy.to_string();
        let board = clipboard.clone();
        add(
            if container { "Copy Subtree as JSON" } else { "Copy Value" },
            Box::new(move || board.set_text(&copy_value)),
        );
        if let Some(pretty) = pretty {
            let (board, pretty) = (clipboard.clone(), pretty.to_string());
            add("Copy Subtree as Pretty JSON", Box::new(move || board.set_text(&pretty)));
            let (weak, pointer) = (Rc::downgrade(self), pointer.to_string());
            add(
                if open { "Collapse" } else { "Expand" },
                Box::new(move || {
                    if let Some(tree) = weak.upgrade() {
                        tree.toggle(&pointer);
                    }
                }),
            );
        }
        let (board, path) = (clipboard.clone(), if pointer.is_empty() { "/".to_string() } else { pointer.to_string() });
        add("Copy Path", Box::new(move || board.set_text(&path)));
        if let Some(route) = target {
            list.append(&page::divider());
            let model = Rc::downgrade(&self.model);
            add(
                "Open",
                Box::new(move || {
                    if let Some(model) = model.upgrade() {
                        model.open(route.clone());
                    }
                }),
            );
        }
        popover.connect_closed(|popover| {
            let popover = popover.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(popover) = popover.upgrade() {
                    popover.unparent();
                }
            });
        });
        popover.popup();
    }

    /// Saves the document. The bridge's documents are already pretty; others are indented
    /// here.
    fn save(self: &Rc<Self>) {
        let Some(window) = self.root.upgrade().and_then(|r| r.root()).and_downcast::<gtk::Window>() else { return };
        let dialog = gtk::FileDialog::new();
        dialog.set_initial_name(Some(&self.filename));
        dialog.set_modal(true);
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("JSON"));
        filter.add_suffix("json");
        filter.add_mime_type("application/json");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        let text = if self.json.contains('\n') {
            self.json.clone()
        } else {
            self.document.borrow().as_ref().map(JsonNode::pretty).unwrap_or_else(|| self.json.clone())
        };
        let parent = window.clone();
        dialog.save(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            if let Err(error) = std::fs::write(&path, text.as_bytes()) {
                crate::shell::dialogs::alert(&parent, "The file was not saved", &error.to_string(), || {});
            }
        });
    }
}

// MARK: - Small marks

/// A link with a mark before its text (`Label(title, systemImage:)` in a borderless
/// button). The mark takes the colour of the link, also under the pointer.
fn icon_link(glyph: Icon, title: &str, action: impl Fn(&gtk::Button) + 'static) -> gtk::Button {
    let action = Rc::new(action);
    let slot: Rc<RefCell<Option<glib::WeakRef<gtk::Button>>>> = Rc::default();
    let button = {
        let (slot, action) = (slot.clone(), action.clone());
        link::link(title, false, move || {
            if let Some(button) = slot.borrow().as_ref().and_then(|b| b.upgrade()) {
                action(&button);
            }
        })
    };
    *slot.borrow_mut() = Some(button.downgrade());
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let mark = icons::text_icon(glyph, 12);
    mark.set_valign(gtk::Align::Center);
    let label = page::text(title, &["callout"]);
    row.append(&mark);
    row.append(&label);
    button.set_child(Some(&row));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(link: Option<api::JsonLinkKind>, target: Option<&str>, nameable: bool) -> api::JsonMark {
        api::JsonMark {
            pointer: "/x".into(),
            link,
            target: target.map(str::to_string),
            nameable,
            date: None,
            estimated: false,
        }
    }

    #[test]
    fn parse_keeps_order_and_number_text() {
        let doc = parse(r#"{"b": 1.50, "a": [1e3, -0, true, null], "s": "x\"é😀"}"#).unwrap();
        let JsonNode::Object(members) = &doc else { panic!() };
        assert_eq!(members.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["b", "a", "s"]);
        assert_eq!(members[0].1, JsonNode::Number("1.50".into()));
        assert_eq!(members[1].1.compact(), "[1e3,-0,true,null]");
        assert_eq!(members[2].1.copy_text(), "x\"é😀");
        assert_eq!(doc.compact(), r#"{"b":1.50,"a":[1e3,-0,true,null],"s":"x\"é😀"}"#);
        assert!(parse("{\"a\":1,}").is_none());
        assert!(parse("[1] x").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn pretty_and_copy() {
        let doc = parse(r#"{"a":{},"b":[],"c":[1,{"d":"e"}]}"#).unwrap();
        assert_eq!(doc.pretty(), "{\n  \"a\": {},\n  \"b\": [],\n  \"c\": [\n    1,\n    {\n      \"d\": \"e\"\n    }\n  ]\n}");
        assert_eq!(JsonNode::String("ab".into()).copy_text(), "ab");
        assert_eq!(JsonNode::Null.copy_text(), "null");
        assert_eq!(quote("a\u{1}\n"), "\"a\\u0001\\n\"");
    }

    #[test]
    fn rows_follow_the_open_nodes() {
        let doc = parse(r#"{"tx":["a","b"],"x/y":{"k~":1},"n":2}"#).unwrap();
        let only_root: HashSet<String> = [String::new()].into();
        let rows = visible_rows(&doc, &only_root, ROW_LIMIT);
        assert_eq!(rows.iter().map(|r| r.pointer.as_str()).collect::<Vec<_>>(), ["", "/tx", "/x~1y", "/n"]);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].label, "tx");
        let all = all_containers(&doc);
        assert_eq!(all.len(), 3);
        let rows = visible_rows(&doc, &all, ROW_LIMIT);
        assert_eq!(
            rows.iter().map(|r| r.pointer.as_str()).collect::<Vec<_>>(),
            ["", "/tx", "/tx/0", "/tx/1", "/x~1y", "/x~1y/k~0", "/n"]
        );
        assert_eq!(rows[2].label, "0");
        // A closed root shows one line.
        assert_eq!(visible_rows(&doc, &HashSet::new(), ROW_LIMIT).len(), 1);
    }

    #[test]
    fn row_limit_stops_the_walk() {
        let items: Vec<String> = (0..12).map(|i| format!("[{i},{i}]")).collect();
        let doc = parse(&format!("[{}]", items.join(","))).unwrap();
        let all = all_containers(&doc);
        let rows = visible_rows(&doc, &all, 10);
        // Past the limit the walk adds no more children, but each remaining sibling still
        // has its own line, as in the Swift view: 1 + 3 * 3 open items, then 9 closed ones.
        // The view shows the first `limit` lines and the note.
        assert_eq!(rows.len(), 19);
        assert!(rows[10..].iter().all(|r| r.depth == 1));
        assert_eq!(visible_rows(&doc, &all, ROW_LIMIT).len(), 1 + 12 * 3);
        assert!(limit_note(ROW_LIMIT).is_none());
        assert_eq!(
            limit_note(ROW_LIMIT + 1).unwrap(),
            "The view shows the first 5000 lines. Collapse a node, or save the document to read all of it."
        );
    }

    #[test]
    fn values_render_as_the_swift_view() {
        let doc = parse(r#"{"o":{"a":1},"a":[1,2],"n":12.0,"t":false,"z":null,"s":"hi"}"#).unwrap();
        let JsonNode::Object(m) = &doc else { panic!() };
        assert_eq!(value_text(&m[0].1, false, None), ("{1}".into(), ValueTone::Label));
        assert_eq!(value_text(&m[1].1, false, None), ("[2]".into(), ValueTone::Label));
        assert_eq!(value_text(&m[2].1, false, None), ("12.0".into(), ValueTone::Record));
        assert_eq!(value_text(&m[3].1, false, None), ("false".into(), ValueTone::Mined));
        assert_eq!(value_text(&m[4].1, false, None), ("null".into(), ValueTone::Mined));
        assert_eq!(value_text(&m[5].1, false, None), ("\"hi\"".into(), ValueTone::Value));
        let long = JsonNode::String("x".repeat(130));
        let (shown, _) = value_text(&long, false, None);
        assert_eq!(shown.chars().count(), 120 + 3);
        assert!(shown.ends_with("…\""));
        assert_eq!(value_text(&long, true, None).0.chars().count(), 132);
        assert_eq!(value_text(&JsonNode::String("a.VRSC@".into()), false, Some("a@")).0, "\"a@\"");
    }

    #[test]
    fn friendly_names_drop_the_vrsc_root() {
        assert_eq!(friendly("Alice.VRSC@"), "Alice@");
        assert_eq!(friendly("bob.vrsc"), "bob");
        assert_eq!(friendly("x.vETH.VRSC@"), "x.vETH@");
        assert_eq!(friendly("vrsc@"), "vrsc@");
        assert_eq!(friendly(".VRSC"), ".VRSC");
        assert_eq!(friendly("bridge.vETH@"), "bridge.vETH@");
    }

    #[test]
    fn links_open_the_object_and_show_names() {
        let tx = mark(Some(api::JsonLinkKind::Transaction), Some("ab12"), false);
        assert_eq!(route(&tx), Some(Route::Transaction { txid: "ab12".into() }));
        assert_eq!(route(&mark(Some(api::JsonLinkKind::Block), Some("5"), false)), Some(Route::Block { id: "5".into() }));
        assert_eq!(
            route(&mark(Some(api::JsonLinkKind::Address), Some("R1"), false)),
            Some(Route::Address { address: "R1".into() })
        );
        assert_eq!(route(&mark(None, Some("x"), false)), None);
        assert_eq!(route(&mark(Some(api::JsonLinkKind::Block), None, false)), None);

        let names: HashMap<String, String> =
            [("iCur".to_string(), "vETH@".to_string()), ("iId".to_string(), "alice@".to_string())].into();
        let currency = mark(Some(api::JsonLinkKind::Currency), Some("iCur"), true);
        assert_eq!(link_text("iCur", &currency, "currencyid", true, &names), "vETH");
        assert_eq!(link_text("iCur", &currency, "currencyid", false, &names), "iCur");
        let identity = mark(Some(api::JsonLinkKind::Identity), Some("iId"), true);
        assert_eq!(link_text("iId", &identity, "parent", true, &names), "alice@");
        // `identityaddress` stays raw: the bridge marks it as not nameable.
        let raw = mark(Some(api::JsonLinkKind::Identity), Some("iId"), false);
        assert_eq!(link_text("iId", &raw, "identityaddress", true, &names), "iId");
        assert_eq!(link_text("Bob.VRSC@", &raw, "friendlyname", true, &names), "Bob@");
    }
}
