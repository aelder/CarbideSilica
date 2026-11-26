//! Peer data beside `NodeStatus`: the ban list, the one-time traffic totals and peer
//! locations from Carbide's location database (`PeerFeed` in `PeerMap.swift`). One feed
//! lives for the session, as `PeerFeed.shared`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::bridge::{self, api};
use crate::model::AppModel;

#[derive(Default)]
pub struct PeerFeed {
    pub banned: RefCell<Vec<api::BannedPeer>>,
    pub banned_error: RefCell<Option<String>>,
    pub totals: RefCell<Option<api::NetworkTotals>>,
    pub totals_error: RefCell<Option<String>>,
    pub locations: RefCell<Option<api::PeerLocations>>,
    /// The place of this node, when the peers report a public address for it and the
    /// location data has that address.
    pub home: RefCell<Option<api::PeerLocation>>,
    pub installing: Cell<bool>,
    pub install_message: RefCell<Option<String>>,
    /// The lowest and highest response time of each peer since the page opened, in
    /// milliseconds, by peer ID.
    pub response_range: RefCell<HashMap<i64, (f64, f64)>>,
}

thread_local! {
    static SHARED: Rc<PeerFeed> = Rc::default();
}

/// The feed of the session.
pub fn shared() -> Rc<PeerFeed> {
    SHARED.with(|feed| feed.clone())
}

/// Records the response time of each peer. The record of a peer that left is removed.
pub fn note_ranges(known: &HashMap<i64, (f64, f64)>, peers: &[api::PeerRow]) -> HashMap<i64, (f64, f64)> {
    let mut kept = HashMap::new();
    for peer in peers {
        let previous = known.get(&peer.id).copied();
        match peer.ping_ms {
            None => {
                if let Some(range) = previous {
                    kept.insert(peer.id, range);
                }
            }
            Some(ms) => {
                let low = previous.map(|r| r.0).unwrap_or(ms).min(ms);
                let high = previous.map(|r| r.1).unwrap_or(ms).max(ms);
                kept.insert(peer.id, (low, high));
            }
        }
    }
    kept
}

/// The address that most peers report for this node (`addrlocal`).
pub fn own_address(peers: &[api::PeerRow]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for peer in peers {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&peer.json) else { continue };
        let Some(seen) = value.get("addrlocal").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else { continue };
        *counts.entry(seen.to_string()).or_default() += 1;
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    // The most frequent; among equals, one fixed choice.
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list.into_iter().next().map(|(address, _)| address)
}

impl PeerFeed {
    pub fn note(&self, peers: &[api::PeerRow]) {
        let kept = note_ranges(&self.response_range.borrow(), peers);
        *self.response_range.borrow_mut() = kept;
    }

    pub async fn load_bans(&self, model: &Rc<AppModel>) {
        let Some(explorer) = model.explorer() else { return };
        match bridge::run(async move { explorer.ban_list().await }).await {
            Ok(list) => {
                *self.banned.borrow_mut() = list;
                *self.banned_error.borrow_mut() = None;
            }
            Err(error) => {
                self.banned.borrow_mut().clear();
                *self.banned_error.borrow_mut() = Some(bridge::describe(&error));
            }
        }
    }

    /// The terminal reads the totals once when Peers opens. So does the app.
    pub async fn load_totals(&self, model: &Rc<AppModel>) {
        let Some(explorer) = model.explorer() else { return };
        match bridge::run(async move { explorer.network_totals().await }).await {
            Ok(totals) => {
                *self.totals.borrow_mut() = Some(totals);
                *self.totals_error.borrow_mut() = None;
            }
            Err(error) => {
                *self.totals.borrow_mut() = None;
                *self.totals_error.borrow_mut() = Some(bridge::describe(&error));
            }
        }
    }

    pub async fn load_locations(&self, model: &Rc<AppModel>, addresses: Vec<String>) {
        let Some(explorer) = model.explorer() else { return };
        let ex = explorer.clone();
        let found = bridge::run(async move { ex.peer_locations(addresses).await }).await;
        *self.locations.borrow_mut() = Some(found);
        // Each peer reports the address that it sees for this node. The place of that
        // address comes from the same local data: the app makes no request for it.
        let peers = model.state().node.as_ref().map(|n| n.peers.clone()).unwrap_or_default();
        let Some(own) = own_address(&peers) else {
            *self.home.borrow_mut() = None;
            return;
        };
        let found = bridge::run(async move { explorer.peer_locations(vec![own]).await }).await.locations.into_iter().next();
        *self.home.borrow_mut() =
            found.filter(|l| l.place == api::PeerPlace::Located && l.latitude.is_some() && l.longitude.is_some());
    }

    /// Downloads the location database. Only the Download key in the confirmation calls it.
    /// The feed is marked as installing at once; the returned work does the download.
    pub fn begin_install(self: &Rc<Self>, model: &Rc<AppModel>, addresses: Vec<String>) -> Option<impl std::future::Future<Output = ()> + 'static> {
        let explorer = model.explorer()?;
        if self.installing.get() {
            return None;
        }
        self.installing.set(true);
        *self.install_message.borrow_mut() = None;
        let (feed, model) = (self.clone(), model.clone());
        Some(async move {
            let result = bridge::run(async move { explorer.install_location_database().await }).await;
            *feed.install_message.borrow_mut() = Some(match result {
                Ok(release) => format!("The app installed the location data for {release}."),
                Err(error) => bridge::describe(&error),
            });
            feed.installing.set(false);
            feed.load_locations(&model, addresses).await;
        })
    }

    pub fn location(&self, address: &str) -> Option<api::PeerLocation> {
        self.locations.borrow().as_ref()?.locations.iter().find(|l| l.address == address).cloned()
    }
}

/// The place of an address in the location data; unresolved when it is not listed.
pub fn place_of(locations: &api::PeerLocations, address: &str) -> api::PeerPlace {
    locations.locations.iter().find(|l| l.address == address).map(|l| l.place).unwrap_or(api::PeerPlace::Unresolved)
}

/// "City, Country (CC)", as the terminal's peer overview.
pub fn place_name(location: &api::PeerLocation) -> String {
    let place = [location.city.clone(), location.country.clone()].into_iter().flatten().collect::<Vec<_>>().join(", ");
    match (place.is_empty(), &location.country_code) {
        (true, Some(code)) => code.clone(),
        (false, Some(code)) => format!("{place} ({code})"),
        (false, None) => place,
        (true, None) => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: i64, ping: Option<f64>, json: &str) -> api::PeerRow {
        api::PeerRow {
            id,
            address: format!("192.0.2.{id}"),
            inbound: false,
            ping_ms: ping,
            synced_blocks: None,
            starting_height: None,
            bytes_sent: 0,
            bytes_received: 0,
            version: None,
            connected_since: None,
            json: json.into(),
        }
    }

    #[test]
    fn ranges_grow_and_forget_peers_that_left() {
        let first = note_ranges(&HashMap::new(), &[peer(1, Some(50.0), "{}"), peer(2, None, "{}")]);
        assert_eq!(first.get(&1), Some(&(50.0, 50.0)));
        assert!(!first.contains_key(&2));
        let second = note_ranges(&first, &[peer(1, Some(30.0), "{}")]);
        let third = note_ranges(&second, &[peer(1, Some(80.0), "{}"), peer(3, Some(5.0), "{}")]);
        assert_eq!(third.get(&1), Some(&(30.0, 80.0)));
        let fourth = note_ranges(&third, &[peer(3, None, "{}")]);
        assert_eq!(fourth.len(), 1);
        assert_eq!(fourth.get(&3), Some(&(5.0, 5.0)));
    }

    #[test]
    fn own_address_is_the_one_most_peers_see() {
        let peers = [
            peer(1, None, r#"{"addrlocal":"198.51.100.7:27485"}"#),
            peer(2, None, r#"{"addrlocal":"198.51.100.7:27485"}"#),
            peer(3, None, r#"{"addrlocal":"203.0.113.1:27485"}"#),
            peer(4, None, r#"{"addrlocal":""}"#),
        ];
        assert_eq!(own_address(&peers).as_deref(), Some("198.51.100.7:27485"));
        assert_eq!(own_address(&[peer(1, None, "{}")]), None);
    }
}
