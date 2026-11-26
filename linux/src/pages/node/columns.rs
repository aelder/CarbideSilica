//! One column layout for the peer list's heading and rows (`PeerColumns`). The flexible
//! columns share the page width, so each column fits without a horizontal scroll: the peer
//! at least 220 wide, the response time from 180 to 300, the data 170 and the time of the
//! connection 70, 16 apart, all at the top.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

const SPACING: i32 = 16;
const PEER_MIN: i32 = 220;
const RESPONSE_MIN: i32 = 180;
const RESPONSE_MAX: i32 = 300;
const DATA: i32 = 170;
const CONNECTED: i32 = 70;

/// The widths of the four columns in a row of `width`.
pub fn widths(width: i32) -> [i32; 4] {
    let rest = (width - DATA - CONNECTED - 3 * SPACING).max(PEER_MIN + RESPONSE_MIN);
    let response = (rest / 2).clamp(RESPONSE_MIN, RESPONSE_MAX).min(rest - PEER_MIN).max(RESPONSE_MIN);
    [rest - response, response, DATA, CONNECTED]
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Columns;

    #[glib::object_subclass]
    impl ObjectSubclass for Columns {
        const NAME: &'static str = "CarbidePeerColumns";
        type Type = super::Columns;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Columns {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Columns {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let obj = self.obj();
            let children: Vec<gtk::Widget> = std::iter::successors(obj.first_child(), |c| c.next_sibling()).collect();
            match orientation {
                gtk::Orientation::Horizontal => {
                    let least = PEER_MIN + RESPONSE_MIN + DATA + CONNECTED + 3 * SPACING;
                    (least, least, -1, -1)
                }
                _ => {
                    let width = if for_size < 0 { PEER_MIN + RESPONSE_MIN + DATA + CONNECTED + 3 * SPACING } else { for_size };
                    let widths = widths(width);
                    let mut height = (0, 0);
                    for (child, w) in children.iter().zip(widths) {
                        if !child.is_visible() {
                            continue;
                        }
                        let (least, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                        let (min, nat, _, _) = child.measure(gtk::Orientation::Vertical, w.max(least));
                        height = (height.0.max(min), height.1.max(nat));
                    }
                    (height.0, height.1, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let obj = self.obj();
            let mut x = 0;
            let children: Vec<gtk::Widget> = std::iter::successors(obj.first_child(), |c| c.next_sibling()).collect();
            let count = children.len();
            for (index, (child, w)) in children.iter().zip(widths(width)).enumerate() {
                // A text wider than its column keeps its width; the last column keeps its
                // right edge, as a column with trailing alignment does.
                let (least, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                let given = w.max(least);
                let left = if index + 1 == count { x + w - given } else { x };
                let (_, nat, _, _) = child.measure(gtk::Orientation::Vertical, given);
                child.size_allocate(&gtk::Allocation::new(left, 0, given, nat), -1);
                x += w + SPACING;
            }
        }
    }
}

glib::wrapper! {
    pub struct Columns(ObjectSubclass<imp::Columns>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Columns {
    pub fn new(peer: &impl IsA<gtk::Widget>, response: &impl IsA<gtk::Widget>, data: &impl IsA<gtk::Widget>, connected: &impl IsA<gtk::Widget>) -> Self {
        let columns: Self = glib::Object::new();
        for child in [peer.as_ref(), response.as_ref(), data.as_ref(), connected.as_ref()] {
            child.set_parent(&columns);
        }
        columns.set_hexpand(true);
        columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_share_the_width() {
        assert_eq!(widths(1100), [512, 300, 170, 70]);
        assert_eq!(widths(800), [256, 256, 170, 70]);
        // Narrower than the least: each flexible column keeps its least width.
        assert_eq!(widths(500), [220, 180, 170, 70]);
    }
}
