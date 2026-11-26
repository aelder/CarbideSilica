//! A container that gives its child at most a maximum width, at its leading edge: the
//! `frame(maxWidth: 1100, alignment: .leading)` of the Swift pages.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Clamp {
        pub maximum: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "CarbideClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Clamp {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.obj().first_child() else { return (0, 0, -1, -1) };
            let maximum = self.maximum.get();
            match orientation {
                gtk::Orientation::Horizontal => {
                    let (min, nat, _, _) = child.measure(orientation, for_size);
                    (min, nat.min(maximum.max(min)), -1, -1)
                }
                _ => {
                    let width = if for_size < 0 { for_size } else { for_size.min(maximum) };
                    child.measure(orientation, width)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                let (min, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let width = width.min(self.maximum.get()).max(min);
                child.size_allocate(&gtk::Allocation::new(0, 0, width, height), baseline);
            }
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }
    }
}

glib::wrapper! {
    pub struct Clamp(ObjectSubclass<imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    pub fn new(child: &impl IsA<gtk::Widget>, maximum: i32) -> Self {
        let clamp: Self = glib::Object::new();
        clamp.imp().maximum.set(maximum);
        child.set_parent(&clamp);
        clamp
    }
}
