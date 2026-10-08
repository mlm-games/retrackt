use std::path::{Path, PathBuf};

use game_utils::save::{SaveManager, Versioned};
use game_utils::save_store::{LoadStatus, SaveStore};
use game_utils::storage::{FsStorage, Storage};
use serde::{Deserialize, Serialize};

use retrackt_format::fingerprint::TrackFingerprint;
use retrackt_format::{
    FORMAT_VERSION, ReplayTape, TrackDocument, decode_replay, encode_replay,
};

pub const SAVE_VERSION: u32 = 3;

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

/// One kept replay tape. The tape itself is a file; this is the row the library
/// lists it by.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct GhostEntry {
    pub name: String,
    /// File stem under the ghosts directory. Separate from `name` because the
    /// library is free-form: two ghosts may carry the same name, and the stem is
    /// what the bytes are actually keyed on.
    pub file: String,
    pub track: TrackFingerprint,
    pub physics: TrackFingerprint,
    /// Race time of the recorded run, in ticks.
    pub ticks: u32,
}

/// Every kept tape, across all tracks.
///
/// Not keyed on track: a ghost is chosen from the whole library, and a tape whose
/// track or tuning does not match the run being started is refused by
/// [`ReplayTape::compatible`] rather than silently resimulated wrong.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct GhostLibrary {
    entries: Vec<GhostEntry>,
}

/// Ceiling on kept tapes. Each is a file the player cannot see the size of, and
/// saving one is a single tap on the results screen, so the library is bounded
/// rather than grown until the save directory fills.
const MAX_GHOSTS: usize = 64;

impl GhostLibrary {
    pub fn all(&self) -> &[GhostEntry] {
        &self.entries
    }

    pub fn get(&self, file: &str) -> Option<&GhostEntry> {
        self.entries.iter().find(|e| e.file == file)
    }

    /// Err iff the library is already at [`MAX_GHOSTS`]; the player frees a slot
    /// by deleting one.
    pub fn insert(&mut self, entry: GhostEntry) -> Result<(), String> {
        if self.entries.len() >= MAX_GHOSTS {
            return Err(format!(
                "Ghost library is full ({MAX_GHOSTS}). Delete one first."
            ));
        }
        self.entries.push(entry);
        Ok(())
    }

    /// The removed row, so the caller can delete the tape it named.
    pub fn remove(&mut self, file: &str) -> Option<GhostEntry> {
        let index = self.entries.iter().position(|e| e.file == file)?;
        Some(self.entries.remove(index))
    }

    /// Drop tapes that can no longer be played, and rows whose file has gone:
    /// a retune invalidates the first, and a deleted file would otherwise
    /// leave a row that fails to load every time it is picked.
    pub fn prune(&mut self, physics: &TrackFingerprint, present: &dyn Fn(&str) -> bool) {
        self.entries
            .retain(|e| e.physics == *physics && present(&e.file));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
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
    #[serde(default)]
    pub ghosts: GhostLibrary,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            version: SAVE_VERSION,
            settings: Settings::default(),
            best_times: LegacyRecords::default(),
            records: Records::default(),
            ghosts: GhostLibrary::default(),
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
        // Nothing to do for 3: the library is `#[serde(default)]`, so a version 2
        // save loads as an empty one rather than failing to deserialise.
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

/// A name reduced to a safe single path segment. Shared by tracks and ghosts:
/// both are keyed on a file stem under their own directory.
fn file_stem(name: &str) -> String {
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
    // A name that sanitises away entirely still needs a stable file: the stem is
    // how the file is found again, and it must not change between saves.
    if cleaned.is_empty() {
        "track".to_string()
    } else {
        cleaned
    }
}

fn tracks_dir() -> PathBuf {
    manager().data_dir().join("tracks")
}

fn ghosts_dir() -> PathBuf {
    manager().data_dir().join("ghosts")
}

/// Tapes are zlib-compressed, so the only integrity test is a decode. A free
/// function rather than a closure because [`SaveStore::load`] takes a `fn` item.
fn is_intact_ghost(bytes: &[u8]) -> bool {
    decode_replay(bytes).is_ok()
}

fn ghost_store<S: Storage>(dir: &Path, stem: &str, storage: S) -> SaveStore<S> {
    SaveStore::new_with_storage(dir, format!("{stem}.ghost"), storage)
        .with_validator(is_intact_ghost)
}

/// A stem no ghost file already occupies. Names are free-form and may repeat, so
/// the name is not a usable file key and collisions are numbered off.
///
/// Takes the storage backend by value rather than by reference: `&S` would make
/// `storage.clone()` resolve to `Clone for &T` and hand back another `&S`.
fn free_ghost_stem<S: Storage>(storage: S, dir: &Path, name: &str) -> String {
    let stem = file_stem(name);
    let taken = |candidate: &str| ghost_store(dir, candidate, storage.clone()).exists();
    if !taken(&stem) {
        return stem;
    }
    for n in 2..1000 {
        let candidate = format!("{stem}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    stem
}

/// Write `tape` under a free stem and return the row describing it.
fn save_ghost_with<S: Storage>(
    storage: S,
    dir: &Path,
    tape: &ReplayTape,
    name: &str,
) -> Result<GhostEntry, String> {
    let file = free_ghost_stem(storage.clone(), dir, name);
    let bytes = encode_replay(tape).map_err(|e| e.to_string())?;
    ghost_store(dir, &file, storage).write(&bytes)?;
    Ok(GhostEntry {
        name: name.to_string(),
        file,
        track: tape.header.track,
        physics: tape.header.physics,
        ticks: tape.header.finish_tick,
    })
}

fn load_ghost_with<S: Storage>(storage: S, dir: &Path, file: &str) -> Option<ReplayTape> {
    let bytes = ghost_store(dir, file, storage).load(&is_intact_ghost, &[]).data?;
    decode_replay(&bytes).ok()
}

pub fn save_ghost(tape: &ReplayTape, name: &str) -> Result<GhostEntry, String> {
    save_ghost_with(FsStorage, &ghosts_dir(), tape, name)
}

pub fn load_ghost(file: &str) -> Option<ReplayTape> {
    load_ghost_with(FsStorage, &ghosts_dir(), file)
}

/// Drop the tape and its rotations. Missing is not an error: the caller wants
/// the file gone, and it already is.
pub fn delete_ghost(file: &str) {
    ghost_store(&ghosts_dir(), file, FsStorage).delete();
}

/// Stems of every ghost file on disk. Read through the platform storage backend
/// for the same reason [`stored_tracks`] does: on wasm the raw path lists nothing.
pub fn stored_ghost_files() -> Vec<String> {
    let storage = FsStorage;
    let Ok(paths) = storage.read_dir(&ghosts_dir()) else {
        return Vec::new();
    };
    let mut files: Vec<String> = paths
        .into_iter()
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            (p.extension().and_then(|e| e.to_str()) == Some("ghost")).then_some(stem)
        })
        .collect();
    files.sort();
    files
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
    let file = format!("{}.ron", file_stem(&doc.name));
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
    let file = format!("{}.ron", file_stem(name));
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

    fn ghost_dir() -> PathBuf {
        PathBuf::from("/mem/ghosts")
    }

    fn sample_tape(track: u8, ticks: u32) -> ReplayTape {
        let mut tape = ReplayTape::new([track; 16], [2u8; 16], crate::SIM_HZ as u16);
        let input = retrackt_format::PackedInput::new(0.0, 1.0, 0.0, false, false, false);
        for _ in 0..ticks {
            tape.push(input);
        }
        tape.header.finish_tick = ticks;
        tape
    }

    fn entry(name: &str, file: &str, track: u8, physics: u8, ticks: u32) -> GhostEntry {
        GhostEntry {
            name: name.into(),
            file: file.into(),
            track: [track; 16],
            physics: [physics; 16],
            ticks,
        }
    }

    #[test]
    fn a_ghost_tape_round_trips_through_memory_storage() {
        let storage = MemoryStorage::new();
        let dir = ghost_dir();
        let tape = sample_tape(7, 500);

        let saved = save_ghost_with(storage.clone(), &dir, &tape, "My run").expect("saves");
        assert_eq!(saved.name, "My run");
        assert_eq!(saved.track, tape.header.track);
        assert_eq!(saved.physics, tape.header.physics);
        assert_eq!(saved.ticks, 500);
        assert_eq!(
            load_ghost_with(storage, &dir, &saved.file).expect("loads"),
            tape
        );
    }

    #[test]
    fn two_ghosts_of_the_same_name_get_distinct_files() {
        let storage = MemoryStorage::new();
        let dir = ghost_dir();
        let first = save_ghost_with(storage.clone(), &dir, &sample_tape(1, 10), "Best")
            .expect("saves");
        let second = save_ghost_with(storage, &dir, &sample_tape(1, 20), "Best").expect("saves");

        assert_ne!(
            first.file, second.file,
            "a repeated name must not overwrite the first tape"
        );
    }

    #[test]
    fn a_ghost_name_cannot_escape_the_ghost_directory() {
        let storage = MemoryStorage::new();
        let dir = ghost_dir();
        let saved =
            save_ghost_with(storage.clone(), &dir, &sample_tape(1, 10), "../../etc/№7: ünïcode?")
                .expect("saves");

        let stem = file_stem(&saved.name);
        assert!(!stem.contains('/') && !stem.contains('\\') && !stem.contains(".."));
        assert!(storage.exists(&dir.join(format!("{stem}.ghost"))));
        assert!(load_ghost_with(storage, &dir, &saved.file).is_some());
    }

    #[test]
    fn a_missing_or_corrupt_ghost_file_reads_as_none() {
        let storage = MemoryStorage::new();
        let dir = ghost_dir();
        assert!(load_ghost_with(storage.clone(), &dir, "never saved").is_none());

        ghost_store(&dir, "broken", storage.clone())
            .write(b"((( not a tape at all")
            .unwrap();
        assert!(load_ghost_with(storage, &dir, "broken").is_none());
    }

    #[test]
    fn a_tape_from_another_track_or_rate_is_refused_for_playback() {
        let doc = retrackt_format::demo_track();
        let world = crate::sim::world::TrackWorld::from_doc(&doc);
        let track = retrackt_format::gameplay_fingerprint(&doc);
        let mut ghost = crate::app::schedule::GhostRes::default();

        let wrong_track = ReplayTape::new(
            [9u8; 16],
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16,
        );
        assert!(ghost.arm(wrong_track, track, &world).is_err());
        assert!(!ghost.armed(), "a refused tape must leave no ghost");

        // Same track and tuning, another rate: the same inputs step over
        // different distances, so playback would not match the recorded run.
        let wrong_hz = ReplayTape::new(
            track,
            crate::sim::car::physics_fingerprint(),
            crate::SIM_HZ as u16 - 1,
        );
        assert!(ghost.arm(wrong_hz, track, &world).is_err());
        assert!(!ghost.armed());
    }

    #[test]
    fn the_library_drops_unplayable_and_missing_rows() {
        let mut library = GhostLibrary::default();
        library.insert(entry("keep", "keep", 1, 2, 100)).unwrap();
        library.insert(entry("retuned", "retuned", 1, 9, 100)).unwrap();
        library.insert(entry("deleted", "deleted", 1, 2, 100)).unwrap();
        assert_eq!(library.all().len(), 3);

        library.prune(&[2u8; 16], &|file| file == "keep");
        assert_eq!(library.all().len(), 1);
        assert_eq!(library.all()[0].name, "keep");
        assert!(library.get("keep").is_some());
        assert!(library.get("retuned").is_none(), "other tuning is unplayable");
        assert!(
            library.get("deleted").is_none(),
            "a row whose file is gone can never be played"
        );
    }

    #[test]
    fn the_library_is_bounded_and_says_why() {
        let mut library = GhostLibrary::default();
        for n in 0..MAX_GHOSTS {
            library
                .insert(entry(&format!("g{n}"), &format!("g{n}"), 1, 2, 100))
                .unwrap();
        }
        assert!(library.insert(entry("one more", "x", 1, 2, 100)).is_err());
        assert_eq!(library.all().len(), MAX_GHOSTS);

        assert!(library.remove("g0").is_some());
        assert!(
            library.insert(entry("now there is room", "y", 1, 2, 100)).is_ok(),
            "deleting frees a slot"
        );
        assert!(library.remove("g0").is_none(), "removing twice is a no-op");
    }

    #[test]
    fn deleting_a_ghost_removes_it_from_the_save_round_trip() {
        let mgr = mem_manager();
        let mut data = SaveData::default();
        data.ghosts
            .insert(entry("two", "two", 1, [2u8; 16], 100))
            .unwrap();
        data.ghosts
            .insert(entry("one", "one", 1, [2u8; 16], 200))
            .unwrap();
        save_with(&mgr, &data).expect("saves");

        let mut loaded = load_with(&mgr).0;
        assert_eq!(loaded.ghosts.all().len(), 2);
        loaded.ghosts.remove("two");
        save_with(&mgr, &loaded).expect("saves");

        let (back, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(back.ghosts.all().len(), 1);
        assert_eq!(back.ghosts.all()[0].name, "one");
    }

    #[test]
    fn a_version_2_save_loads_with_an_empty_ghost_library() {
        let mgr = mem_manager();
        let old = "(version: 2, settings: (master_volume: 0.5, music_volume: 0.25, \
                   sfx_volume: 0.75))";
        mgr.storage().write(&mgr.path(), old.as_bytes()).unwrap();

        let (data, status) = load_with(&mgr);
        assert_eq!(status, LoadStatus::Ok);
        assert_eq!(data.version, SAVE_VERSION);
        assert!(data.ghosts.is_empty(), "no tapes existed before version 3");
        assert_eq!(data.settings.master_volume, 0.5);
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
        let stem = file_stem(&doc.name);
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
