//! A widget that draws with a closure: the Linux form of SwiftUI's `Canvas`.
//!
//! The closure gets the snapshot and the size. It runs when GTK asks for a new picture of
//! the widget: after `queue_draw`, a new size, or a change of the widget's state. Keep a
//! canvas small when it redraws often. A canvas does not take clicks unless the page gives
//! it controllers.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

type Draw = Box<dyn Fn(&Canvas, &gtk::Snapshot, f32, f32)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Canvas {
        pub draw: RefCell<Option<Draw>>,
        pub natural_width: Cell<i32>,
        pub natural_height: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "CarbideCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            if let Some(draw) = self.draw.borrow().as_ref() {
                draw(&widget, snapshot, w, h);
            }
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Horizontal => self.natural_width.get(),
                _ => self.natural_height.get(),
            };
            (size, size, -1, -1)
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Canvas {
    /// A canvas of at least `width` by `height` (0 for no minimum).
    pub fn new(width: i32, height: i32, draw: impl Fn(&Canvas, &gtk::Snapshot, f32, f32) + 'static) -> Self {
        let canvas: Self = glib::Object::new();
        canvas.imp().natural_width.set(width);
        canvas.imp().natural_height.set(height);
        *canvas.imp().draw.borrow_mut() = Some(Box::new(draw));
        canvas
    }

    /// Replaces the drawing.
    pub fn set_draw(&self, draw: impl Fn(&Canvas, &gtk::Snapshot, f32, f32) + 'static) {
        *self.imp().draw.borrow_mut() = Some(Box::new(draw));
        self.queue_draw();
    }

    pub fn set_size(&self, width: i32, height: i32) {
        self.imp().natural_width.set(width);
        self.imp().natural_height.set(height);
        self.queue_resize();
    }
}
