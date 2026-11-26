//! A row of children that wraps to the next line when it is full, as the Swift app's
//! `FlowLayout`: the plates of a currency and the keys of the options.
//!
//! `gtk::FlowBox` puts its children in columns, so the gaps between them are not equal.
//! This container places each child at its natural width after the one before it.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Flow {
        pub spacing: Cell<i32>,
        pub line_spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Flow {
        const NAME: &'static str = "CarbideCurrenciesFlow";
        type Type = super::Flow;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Flow {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Flow {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.obj().visible_children();
            if children.is_empty() {
                return (0, 0, -1, -1);
            }
            let spacing = self.spacing.get();
            match orientation {
                gtk::Orientation::Horizontal => {
                    let mut minimum = 0;
                    let mut natural = 0;
                    for child in &children {
                        let (min, nat, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                        minimum = minimum.max(min);
                        natural += nat;
                    }
                    natural += spacing * (children.len() as i32 - 1);
                    (minimum, natural.max(minimum), -1, -1)
                }
                _ => {
                    let width = if for_size < 0 { i32::MAX } else { for_size };
                    let lines = self.lines(&children, width);
                    let height = lines.iter().map(|line| line.height).sum::<i32>()
                        + self.line_spacing.get() * (lines.len() as i32 - 1).max(0);
                    (height, height, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let children = self.obj().visible_children();
            let mut y = 0;
            for line in self.lines(&children, width) {
                let mut x = 0;
                for (child, child_width) in &line.items {
                    child.size_allocate(&gtk::Allocation::new(x, y, *child_width, line.height), -1);
                    x += child_width + self.spacing.get();
                }
                y += line.height + self.line_spacing.get();
            }
        }
    }

    pub struct Line {
        pub items: Vec<(gtk::Widget, i32)>,
        pub height: i32,
    }

    impl Flow {
        pub fn lines(&self, children: &[gtk::Widget], width: i32) -> Vec<Line> {
            let spacing = self.spacing.get();
            let mut lines: Vec<Line> = Vec::new();
            let mut x = 0;
            for child in children {
                let (min, nat, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let child_width = nat.min(width).max(min);
                let (_, height, _, _) = child.measure(gtk::Orientation::Vertical, child_width);
                let start = lines.last().is_none_or(|line| !line.items.is_empty() && x + child_width > width);
                if start {
                    lines.push(Line { items: Vec::new(), height: 0 });
                    x = 0;
                }
                let line = lines.last_mut().expect("a line");
                line.items.push((child.clone(), child_width));
                line.height = line.height.max(height);
                x += child_width + spacing;
            }
            lines
        }
    }
}

glib::wrapper! {
    pub struct Flow(ObjectSubclass<imp::Flow>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Flow {
    /// `spacing` between children in a line and between lines.
    pub fn new(spacing: i32) -> Self {
        let flow: Self = glib::Object::new();
        flow.imp().spacing.set(spacing);
        flow.imp().line_spacing.set(spacing);
        flow
    }

    /// The space between lines, when it differs from the space in a line.
    pub fn set_line_spacing(&self, spacing: i32) {
        self.imp().line_spacing.set(spacing);
        self.queue_resize();
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
    }

    fn visible_children(&self) -> Vec<gtk::Widget> {
        let mut children = Vec::new();
        let mut next = self.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            if child.is_visible() {
                children.push(child);
            }
        }
        children
    }
}
