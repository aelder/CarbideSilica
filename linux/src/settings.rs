//! Stored settings: the Linux form of the Swift app's `UserDefaults`.
//!
//! The values are in one JSON file, `$XDG_CONFIG_HOME/carbide-silica/settings.json`. The
//! keys are the same as on macOS (`SettingsKey`, `DisplayKey` and `ClickSound.settingKey`).
//! A write saves the file at once. Code that must redraw on a change subscribes with
//! `model.watch(Topic::Display, ...)`: `AppModel` sends that topic after each change of a
//! display key.

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{Map, Value};

/// Keys of the connection settings (`SettingsKey` in `AppModel.swift`).
pub mod key {
    pub const CHAIN: &str = "chain";
    pub const TESTNET: &str = "testnet";
    pub const CONFIG_FILE: &str = "configFile";
    pub const API_ENABLED: &str = "apiEnabled";
    pub const CHAIN_CONFIGS: &str = "chainConfigs";
    pub const CACHE_DIR: &str = "cacheDir";
    pub const USE_UTC: &str = "useUTC";
    pub const API_URL: &str = "apiURL";
    /// Sources: show the activity of every source, not only the selected one.
    pub const ALL_SOURCE_ACTIVITY: &str = "allSourceActivity";
    /// Show names in place of identity and currency IDs. On by default.
    pub const SHOWS_NAMES: &str = "showsNames";
    /// Show dates for block heights, confirmation counts and Unix timestamps. Off by default.
    pub const SHOWS_DATES: &str = "showsDates";
    /// The interface sounds. On by default.
    pub const INTERFACE_SOUNDS: &str = "interfaceSounds";
    pub const SEARCH_HISTORY: &str = "searchHistory";
    pub const DASHBOARD_STREAMS_BLOCKS: &str = "dashboardStreamsBlocks";

    pub const DEFAULT_API: &str = "https://api.verus.services";
}

struct Store {
    path: Option<PathBuf>,
    values: Map<String, Value>,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::load());
}

impl Store {
    fn load() -> Self {
        // CARBIDE_SILICA_SETTINGS names a different file, for tests and pictures.
        let path = std::env::var_os("CARBIDE_SILICA_SETTINGS")
            .map(PathBuf::from)
            .or_else(|| {
                directories::ProjectDirs::from("", "", "carbide-silica")
                    .map(|dirs| dirs.config_dir().join("settings.json"))
            });
        let values = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        Self { path, values }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(self.values.clone())) {
            let _ = std::fs::write(path, text);
        }
    }
}

pub fn value(key: &str) -> Option<Value> {
    STORE.with(|store| store.borrow().values.get(key).cloned())
}

pub fn set_value(key: &str, value: Option<Value>) {
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        match value {
            Some(value) => store.values.insert(key.to_string(), value),
            None => store.values.remove(key),
        };
        store.save();
    });
}

pub fn bool(key: &str, default: bool) -> bool {
    value(key).and_then(|value| value.as_bool()).unwrap_or(default)
}

pub fn set_bool(key: &str, on: bool) {
    set_value(key, Some(Value::Bool(on)));
}

/// Turns a setting on or off and returns the new value.
pub fn toggle(key: &str, default: bool) -> bool {
    let on = !bool(key, default);
    set_bool(key, on);
    on
}

pub fn string(key: &str) -> Option<String> {
    value(key).and_then(|value| value.as_str().map(str::to_string))
}

pub fn set_string(key: &str, text: &str) {
    set_value(key, Some(Value::String(text.to_string())));
}

pub fn strings(key: &str) -> Vec<String> {
    value(key)
        .and_then(|value| value.as_array().cloned())
        .map(|list| list.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn set_strings(key: &str, list: &[String]) {
    set_value(key, Some(Value::Array(list.iter().cloned().map(Value::String).collect())));
}

pub fn string_map(key: &str) -> std::collections::BTreeMap<String, String> {
    value(key)
        .and_then(|value| value.as_object().cloned())
        .map(|map| {
            map.into_iter()
                .filter_map(|(name, value)| value.as_str().map(|text| (name, text.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

pub fn set_string_map(key: &str, map: &std::collections::BTreeMap<String, String>) {
    let object: Map<String, Value> =
        map.iter().map(|(name, value)| (name.clone(), Value::String(value.clone()))).collect();
    set_value(key, Some(Value::Object(object)));
}
