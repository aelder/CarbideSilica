//! Carbide Silica for Linux: the GTK 4 port of the macOS explorer for Verus.
//!
//! `linux/PORT.md` describes the structure. The app calls the bridge crate
//! (`macos/bridge/rust`, `carbide_bridge::api`) for all of its data, and is read-only.

#![allow(dead_code)] // The design system and the model are a toolkit for the pages that follow.

mod bridge;
mod display;
mod format;
mod launch;
mod model;
mod page;
mod pages;
mod route;
mod settings;
mod shared;
mod shell;
mod theme;
mod wait_record;
mod widgets;

use gtk::glib;
use gtk::prelude::*;

/// The application ID. It is also the name of the icon, the desktop file and the AppStream
/// metainfo file (`linux/data`).
pub const APP_ID: &str = "org.carbide.CarbideSilica";

fn main() -> glib::ExitCode {
    // The fonts must be known to fontconfig before GTK makes its font map.
    if !theme::fonts::register() {
        eprintln!("The app did not find its fonts in {}.", theme::fonts::resources().join("fonts").display());
    }
    // The start-up arguments are Carbide's (`-conf=`, `-chain=`, ...). GTK must not read them.
    let _ = launch::arguments();
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|_| theme::install());
    app.connect_activate(|app| {
        let shell = shell::build(app);
        // The shell lives as long as its window.
        let keep = std::cell::RefCell::new(Some(shell));
        app.connect_shutdown(move |_| {
            keep.borrow_mut().take();
        });
    });
    let program = std::env::args().next().unwrap_or_else(|| "carbide-silica".into());
    app.run_with_args(&[program])
}
