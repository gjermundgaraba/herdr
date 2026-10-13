//! Persisted metadata tokens.

use super::*;

#[test]
fn only_persisted_tokens_are_saved_and_a_plain_patch_unpersists() {
    let now = Instant::now();
    let mut tokens = MetadataTokens::default();
    tokens.patch_persisted(patch(&[("group", Some("work")), ("priority", Some("1"))]));
    tokens.patch(patch(&[("git", Some("dirty"))]), None, now);
    tokens.patch(patch(&[("priority", Some("2"))]), None, now);

    let saved = tokens.persisted();
    assert_eq!(saved, HashMap::from([("group".into(), "work".into())]));
    assert_eq!(MetadataTokens::restored(saved).persisted().len(), 1);
}
