use std::cell::RefCell;

use retrackt_format::TrackDocument;

thread_local! {
    static SAVED: RefCell<Option<Vec<TrackDocument>>> = const { RefCell::new(None) };
}

pub fn invalidate() {
    SAVED.with(|slot| *slot.borrow_mut() = None);
}

/// Every stored track, read once and cached.
///
/// The read goes through the platform storage backend, which on wasm is the only
/// route to what the game itself wrote; the raw path silently listed nothing.
///
/// Returned as a borrow rather than a clone: the title screen and the editor
/// both ask every frame, and cloning every document on every frame is a cost
/// that grows with the player's own library. The borrow cannot outlive the
/// closure, which is enough because every caller reads and drops it.
pub fn with_saved<R>(f: impl FnOnce(&[TrackDocument]) -> R) -> R {
    SAVED.with(|slot| {
        f(slot
            .borrow_mut()
            .get_or_insert_with(crate::save::stored_tracks))
    })
}

/// Names only, deduplicated in stored order.
///
/// Two files can carry the same track name, and that is not the same as two
/// adjacent names: `dedup` would keep the second copy whenever the list were not
/// sorted. Kept in stored order, first occurrence wins.
fn saved_names() -> Vec<String> {
    with_saved(|docs| {
        let mut names: Vec<String> = Vec::new();
        for doc in docs {
            if !names.contains(&doc.name) {
                names.push(doc.name.clone());
            }
        }
        names
    })
}

pub fn persisted(track: &TrackDocument) -> bool {
    with_saved(|docs| docs.iter().any(|doc| doc == track))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str, pieces: usize) -> TrackDocument {
        let mut d = TrackDocument::empty();
        d.name = name.into();
        d.pieces =
            vec![
                retrackt_format::PieceInstance::new(retrackt_format::PieceId::Straight, [0, 0, 0],);
                pieces
            ];
        d
    }

    #[test]
    fn duplicate_names_collapse_and_keep_stored_order() {
        SAVED.with(|slot| {
            *slot.borrow_mut() = Some(vec![doc("b", 1), doc("a", 1), doc("b", 2)]);
        });
        // First occurrence wins, so the name list is in stored order and has no
        // repeats — not sorted, and not the *last* copy of a repeated name.
        assert_eq!(saved_names(), vec!["b".to_string(), "a".to_string()]);
        invalidate();
    }

    #[test]
    fn persisted_compares_the_whole_document() {
        let stored = doc("t", 2);
        SAVED.with(|slot| *slot.borrow_mut() = Some(vec![stored.clone()]));
        assert!(persisted(&stored));

        let mut edited = stored.clone();
        edited.pieces[0].yaw = 1;
        assert!(
            !persisted(&edited),
            "one edited field must stop it reading as saved"
        );
        invalidate();
    }

    #[test]
    fn invalidating_forces_a_reread() {
        SAVED.with(|slot| *slot.borrow_mut() = Some(vec![doc("first", 1)]));
        assert_eq!(saved_names(), vec!["first".to_string()]);
        // Nothing replaces the cached copy on its own, so asking twice is stable.
        assert_eq!(saved_names(), vec!["first".to_string()]);

        invalidate();
        // Cleared rather than flagged stale: there is no other way for the next
        // read to tell a cleared slot from a cached one, and a flag the cache
        // could forget to clear would keep serving a library the player has left.
        assert!(
            SAVED.with(|slot| slot.borrow().is_none()),
            "invalidate must empty the cache, not mark it"
        );
        invalidate();
    }
}
