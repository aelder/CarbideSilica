//! Owner: the Currency page (`pages/currency`). Helpers of the currency pages
//! (`CurrencyStores.swift`): `knownName`, `observationSource`, `heightText` and
//! `OtherChainNote` are here and complete. `CurrencyBrowserState` (owner: the Currencies
//! page) and `NotarizationRequests` (owner: the Currency page; the Bookmarks page uses it)
//! are not ported yet.

use std::rc::Rc;

use gtk::prelude::*;

use crate::display;
use crate::format;
use crate::model::AppModel;
use crate::page;
use crate::widgets::link;

/// A currency or system name for display: a resolved name without the trailing "@", else
/// the given fallback, else a shortened ID. When names are off, the ID.
pub fn known_name(id: &str, model: &AppModel, fallback: Option<&str>) -> String {
    if !display::shows_names() {
        return id.to_string();
    }
    if model.state().status.as_ref().and_then(|s| s.chain_id.as_deref()) == Some(id) {
        return model.coin();
    }
    if let Some(name) = model.state().names.get(id) {
        return name.strip_suffix('@').unwrap_or(name).to_string();
    }
    match fallback.filter(|f| !f.is_empty()) {
        Some(fallback) => fallback.to_string(),
        None => format::short(id, 6, 4),
    }
}

/// Carbide's source IDs are `kind:digest`. Show the kind and a short digest.
pub fn observation_source(source: &str) -> String {
    let parts: Vec<&str> = source.splitn(2, ':').collect();
    if parts.len() != 2 {
        return source.to_string();
    }
    let word = match parts[0] {
        "local-daemon" => "Local node",
        "public-api" => "Public API",
        other => other,
    };
    format!("{word} {}", format::short(parts[1], 6, 4))
}

/// A height as a number, or as a date when dates are on.
pub fn height_text(height: u64, known: Option<u64>, model: &Rc<AppModel>) -> String {
    display::height_text(height, known, model)
}

/// Names the chain of a block or transaction that is on another chain. The app shows one
/// chain at a time, so the object stays text. When a node for that chain is configured on
/// this computer, the person can make it the current chain (`OtherChainNote`).
pub fn other_chain_note(model: &Rc<AppModel>, window: &gtk::Window, system_id: &str, fallback: Option<&str>) -> gtk::Box {
    let name = known_name(system_id, model, fallback);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&page::text(&format!("on {name}"), &["caption-text", "label"]));
    let node = model.explorer().and_then(|explorer| {
        explorer.sources_for_system(system_id.to_string()).into_iter().find(|source| {
            source.kind == crate::bridge::api::SourceKind::LocalNode && source.config_file.is_some() && !source.current
        })
    });
    if let Some(source) = node {
        let (weak, window, title) = (Rc::downgrade(model), window.downgrade(), name.clone());
        let switch = link::link("Switch Chain…", false, move || {
            let (Some(model), Some(window)) = (weak.upgrade(), window.upgrade()) else { return };
            let (source, weak) = (source.clone(), Rc::downgrade(&model));
            crate::shell::dialogs::confirm(
                &window,
                &format!("Switch to {title}?"),
                &format!("The app connects to the {title} node and closes the pages that are open now."),
                "Switch Chain",
                move || {
                    if let Some(model) = weak.upgrade() {
                        model.switch_chain_to_source(&source);
                    }
                },
            );
        });
        switch.add_css_class("caption-text");
        switch.set_tooltip_text(Some(&format!("Make {name} the current chain. The app then shows its blocks and transactions.")));
        row.append(&switch);
    }
    row
}
