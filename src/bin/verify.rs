//! Replays every kept tape and reports the ones that no longer hold up.
//!
//! Nothing in a normal run would notice a tape that decodes but no longer
//! reproduces: the ghost drives somewhere else and the race still finishes. So
//! each tape is run twice through the same `step_tape` the game uses, and the two
//! have to agree to the bit. Exits non-zero on any failure.

use retrackt::app::schedule::{GhostRes, step_tape};
use retrackt::sim::car::Car;
use retrackt::sim::world::TrackWorld;
use retrackt::{SIM_STEP, save};
use retrackt_format::fingerprint::TrackFingerprint;
use retrackt_format::replay::ReplayTape;
use retrackt_format::{TrackDocument, gameplay_fingerprint};

fn main() {
    let (mut data, status) = save::load();
    println!("save: {status:?}");

    let tracks = index_tracks();
    let mut failures = 0;
    let mut skipped = 0;
    let mut checked = 0;

    for entry in data.ghosts.all() {
        checked += 1;
        match verify_ghost(entry, &tracks) {
            Ok(note) => println!("ok    {:<28} {note}", entry.name),
            // Not a defect: a tape for a track the player has since deleted cannot be
            // replayed, and there is nothing wrong with having kept it.
            Err(why) if why.starts_with(SKIP) => {
                skipped += 1;
                println!("skip  {:<28} {why}", entry.name);
            }
            Err(why) => {
                failures += 1;
                println!("FAIL  {:<28} {why}", entry.name);
            }
        }
    }

    // A record's number and the tape behind it have to be the same run. If they are
    // not, the player is shown a personal best they cannot watch or race.
    for record in data.records.entries_mut() {
        let Some(file) = record.ghost.as_deref() else {
            continue;
        };
        checked += 1;
        let why = match save::load_ghost(file) {
            None => format!("ghost \"{file}\" does not decode"),
            Some(tape) if tape.header.finish_tick != record.ticks => format!(
                "ghost \"{file}\" finishes at {} but the record says {}",
                tape.header.finish_tick, record.ticks
            ),
            Some(_) => String::new(),
        };
        if why.is_empty() {
            println!(
                "ok    record on {} at {}",
                short(&record.track),
                record.ticks
            );
        } else {
            failures += 1;
            println!("FAIL  record on {}: {why}", short(&record.track));
        }
    }

    println!(
        "\n{checked} checked, {failures} failed, {skipped} skipped, {} track(s) available",
        tracks.len()
    );
    if failures > 0 {
        std::process::exit(1);
    }
}

/// Every document this installation can build a world from, by fingerprint.
/// Prefix marking a verdict that is not a defect: nothing to check rather than
/// something wrong.
const SKIP: &str = "skip:";

fn index_tracks() -> Vec<(TrackFingerprint, TrackDocument)> {
    let mut all = retrackt_format::builtin_tracks();
    all.extend(save::stored_tracks());
    all.into_iter()
        .map(|doc| (gameplay_fingerprint(&doc), doc))
        .collect()
}

fn verify_ghost(
    entry: &save::GhostEntry,
    tracks: &[(TrackFingerprint, TrackDocument)],
) -> Result<String, String> {
    let Some(tape) = save::load_ghost(&entry.file) else {
        return Err("tape does not decode".into());
    };
    let Some((_, doc)) = tracks.iter().find(|(fp, _)| *fp == entry.track) else {
        return Err(format!(
            "{SKIP} recorded on a track this install does not have ({})",
            short(&entry.track)
        ));
    };
    let world = TrackWorld::from_doc(doc);

    // Armed, not constructed: this is what refuses a tape recorded against
    // different geometry or a different build.
    let first = run_to_end(&tape, entry.track, &world)?;
    let second = run_to_end(&tape, entry.track, &world)?;

    if first.tick == 0 {
        return Err("tape covers no ticks".into());
    }
    if tape.header.finish_tick != 0 && first.tick < tape.header.finish_tick {
        return Err(format!(
            "tape stops at tick {} but its finish is at {}",
            first.tick, tape.header.finish_tick
        ));
    }
    if !same_car(&first.car, &second.car) {
        return Err("two runs of one tape disagree".into());
    }
    // Still on the road. A replay that ends in mid air or under the track is what a
    // player sees as "that ghost drove off".
    if world.ground_at(first.car.pos, 4.0).is_none() {
        return Err(format!(
            "ends off the track at {:?}",
            first.car.pos.to_array()
        ));
    }

    Ok(format!(
        "{} ticks, finish {}, ends at y {:.2}",
        first.tick, tape.header.finish_tick, first.car.pos.y
    ))
}

fn run_to_end(
    tape: &ReplayTape,
    track: TrackFingerprint,
    world: &TrackWorld,
) -> Result<GhostRes, String> {
    let mut ghost = GhostRes::default();
    ghost.arm(tape.clone(), track, world)?;
    while step_tape(&mut ghost, world, SIM_STEP.as_secs_f32()) {}
    Ok(ghost)
}

/// Whether two runs of one tape ended on the same car, exactly.
///
/// Bit for bit, not within a tolerance: a tolerance would pass a ghost that drifts
/// a little further on every lap, which is what determinism exists to prevent.
fn same_car(a: &Car, b: &Car) -> bool {
    let same = |x: f32, y: f32| x.to_bits() == y.to_bits();
    let axes = |c: &Car| {
        c.pos
            .to_array()
            .into_iter()
            .chain(c.vel.to_array())
            .chain(c.heading_dir.to_array())
            .chain(c.ground_normal.to_array())
    };
    axes(a).zip(axes(b)).all(|(x, y)| same(x, y))
        && a.orient.to_array() == b.orient.to_array()
        && a.wheel_spin
            .iter()
            .zip(&b.wheel_spin)
            .all(|(x, y)| same(*x, *y))
        && a.compression
            .iter()
            .zip(&b.compression)
            .all(|(x, y)| same(*x, *y))
        && same(a.steer_angle, b.steer_angle)
        && same(a.air_time, b.air_time)
        && same(a.wall_contact, b.wall_contact)
        && same(a.boost_left, b.boost_left)
        && a.grounded == b.grounded
}

/// Enough of a fingerprint to recognise the track by.
fn short(fp: &TrackFingerprint) -> String {
    fp.iter().take(6).map(|b| format!("{b:02x}")).collect()
}
