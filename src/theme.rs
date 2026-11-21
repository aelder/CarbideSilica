use crate::display_time::TimeDisplay;
use std::cell::{Cell, RefCell};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::cli::EmojiMode;

pub const SELECTION_MARKER: &str = ">";

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub panel: Color,
    pub search_placeholder: Color,
    search_background: Color,
    search_focus_background: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub warning: Color,
    pub positive: Color,
    pub negative: Color,
    pub link: Color,
    pub json_number: Color,
    pub focus: Color,
    emoji: EmojiMode,
    pub time_display: Cell<TimeDisplay>,
    pub heights: RefCell<crate::height_display::HeightDisplay>,
    pub(crate) copy_values: RefCell<crate::copy_values::Values>,
}

impl Theme {
    pub fn desert(emoji: EmojiMode) -> Self {
        Self {
            background: Color::Rgb(51, 51, 51),
            panel: Color::Rgb(28, 28, 28),
            search_placeholder: Color::Rgb(127, 142, 158),
            search_background: Color::Rgb(24, 36, 49),
            search_focus_background: Color::Rgb(42, 58, 74),
            text: Color::Rgb(245, 245, 220),
            muted: Color::Rgb(188, 188, 188),
            accent: Color::Rgb(255, 215, 0),
            warning: Color::Rgb(255, 160, 80),
            positive: Color::Rgb(135, 215, 135),
            negative: Color::Rgb(232, 128, 128),
            link: Color::Rgb(135, 175, 255),
            json_number: Color::Rgb(190, 175, 205),
            focus: Color::Rgb(255, 255, 175),
            emoji,
            time_display: Cell::new(TimeDisplay::default()),
            heights: RefCell::default(),
            copy_values: RefCell::default(),
        }
    }

    pub fn format_time(&self, timestamp: u64) -> String {
        self.time_display.get().format(timestamp)
    }

    pub fn format_height(&self, height: u64) -> String {
        self.format_height_at(height, None)
    }

    pub fn format_height_at(&self, height: u64, time: Option<u64>) -> String {
        self.heights
            .borrow_mut()
            .format(height, time, self.time_display.get())
    }

    pub fn format_height_on(&self, system: &str, height: u64) -> String {
        let mut display = self.heights.borrow_mut();
        if system.is_empty() || display.scope.starts_with(&format!("{system}:")) {
            return display.format(height, None, self.time_display.get());
        }
        let dates = display.dates;
        let mut foreign = display
            .foreign
            .entry(system.into())
            .or_default()
            .borrow_mut();
        foreign.dates = dates;
        foreign.format(height, None, self.time_display.get())
    }

    /// Translate explicit height markers in our generated protocol summaries.
    /// Never apply this to JSON, arbitrary RPC messages or user input.
    pub fn height_text(&self, text: &str, system: &str) -> String {
        if !self.heights.borrow().dates {
            return text.into();
        }
        let source = text
            .split("source ")
            .nth(1)
            .and_then(|s| s.split_once(" at height "))
            .map(|(id, _)| id)
            .filter(|id| crate::world::SystemId::try_from((*id).to_string()).is_ok());
        let system = source.unwrap_or(system);
        let mut output = String::new();
        let mut remaining = text;
        while let Some((index, marker)) = ["height ", "heights ", "window "]
            .into_iter()
            .filter_map(|marker| remaining.find(marker).map(|i| (i, marker)))
            .min_by_key(|x| x.0)
        {
            let after = &remaining[index + marker.len()..];
            let length = after.bytes().take_while(u8::is_ascii_digit).count();
            output.push_str(&remaining[..index + marker.len()]);
            if let Ok(h) = after[..length].parse::<u64>() {
                output.push_str(&self.format_height_on(system, h));
                remaining = &after[length..];
                let separator = if remaining.starts_with("..") {
                    ".."
                } else if remaining.starts_with('-') {
                    "-"
                } else {
                    ""
                };
                if !separator.is_empty() {
                    let tail = &remaining[separator.len()..];
                    let n = tail.bytes().take_while(u8::is_ascii_digit).count();
                    if let Ok(h) = tail[..n].parse::<u64>() {
                        output.push_str(" to ");
                        output.push_str(&self.format_height_on(system, h));
                        remaining = &tail[n..];
                    }
                }
            } else {
                remaining = after;
            }
        }
        output.push_str(remaining);
        output
    }

    pub fn status_badge(&self) -> Style {
        Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(0, 100, 110))
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_badge_on(&self, color: Color) -> Style {
        self.status_badge().fg(self.panel).bg(color)
    }

    pub fn base(&self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    pub fn search_field(&self, active: bool) -> Style {
        let background = if active {
            self.search_focus_background
        } else {
            self.search_background
        };
        self.base().bg(background)
    }

    pub fn header(&self) -> Style {
        self.selection_cursor()
    }

    pub fn selection_cursor(&self) -> Style {
        Style::default()
            .fg(self.background)
            .bg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn selection_marker(&self, selected: bool) -> Span<'static> {
        if selected {
            Span::styled(SELECTION_MARKER, self.selection_cursor())
        } else {
            // Keep the cursor column when a wrapping paragraph trims leading spaces.
            Span::raw("\u{00a0}")
        }
    }

    pub fn muted(&self) -> Style {
        Style::default().fg(self.muted).bg(self.background)
    }

    pub fn focused(&self) -> Style {
        Style::default()
            .fg(self.focus)
            .bg(self.panel)
            .add_modifier(Modifier::BOLD)
    }

    pub(crate) fn value(
        &self,
        label: &str,
        value: impl Into<String>,
        style: Style,
    ) -> Span<'static> {
        let value = value.into();
        if !self.copy_values.borrow().is_recording() {
            return Span::styled(value, style);
        }
        self.value_as(label, value.clone(), value, style)
    }

    pub(crate) fn value_as(
        &self,
        label: &str,
        display: impl Into<String>,
        value: impl Into<String>,
        style: Style,
    ) -> Span<'static> {
        let span = Span::styled(display.into(), style);
        self.copy_values.borrow_mut().span(
            span,
            crate::copy_values::Target::Text {
                label: label.to_owned(),
                value: value.into(),
            },
        )
    }

    pub fn tile_gutter(&self) -> Style {
        Style::default().fg(self.text).bg(self.panel)
    }

    pub fn tile_body(&self, selected: bool) -> Style {
        if selected {
            self.focused()
        } else {
            self.base()
        }
    }

    pub fn warning(&self) -> Style {
        Style::default().fg(self.warning).bg(self.background)
    }

    pub fn positive(&self) -> Style {
        Style::default().fg(self.positive).bg(self.background)
    }

    pub fn negative(&self) -> Style {
        Style::default().fg(self.negative).bg(self.background)
    }

    pub fn link(&self) -> Style {
        Style::default().fg(self.link).bg(self.background)
    }

    pub fn glyph(&self, emoji: &'static str, fallback: &'static str) -> &'static str {
        match self.emoji {
            EmojiMode::Off => fallback,
            EmojiMode::Auto | EmojiMode::On => emoji,
        }
    }
}

#[cfg(test)]
mod height_tests {
    use super::*;
    #[test]
    fn dates_are_chain_scoped_and_block_counts_and_raw_values_are_untouched() {
        let theme = Theme::desert(EmojiMode::Off);
        {
            let mut display = theme.heights.borrow_mut();
            display.dates = true;
            display.scope = "local:revision".into();
            display.timestamps.insert(12, Some(1000));
            display
                .foreign
                .entry("remote".into())
                .or_default()
                .borrow_mut()
                .timestamps
                .insert(12, Some(2000));
        }
        assert_eq!(theme.format_height_on("local", 12), "1970-01-01T00:16:40Z");
        assert_eq!(theme.format_height_on("remote", 12), "1970-01-01T00:33:20Z");
        assert!(
            theme
                .height_text("Unlock height 12; delay 100 blocks", "local")
                .contains("height 1970-01-01T00:16:40Z; delay 100 blocks")
        );
        assert_eq!(theme.height_text("100 blocks", "local"), "100 blocks");
        theme.heights.borrow_mut().dates = false;
        assert_eq!(theme.height_text("height 12", "local"), "height 12");
    }
}
