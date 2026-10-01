//! Canonical track fingerprint.
//!
//! Hashes gameplay-relevant state only. Cosmetic metadata (name, author)
//! is excluded so renaming a track does not invalidate the replays
//! recorded on it, while any change that moves the road does.

use crate::TrackDocument;

/// 128-bit gameplay identity, blake3-derived.
pub type TrackFingerprint = [u8; 16];

pub fn gameplay_fingerprint(doc: &TrackDocument) -> TrackFingerprint {
    let mut pieces: Vec<_> = doc.pieces.iter().collect();
    // Uid order, not array order: shuffling the file must not change the
    // fingerprint of an identical track.
    pieces.sort_by_key(|p| p.uid);

    let mut h = blake3::Hasher::new();
    h.update(b"retrackt-track-v1");
    h.update(&doc.format.to_le_bytes());
    h.update(&doc.cell_size.to_bits().to_le_bytes());

    match doc.spawn_xyz {
        Some(v) => {
            h.update(&[1]);
            for f in v {
                h.update(&f.to_bits().to_le_bytes());
            }
        }
        None => {
            h.update(&[0]);
        }
    }
    h.update(&doc.spawn_yaw_deg.to_bits().to_le_bytes());
    h.update(&(pieces.len() as u32).to_le_bytes());

    for p in pieces {
        h.update(&p.uid.0.to_le_bytes());
        h.update(&p.id.as_u16().to_le_bytes());
        for c in p.cell {
            h.update(&c.to_le_bytes());
        }
        h.update(&[p.yaw & 3]);
        h.update(&[p.params.length_cells.unwrap_or(0)]);
        h.update(&[p.params.radius_cells.unwrap_or(0)]);
        h.update(&[p.params.bank_deg.unwrap_or(0) as u8]);
    }

    finish(&h.finalize())
}

/// 128-bit identity of the vehicle tuning + simulation version, so a replay
/// recorded under one physics revision is not re-verified under another.
pub fn physics_fingerprint(version: u32, tuning: &crate::replay::PhysicsStamp) -> TrackFingerprint {
    let mut h = blake3::Hasher::new();
    h.update(b"retrackt-physics-v1");
    h.update(&version.to_le_bytes());
    h.update(&tuning.0);
    finish(&h.finalize())
}

fn finish(digest: &blake3::Hash) -> TrackFingerprint {
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PieceId, PieceInstance, PieceParams, PieceUid, TrackDocument};

    #[test]
    fn identical_tracks_fingerprint_equal() {
        assert_eq!(
            gameplay_fingerprint(&demo()),
            gameplay_fingerprint(&demo()),
            "same document must hash the same"
        );
    }

    #[test]
    fn cosmetic_renaming_does_not_break_the_fingerprint() {
        let a = demo();
        let mut b = demo();
        b.name = "Renamed".into();
        b.author = "someone else".into();
        assert_eq!(
            gameplay_fingerprint(&a),
            gameplay_fingerprint(&b),
            "metadata must not enter the hash"
        );
    }

    #[test]
    fn piece_order_does_not_matter() {
        let a = demo();
        let mut b = demo();
        b.pieces.reverse();
        assert_eq!(gameplay_fingerprint(&a), gameplay_fingerprint(&b));
    }

    #[test]
    fn moving_a_piece_changes_the_fingerprint() {
        let a = demo();
        let mut b = demo();
        b.pieces[2].cell[2] += 1;
        assert_ne!(gameplay_fingerprint(&a), gameplay_fingerprint(&b));
    }

    #[test]
    fn retuning_a_piece_changes_the_fingerprint() {
        let a = demo();
        let mut b = demo();
        b.pieces[2].params = PieceParams::default().length(4);
        assert_ne!(gameplay_fingerprint(&a), gameplay_fingerprint(&b));
    }

    #[test]
    fn yaw_change_changes_the_fingerprint() {
        let a = demo();
        let mut b = demo();
        b.pieces[2].yaw = (b.pieces[2].yaw + 1) % 4;
        assert_ne!(gameplay_fingerprint(&a), gameplay_fingerprint(&b));
    }

    #[test]
    fn physics_version_separates_fingerprints() {
        let stamp = crate::replay::PhysicsStamp([0u8; 16]);
        assert_ne!(
            physics_fingerprint(1, &stamp),
            physics_fingerprint(2, &stamp)
        );
    }

    fn demo() -> TrackDocument {
        let mut doc = TrackDocument::empty();
        doc.pieces = vec![
            PieceInstance::new(PieceId::Straight, [0, 0, 0]).with_uid(PieceUid(1)),
            PieceInstance::new(PieceId::Straight, [0, 0, 1]).with_uid(PieceUid(2)),
            PieceInstance::new(PieceId::CurveRight90, [0, 0, 2]).with_uid(PieceUid(3)),
        ];
        doc.normalize_uids();
        doc
    }
}
