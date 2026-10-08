use std::cell::RefCell;

use retrackt_format::TrackDocument;

thread_local! {
    static SAVED: RefCell<Option<Vec<TrackDocument>>> = const { RefCell::new(None) };
}

pub fn invalidate() {
    SAVED.with(|slot| *slot.borrow_mut() = None);
}

pub fn saved_tracks() -> Vec<String> {
    with_docs(|docs| {
        // Two files can carry the same track name, and that is not the same as
        // two adjacent names: `dedup` would keep the second copy whenever the
        // list were not sorted. Kept in stored order, first occurrence wins.
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
    with_docs(|docs| docs.iter().any(|doc| doc == track))
}

fn with_docs<R>(f: impl FnOnce(&[TrackDocument]) -> R) -> R {
    SAVED.with(|slot| f(slot.borrow_mut().get_or_insert_with(crate::save::stored_tracks)))
}
