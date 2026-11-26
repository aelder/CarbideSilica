//! Parts that more than one page uses. Each file has one owner: the engineer of the page
//! named in its first line finishes it, and the other pages only call its public functions,
//! whose signatures stay as they are. See `linux/PORT.md`, "Shared parts".

pub mod address_activity;
pub mod branches;
pub mod confidence;
pub mod currency_stores;
pub mod estimator;
pub mod identity_support;
pub mod inclusion;
pub mod json_tree;
pub mod statement_words;
