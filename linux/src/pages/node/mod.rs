//! The Node and Peers page. Port of NodeView.swift, PeerControls.swift, PeerMap.swift, PeerOutlineMap.swift and NodeDiagrams.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Not ported yet. This module owns its folder: add files here and declare them below.

use std::rc::Rc;

use crate::model::AppModel;
use crate::page::{self, Page};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    page::not_ported(model, "Node and Peers", "NodeView.swift, PeerControls.swift, PeerMap.swift, PeerOutlineMap.swift and NodeDiagrams.swift")
}
