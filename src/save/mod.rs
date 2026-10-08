use std::path::{Path, PathBuf};

use game_utils::save::{SaveManager, Versioned};
use game_utils::save_store::{LoadStatus, SaveStore};
use game_utils::storage::{FsStorage, Storage};
use serde::{Deserialize, Serialize};

use retrackt_format::fingerprint::TrackFingerprint;
use retrackt_format::{FORMAT_VERSION, TrackDocument};

pub const SAVE_VERSION: u32 = 2;

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

/// One stored record: the fastest run on a track under one vehicle tuning.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct RecordEntry {
    pub track: TrackFingerprint,
    pub physics: TrackFingerprint,
    /// Race time in simulated ticks. The race clock is integral, so this is
    /// exactly what was run.
    pub ticks: u32,
}

/// Records per track *and* vehicle tuning. A `Vec` rather than a map so the RON
/// round-trip never has to use byte arrays as map keys.
///
/// Physics is part of the key because a time set under one tuning is not
/// comparable with one set under another: retune the car and the old number
/// stops being a target the player can read anything into.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Records {
    entries: Vec<RecordEntry>,
}

impl Records {
    pub fn get(&self, track: &TrackFingerprint, physics: &TrackFingerprint) -> Option<u32> {
        self.entries
            .iter()
            .find(|e| e.track == *track && e.physics == *physics)
            .map(|e| e.ticks)
    }

    /// Returns true iff `ticks` beat the previous best for this track and tuning.
    pub fn insert(
        &mut self,
        track: &TrackFingerprint,
        physics: &TrackFingerprint,
        ticks: u32,
    ) -> bool {
        for entry in &mut self.entries {
            if entry.track == *track && entry.physics == *physics {
                if ticks < entry.ticks {
                    entry.ticks = ticks;
                    return true;
                }
                return false;
            }
        }
        self.entries.push(RecordEntry {
            track: *track,
            physics: *physics,
            ticks,
        });
        true
    }

    /// Drop records set under other tuning. They cannot be read as records
    /// anymore, and leaving them in the file grows it on every retune.
    pub fn prune(&mut self, physics: &TrackFingerprint) {
        self.entries.retain(|e| e.physics == *physics);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Records as written by save format 1, keyed on the track alone. Read only so a
/// version 1 file still deserialises; [`Versioned::migrate`] discards them,
/// because a time set under an unknown physics revision is not comparable with
/// one set under the current build.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct LegacyRecords {
    entries: Vec<(TrackFingerprint, f32)>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SaveData {
    #[serde(default)]
    pub version: u32,
    pub settings: Settings,
    #[serde(default)]
    pub best_times: LegacyRecords,
    #[serde(default)]
    pub records: Records,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            version: SAVE_VERSION,
            settings: Settings::default(),
            best_times: LegacyRecords::default(),
            records: Records::default(),
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

    fn migrate(&mut self, from: u32, _to: u32) {
        if from < 2 {
            self.best_times = LegacyRecords::default();
        }
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

/// Parse a stored track, refusing anything written under a different format
/// version. `TrackDocument::from_ron` re-stamps `format` via `normalize_uids`,
/// so the on-disk version must be read before parsing or a future file is adopted.
fn parse_stored(text: &str) -> Option<TrackDocument> {
    let header = ron::from_str::<TrackHeader>(text).ok()?;
    if header.format != FORMAT_VERSION {
        return None;
    }
    TrackDocument::from_ron(text).ok()
}

fn load_track_with<S: Storage>(storage: S, dir: &Path, name: &str) -> Option<TrackDocument> {
    let file = format!("{}.ron", track_stem(name));
    let bytes = track_store(dir, file, storage)
        .load(&SaveStore::<S>::is_intact_ron, &[])
        .data?;
    parse_stored(std::str::from_utf8(&bytes).ok()?)
}

pub fn save_track(track: &TrackDocument) -> Result<(), String> {
    save_track_with(FsStorage, &tracks_dir(), track)
}

pub fn load_track(name: &str) -> Option<TrackDocument> {
    load_track_with(FsStorage, &tracks_dir(), name)
}

/// Every stored track, read through the platform storage backend rather than
/// `std::fs`: on wasm that is the only route to what the game itself wrote, and
/// the raw path silently listed nothing.
pub fn stored_tracks() -> Vec<TrackDocument> {
    let storage = FsStorage;
    let Ok(paths) = storage.read_dir(&tracks_dir()) else {
        return Vec::new();
    };
    let mut docs: Vec<TrackDocument> = paths
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ron"))
        .filter_map(|p| {
            let bytes = storage.read(&p).ok().flatten()?;
            std::str::from_utf8(&bytes).ok().and_then(parse_stored)
        })
        .collect();
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    docs
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
        let tune = [8u8; 16];
        assert!(data.records.insert(&fp, &tune, 5_100));
        let other_fp = [3u8; 16];
        assert!(data.records.insert(&other_fp, &tune, 11_900));

        save_with(&mgr, &data).expect("saves");
        let (loaded, status) = load_with(&mgr);

        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(loaded, data);
        assert_eq!(loaded.records.get(&other_fp, &tune), Some(11_900));
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
    fn records_only_replace_strictly_faster_runs() {
        let mut records = Records::default();
        let fp = [1u8; 16];
        let tune = [2u8; 16];
        let other = [3u8; 16];

        assert_eq!(records.get(&fp, &tune), None);
        assert!(records.insert(&fp, &tune, 1200));
        assert!(!records.insert(&fp, &tune, 1300));
        assert!(records.insert(&fp, &tune, 1100));
        assert_eq!(records.get(&fp, &tune), Some(1100));

        assert!(records.insert(&other, &tune, 2400));
        assert_eq!(records.get(&other, &tune), Some(2400));
        assert_eq!(records.get(&fp, &tune), Some(1100));
    }

    #[test]
    fn a_record_under_other_tuning_is_a_different_record() {
        let mut records = Records::default();
        let fp = [1u8; 16];
        let tune = [2u8; 16];
        let retuned = [9u8; 16];

        assert!(records.insert(&fp, &tune, 1200));
        assert_eq!(records.get(&fp, &retuned), None, "tuning is part of the key");
        assert!(
            records.insert(&fp, &retuned, 5000),
            "a slow run on new tuning is still its first record"
        );
        assert_eq!(records.get(&fp, &tune), Some(1200));
        assert_eq!(records.get(&fp, &retuned), Some(5000));

        records.prune(&retuned);
        assert_eq!(records.get(&fp, &tune), None, "prune drops other tuning");
        assert_eq!(records.get(&fp, &retuned), Some(5000));
    }

    #[test]
    fn an_older_save_is_migrated_and_keeps_its_settings() {
        let mgr = mem_manager();
        // Built the way a version 1 save was written rather than typed by hand: a
        // fingerprint inside a tuple has no hand-writable RON spelling, and this
        // is the exact shape on disk.
        let legacy = ron::ser::to_string(&LegacyRecords {
            entries: vec![([1u8; 16], 42.0)],
        })
        .expect("serialises");
        let old = format!(
            "(version: 1, settings: (master_volume: 0.5, music_volume: 0.25, \
             sfx_volume: 0.75), best_times: {legacy})"
        );
        mgr.storage().write(&mgr.path(), old.as_bytes()).unwrap();

        let (data, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(data.version, SAVE_VERSION);
        assert_eq!(data.settings.master_volume, 0.5);
        assert!(
            data.best_times.entries.is_empty(),
            "a v1 record has no physics revision, so it is discarded"
        );
        assert!(data.records.is_empty());
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
        assert!(data.records.insert(&[9u8; 16], &[4u8; 16], 1_500));

        save_with(&mgr, &data).unwrap();
        let (loaded, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(loaded.version, SAVE_VERSION);
        assert_eq!(loaded.settings, data.settings);
        assert_eq!(loaded.records, data.records);
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
