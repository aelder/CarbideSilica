//! The design of the app: the display of a ship's computer (`DisplayTheme.swift`).
//!
//! The app is a black display. Colour is a signal and not decoration: violet is staked,
//! amber is mined, warm white is the present and the selection, pale blue is a link and a
//! part of a record that is done, red is an alert. Lines are thin and flat. Light is for
//! the things that are live. Names and figures are in Jost, the open face that takes the
//! place of Futura. Long text is in Inter and IDs are in IBM Plex Mono, because a person
//! must read them and compare them.
//!
//! The colours are here as constants for drawing ([`Color`]) and in `style.css` as named
//! colours (`@screen`, `@rule` and so on) for widgets. Keep the two in step.

pub mod draw;
pub mod fonts;
pub mod sound;

use gtk::gdk;
use gtk::prelude::*;

/// A colour of the display, in sRGB from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// The colour with its alpha multiplied by `factor`, as SwiftUI's `.opacity()`.
    pub fn opacity(self, factor: f64) -> Self {
        Self { a: self.a * factor as f32, ..self }
    }

    pub fn rgba(self) -> gdk::RGBA {
        gdk::RGBA::new(self.r, self.g, self.b, self.a)
    }

    /// The colour as CSS: `rgba(…)`.
    pub fn css(self) -> String {
        format!(
            "rgba({},{},{},{})",
            (self.r * 255.0).round(),
            (self.g * 255.0).round(),
            (self.b * 255.0).round(),
            self.a
        )
    }
}

/// The colours of the display (`Tone` in the Swift app).
///
/// The colours are those of the display on film stock: the white is a pale grey-green at
/// four fifths of full light, a signal colour is a pale core with its colour in the light
/// around it, and the black is a little above zero.
pub mod tone {
    use super::Color;

    pub const SCREEN: Color = Color::rgb(0.012, 0.012, 0.016);
    /// The border of a panel and the rule above a card.
    pub const RULE: Color = Color::rgb(0.75, 0.8, 0.75).with_alpha(0.15);
    pub const GRID: Color = Color::rgb(0.75, 0.8, 0.75).with_alpha(0.09);
    pub const AXIS: Color = Color::rgb(0.75, 0.8, 0.75).with_alpha(0.36);
    /// Names of things: axis labels, readout names.
    pub const LABEL: Color = Color::rgb(0.62, 0.67, 0.64);
    /// Figures and text that a person reads.
    pub const VALUE: Color = Color::rgb(0.75, 0.8, 0.75);
    /// The present and the selection: the lightest tone of the display.
    pub const PRESENT: Color = Color::rgb(0.8, 0.84, 0.77);
    pub const STAKED: Color = Color::rgb(0.72, 0.68, 0.8);
    pub const MINED: Color = Color::rgb(0.74, 0.67, 0.44);
    /// A link, and the part of a record that is done or passed.
    pub const RECORD: Color = Color::rgb(0.5, 0.74, 0.71);
    pub const GOOD: Color = Color::rgb(0.26, 0.65, 0.48);
    pub const ALERT: Color = Color::rgb(0.82, 0.33, 0.24);
    /// The light around a staked mark and around a mined mark. On film the core of a
    /// strong colour goes pale, and the colour is in the light around it.
    pub const STAKED_GLOW: Color = Color::rgb(0.56, 0.3, 0.86);
    pub const MINED_GLOW: Color = Color::rgb(1.0, 0.5, 0.12);
    /// Secondary text: SwiftUI's `.secondary` on the white of the display.
    pub const SECONDARY: Color = Color::rgb(0.75, 0.8, 0.75).with_alpha(0.6);
    /// SwiftUI's `.tertiary`.
    pub const TERTIARY: Color = Color::rgb(0.75, 0.8, 0.75).with_alpha(0.38);
    /// The system colours that the Swift app uses for the state of a source.
    pub const SYSTEM_GREEN: Color = Color::rgb(0.19, 0.82, 0.35);
    pub const SYSTEM_ORANGE: Color = Color::rgb(1.0, 0.62, 0.04);
    /// Series of a chart, in order, when a chart has more than the named signals.
    pub const SERIES: [Color; 6] = [RECORD, MINED, STAKED, GOOD, Color::rgb(0.78, 0.5, 0.56), VALUE];
}

/// The colours of the lamps and of a legend on a lit key (`LampTone`).
pub mod lamp {
    use super::Color;

    pub const BLUE_CENTRE: Color = Color::rgb(0.62, 0.73, 0.73);
    pub const BLUE_EDGE: Color = Color::rgb(0.42, 0.57, 0.6);
    pub const AMBER_CENTRE: Color = Color::rgb(0.71, 0.74, 0.64);
    pub const AMBER_EDGE: Color = Color::rgb(0.62, 0.53, 0.44);
    /// A legend on a lit key: a dark red-brown, as a legend on film is.
    pub const LEGEND: Color = Color::rgb(0.33, 0.24, 0.2);
    /// The red-orange edge that film gives to a strong light.
    pub const HALATION: Color = Color::rgb(0.95, 0.33, 0.14);
    /// The face of a key that is off.
    pub const FACE: Color = Color::rgb(0.045, 0.045, 0.045);
}

/// The type roles. Each is a font, a size in logical pixels, a tracking and a case.
pub mod font {
    /// The face of the Futura roles.
    pub const DISPLAY: &str = "Jost";
    /// The face of text that a person reads as sentences: the system face on macOS.
    pub const TEXT: &str = "Inter";
    /// The face of IDs and hashes: the monospaced system face on macOS.
    pub const MONO: &str = "IBM Plex Mono";

    #[derive(Clone, Copy, Debug)]
    pub struct Role {
        pub family: &'static str,
        pub size: f64,
        /// 500 is Futura-Medium, 700 is Futura-Bold.
        pub weight: i32,
        /// Space between letters, in logical pixels (SwiftUI's `tracking`).
        pub tracking: f64,
        pub uppercase: bool,
        /// Figures of one width, as `monospacedDigit()`.
        pub tabular: bool,
    }

    const fn display(size: f64, weight: i32, tracking: f64, uppercase: bool) -> Role {
        Role { family: DISPLAY, size, weight, tracking, uppercase, tabular: false }
    }

    /// Axis labels and readout names.
    pub const SMALL: Role = display(9.5, 500, 1.1, true);
    /// The lettering of a plate.
    pub const PLATE: Role = display(8.5, 700, 0.8, true);
    /// A figure of a readout.
    pub const FIGURE: Role = Role { tabular: true, ..display(17.0, 500, 0.0, false) };
    pub const FIGURE_SMALL: Role = Role { tabular: true, ..display(11.5, 500, 0.0, false) };
    /// The heading of a card.
    pub const HEADING: Role = display(13.0, 500, 2.2, true);
    /// The title of a page.
    pub const TITLE: Role = display(26.0, 500, 0.0, false);
    /// The legend of a small key.
    pub const KEY: Role = display(10.5, 500, 1.2, true);
    /// The legend of a key of the sidebar.
    pub const PLACE: Role = display(11.5, 500, 1.6, true);
    /// Text in a panel that a person reads as a sentence.
    pub const CAPTION: Role = Role { family: TEXT, size: 11.5, weight: 400, tracking: 0.0, uppercase: false, tabular: false };
    /// Body text (`.body` on macOS).
    pub const BODY: Role = Role { family: TEXT, size: 13.0, weight: 400, tracking: 0.0, uppercase: false, tabular: false };
    /// `.callout` on macOS.
    pub const CALLOUT: Role = Role { size: 12.0, ..BODY };
    pub const MONO_CALLOUT: Role = Role { family: MONO, size: 12.0, weight: 400, tracking: 0.0, uppercase: false, tabular: false };
}

/// Loads the fonts, the colours and the style sheet. Call once after GTK starts.
pub fn install() {
    let Some(display) = gdk::Display::default() else { return };
    if let Some(settings) = gtk::Settings::for_display(&display).into() {
        let settings: gtk::Settings = settings;
        settings.set_gtk_application_prefer_dark_theme(true);
        settings.set_gtk_font_name(Some("Inter 10"));
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&stylesheet());
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// The style sheet: the named colours, then `style.css`.
pub fn stylesheet() -> String {
    let mut css = String::new();
    let colours: [(&str, Color); 22] = [
        ("screen", tone::SCREEN),
        ("rule", tone::RULE),
        ("grid", tone::GRID),
        ("axis", tone::AXIS),
        ("label", tone::LABEL),
        ("value", tone::VALUE),
        ("present", tone::PRESENT),
        ("staked", tone::STAKED),
        ("mined", tone::MINED),
        ("record", tone::RECORD),
        ("good", tone::GOOD),
        ("alert", tone::ALERT),
        ("secondary", tone::SECONDARY),
        ("tertiary", tone::TERTIARY),
        ("blue_centre", lamp::BLUE_CENTRE),
        ("blue_edge", lamp::BLUE_EDGE),
        ("amber_centre", lamp::AMBER_CENTRE),
        ("amber_edge", lamp::AMBER_EDGE),
        ("legend", lamp::LEGEND),
        ("halation", lamp::HALATION),
        ("face", lamp::FACE),
        ("system_orange", tone::SYSTEM_ORANGE),
    ];
    for (name, colour) in colours {
        css.push_str(&format!("@define-color {name} {};\n", colour.css()));
    }
    css.push_str(include_str!("style.css"));
    css
}

/// Pango markup for a text in a role: font, size, tracking and case. The text is escaped.
pub fn markup(role: font::Role, text: &str) -> String {
    let text = if role.uppercase { text.to_uppercase() } else { text.to_string() };
    let spacing = (role.tracking * 1024.0).round() as i64;
    let features = if role.tabular { " font_features=\"tnum\"" } else { "" };
    format!(
        "<span font_family=\"{}\" font_size=\"{}\" font_weight=\"{}\" letter_spacing=\"{}\"{}>{}</span>",
        role.family,
        (role.size * 1024.0).round() as i64,
        role.weight,
        spacing,
        features,
        gtk::glib::markup_escape_text(&text)
    )
}

/// A label in a role, in a colour class from `style.css` (`value`, `label`, `secondary`…).
pub fn label(role: font::Role, text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_markup(&markup(role, text));
    label.set_xalign(0.0);
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

/// Sets the text of a label made by [`label`].
pub fn set_label(label: &gtk::Label, role: font::Role, text: &str) {
    label.set_markup(&markup(role, text));
}
