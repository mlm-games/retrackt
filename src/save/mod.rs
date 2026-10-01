use std::path::{Path, PathBuf};

use game_utils::save::{SaveManager, Versioned};
use game_utils::save_store::{LoadStatus, SaveStore};
use game_utils::storage::{FsStorage, Storage};
use serde::{Deserialize, Serialize};

use retrackt_format::fingerprint::TrackFingerprint;
use retrackt_format::{FORMAT_VERSION, TrackDocument};

pub const SAVE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Settings {
    pub master_volume: f32,
    pub music_volume: f32,
    pub sfx_volume: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            music_volume: 1.0,
            sfx_volume: 1.0,
        }
    }
}

/// Best time per track, keyed by gameplay fingerprint. A `Vec` rather than a
/// map so the RON round-trip never has to use byte arrays as map keys.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct BestTimes {
    entries: Vec<(TrackFingerprint, f32)>,
}

impl BestTimes {
    pub fn get(&self, fp: &TrackFingerprint) -> Option<f32> {
        self.entries.iter().find(|(k, _)| k == fp).map(|(_, t)| *t)
    }

    /// Returns true iff `time` beat the previous best for `fp`.
    pub fn insert(&mut self, fp: &TrackFingerprint, time: f32) -> bool {
        for entry in &mut self.entries {
            if entry.0 == *fp {
                if time < entry.1 {
                    entry.1 = time;
                    return true;
                }
                return false;
            }
        }
        self.entries.push((*fp, time));
        true
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SaveData {
    #[serde(default)]
    pub version: u32,
    pub settings: Settings,
    #[serde(default)]
    pub best_times: BestTimes,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            version: SAVE_VERSION,
            settings: Settings::default(),
            best_times: BestTimes::default(),
        }
    }
}

impl Versioned for SaveData {
    fn version(&self) -> u32 {
        self.version
    }

    fn set_version(&mut self, version: u32) {
        self.version = version;
    }
}

pub fn manager() -> SaveManager {
    SaveManager::new("com", "mlm-games", "retrackt", "save.ron", SAVE_VERSION)
}

fn save_with<S: Storage>(mgr: &SaveManager<S>, data: &SaveData) -> Result<(), String> {
    let mut stamped = data.clone();
    mgr.save_versioned(&mut stamped)
}

fn load_with<S: Storage>(mgr: &SaveManager<S>) -> (SaveData, LoadStatus) {
    mgr.load_with_status()
}

fn boot_with<S: Storage>(mgr: &SaveManager<S>) -> SaveData {
    let (data, status) = load_with(mgr);
    if status == LoadStatus::Ok {
        data
    } else {
        SaveData::default()
    }
}

pub fn boot() -> SaveData {
    boot_with(&manager())
}

pub fn load() -> (SaveData, LoadStatus) {
    load_with(&manager())
}

pub fn save(data: &SaveData) -> Result<(), String> {
    save_with(&manager(), data)
}

pub fn default_data() -> SaveData {
    SaveData::default()
}

fn track_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "track".to_string()
    } else {
        cleaned
    }
}

fn tracks_dir() -> PathBuf {
    manager().data_dir().join("tracks")
}

fn track_store<S: Storage>(dir: &Path, file_name: impl Into<String>, storage: S) -> SaveStore<S> {
    SaveStore::new_with_storage(dir, file_name, storage)
        .with_validator(SaveStore::<S>::is_intact_ron)
}

#[derive(Deserialize)]
struct TrackHeader {
    #[serde(default)]
    format: u32,
}

fn save_track_with<S: Storage>(
    storage: S,
    dir: &Path,
    track: &TrackDocument,
) -> Result<(), String> {
    let mut doc = track.clone();
    doc.normalize_uids();
    let text = doc.to_ron().map_err(|e| e.to_string())?;
    let file = format!("{}.ron", track_stem(&doc.name));
    track_store(dir, file, storage).write(text.as_bytes())
}

fn load_track_with<S: Storage>(storage: S, dir: &Path, name: &str) -> Option<TrackDocument> {
    let file = format!("{}.ron", track_stem(name));
    let bytes = track_store(dir, file, storage)
        .load(&SaveStore::<S>::is_intact_ron, &[])
        .data?;
    let text = std::str::from_utf8(&bytes).ok()?;
    // `TrackDocument::from_ron` re-stamps `format` via `normalize_uids`, so the
    // on-disk version must be read before parsing or a future file is adopted.
    let header = ron::from_str::<TrackHeader>(text).ok()?;
    if header.format != FORMAT_VERSION {
        return None;
    }
    TrackDocument::from_ron(text).ok()
}

pub fn save_track(track: &TrackDocument) -> Result<(), String> {
    save_track_with(FsStorage, &tracks_dir(), track)
}

pub fn load_track(name: &str) -> Option<TrackDocument> {
    load_track_with(FsStorage, &tracks_dir(), name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_utils::MemoryStorage;
    use game_utils::save::SaveManager;

    fn mem_manager() -> SaveManager<MemoryStorage> {
        SaveManager::new_with_storage(
            "com",
            "retrackt-test",
            "save-roundtrip",
            "save.ron",
            SAVE_VERSION,
            MemoryStorage::new(),
        )
    }

    #[test]
    fn save_then_load_roundtrips_through_memory_storage() {
        let mgr = mem_manager();
        let mut data = SaveData {
            settings: Settings {
                master_volume: 0.4,
                sfx_volume: 0.7,
                ..Settings::default()
            },
            ..SaveData::default()
        };
        let fp = [7u8; 16];
        assert!(data.best_times.insert(&fp, 42.5));
        let other_fp = [3u8; 16];
        assert!(data.best_times.insert(&other_fp, 99.0));

        save_with(&mgr, &data).expect("saves");
        let (loaded, status) = load_with(&mgr);

        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(loaded, data);
        assert_eq!(loaded.best_times.get(&other_fp), Some(99.0));
    }

    #[test]
    fn boot_without_a_save_returns_defaults() {
        let mgr = mem_manager();
        assert_eq!(boot_with(&mgr), SaveData::default());
    }

    #[test]
    fn boot_falls_back_to_defaults_when_the_save_is_corrupt() {
        let mgr = mem_manager();
        mgr.storage()
            .write(&mgr.path(), b"((( definitely not ron")
            .unwrap();

        let (data, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Corrupt);
        assert_eq!(data, SaveData::default());
        assert_eq!(boot_with(&mgr), SaveData::default());
    }

    #[test]
    fn best_times_only_replace_strictly_faster_runs() {
        let mut best = BestTimes::default();
        let fp = [1u8; 16];
        let other = [2u8; 16];

        assert_eq!(best.get(&fp), None);
        assert!(best.insert(&fp, 10.0));
        assert!(!best.insert(&fp, 11.0));
        assert!(best.insert(&fp, 9.0));
        assert_eq!(best.get(&fp), Some(9.0));

        assert!(best.insert(&other, 20.0));
        assert_eq!(best.get(&other), Some(20.0));
        assert_eq!(best.get(&fp), Some(9.0));
    }

    #[test]
    fn an_older_save_is_migrated_and_keeps_its_data() {
        let mgr = mem_manager();
        let old =
            b"(version: 0, settings: (master_volume: 0.5, music_volume: 0.25, sfx_volume: 0.75))";
        mgr.storage().write(&mgr.path(), old).unwrap();

        let (data, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(data.version, SAVE_VERSION);
        assert_eq!(data.settings.master_volume, 0.5);
        assert_eq!(data.best_times, BestTimes::default());
        assert_eq!(boot_with(&mgr), data);
    }

    #[test]
    fn a_newer_save_is_refused_and_boots_to_defaults() {
        let mgr = mem_manager();
        let new =
            b"(version: 99, settings: (master_volume: 0.1, music_volume: 0.1, sfx_volume: 0.1))";
        mgr.storage().write(&mgr.path(), new).unwrap();

        let (data, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Corrupt);
        assert_eq!(data.version, 99);
        assert_eq!(boot_with(&mgr), SaveData::default());
    }

    #[test]
    fn save_stamps_the_current_version_whatever_the_caller_holds() {
        let mgr = mem_manager();
        let mut data = SaveData {
            version: 0,
            ..SaveData::default()
        };
        assert!(data.best_times.insert(&[9u8; 16], 12.5));

        save_with(&mgr, &data).unwrap();
        let (loaded, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(loaded.version, SAVE_VERSION);
        assert_eq!(loaded.settings, data.settings);
        assert_eq!(loaded.best_times, data.best_times);
    }

    fn track_dir() -> PathBuf {
        PathBuf::from("/mem/tracks")
    }

    #[test]
    fn track_save_then_load_roundtrips() {
        let storage = MemoryStorage::new();
        let dir = track_dir();
        let mut doc = retrackt_format::demo_track();
        doc.normalize_uids();

        save_track_with(storage.clone(), &dir, &doc).expect("saves");
        let back = load_track_with(storage, &dir, &doc.name).expect("loads");
        assert_eq!(back, doc);
    }

    #[test]
    fn odd_track_names_round_trip_and_cannot_escape_the_track_dir() {
        let storage = MemoryStorage::new();
        let dir = track_dir();
        let mut doc = retrackt_format::demo_track();
        doc.normalize_uids();
        doc.name = "../../etc/№7: ünïcode?".into();

        save_track_with(storage.clone(), &dir, &doc).expect("saves");
        let stem = track_stem(&doc.name);
        assert!(!stem.contains('/') && !stem.contains('\\') && !stem.contains(".."));
        assert!(storage.exists(&dir.join(format!("{stem}.ron"))));

        let back = load_track_with(storage, &dir, &doc.name).expect("loads");
        assert_eq!(back, doc);
    }

    #[test]
    fn missing_corrupt_or_malformed_track_files_return_none() {
        let storage = MemoryStorage::new();
        let dir = track_dir();
        assert_eq!(load_track_with(storage.clone(), &dir, "never saved"), None);

        track_store(&dir, "broken.ron", storage.clone())
            .write(b"((( definitely not ron")
            .unwrap();
        assert_eq!(load_track_with(storage.clone(), &dir, "broken"), None);

        track_store(&dir, "other.ron", storage.clone())
            .write(b"(a: 1)")
            .unwrap();
        assert_eq!(load_track_with(storage, &dir, "other"), None);
    }

    #[test]
    fn track_files_from_another_format_version_are_refused() {
        let storage = MemoryStorage::new();
        let dir = track_dir();
        let track_ron = |format: u32| {
            format!("(format: {format}, name: \"f\", author: \"a\", cell_size: 4.0, pieces: [])")
        };

        track_store(&dir, "current.ron", storage.clone())
            .write(track_ron(FORMAT_VERSION).as_bytes())
            .unwrap();
        assert!(load_track_with(storage.clone(), &dir, "current").is_some());

        track_store(&dir, "future.ron", storage.clone())
            .write(track_ron(FORMAT_VERSION + 1).as_bytes())
            .unwrap();
        assert_eq!(load_track_with(storage.clone(), &dir, "future"), None);

        track_store(&dir, "unversioned.ron", storage.clone())
            .write(b"(name: \"f\", author: \"a\", cell_size: 4.0, pieces: [])")
            .unwrap();
        assert_eq!(load_track_with(storage, &dir, "unversioned"), None);
    }
}
