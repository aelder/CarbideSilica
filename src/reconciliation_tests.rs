use super::*;
use std::collections::HashMap;
fn branch(prefix: &str, start: u64, end: u64, parent: &str) -> Vec<Header> {
    let mut parent = parent.to_owned();
    (start..=end)
        .map(|height| {
            let h = Header {
                hash: format!("{prefix}{height}"),
                height,
                parent: (height > 0).then_some(parent.clone()),
                time: height * 60,
            };
            parent = h.hash.clone();
            h
        })
        .collect()
}
async fn walk(old: &State, tip: Header, headers: &[Header]) -> Result<Change> {
    let headers: HashMap<_, _> = headers
        .iter()
        .map(|h| (h.hash.clone(), h.clone()))
        .collect();
    plan(old, tip, |hash| {
        let result = headers.get(&hash).cloned().context("unavailable header");
        async move { result }
    })
    .await
}
#[tokio::test]
async fn bootstrap_is_bounded_but_deep_forks_reach_the_ancestor() {
    let main = branch("a", 0, 250, "");
    let initial = walk(&State::default(), main[250].clone(), &main)
        .await
        .unwrap();
    assert_eq!(initial.attached.len(), 100);
    assert_eq!(initial.state.canonical.first_key_value().unwrap().0, &151);
    let fork = branch("b", 20, 260, "a19");
    let headers: Vec<_> = main.iter().chain(fork.iter()).cloned().collect();
    let changed = walk(&initial.state, fork.last().unwrap().clone(), &headers)
        .await
        .unwrap();
    assert_eq!(changed.ancestor.unwrap().height, 19);
    assert_eq!(changed.detached.len(), 231);
    assert_eq!(changed.attached.len(), 241);
    assert_eq!(changed.state.tip().unwrap().hash, "b260");
    assert_eq!(initial.state.tip().unwrap().hash, "a250");
}
#[tokio::test]
async fn skipped_advance_same_height_replacement_and_rollback() {
    let main = branch("a", 0, 10, "");
    let initial = walk(&State::default(), main[5].clone(), &main)
        .await
        .unwrap();
    let advance = walk(&initial.state, main[10].clone(), &main).await.unwrap();
    assert!(advance.detached.is_empty());
    assert_eq!(advance.attached.len(), 5);
    let fork = branch("b", 9, 10, "a8");
    let headers: Vec<_> = main.iter().chain(fork.iter()).cloned().collect();
    let changed = walk(&advance.state, fork[1].clone(), &headers)
        .await
        .unwrap();
    assert_eq!(changed.detached.len(), 2);
    assert_eq!(changed.ancestor.as_ref().unwrap().height, 8);
    let rollback = walk(&changed.state, main[7].clone(), &headers)
        .await
        .unwrap();
    assert_eq!(rollback.detached.len(), 3);
    assert!(rollback.attached.is_empty());
    let same = walk(&rollback.state, main[7].clone(), &[]).await.unwrap();
    assert_eq!(same.state.revision, rollback.state.revision);
}
#[tokio::test]
async fn missing_or_invalid_headers_never_produce_a_partial_change() {
    let main = branch("a", 0, 4, "");
    let initial = walk(&State::default(), main[2].clone(), &main)
        .await
        .unwrap();
    assert!(walk(&initial.state, main[4].clone(), &[]).await.is_err());
    assert!(
        plan(&initial.state, main[4].clone(), |_| async {
            Ok(Header {
                hash: "wrong".into(),
                height: 3,
                parent: Some("a2".into()),
                time: 0,
            })
        })
        .await
        .is_err()
    );
    assert_eq!(initial.state.tip().unwrap().hash, "a2");
}
