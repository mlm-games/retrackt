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
        let mut names: Vec<String> = docs.iter().map(|doc| doc.name.clone()).collect();
        names.dedup();
        names
    })
}

pub fn persisted(track: &TrackDocument) -> bool {
    with_docs(|docs| docs.iter().any(|doc| doc == track))
}

fn with_docs<R>(f: impl FnOnce(&[TrackDocument]) -> R) -> R {
    SAVED.with(|slot| f(slot.borrow_mut().get_or_insert_with(read_saved)))
}

fn read_saved() -> Vec<TrackDocument> {
    let dir = crate::save::manager().data_dir().join("tracks");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut docs: Vec<TrackDocument> = entries
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("ron"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| TrackDocument::from_ron(&text).ok())
        .collect();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    docs
}
