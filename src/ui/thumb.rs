//! Track thumbnails for the library lists.
//!
//! A schematic rather than a render: the track's footprint is drawn from its own
//! geometry, one small box per occupied cell, coloured by what the piece is. A
//! true screenshot would need an offscreen render target per track per list
//! frame, which is a large cost for a 64px row, and would show the player's car
//! parked at the spawn — the one thing that says nothing about the track.

use std::collections::BTreeMap;

use repose_core::{AlignItems, Color, Modifier, View};
use repose_ui::{Box, Column, Row, ViewExt};
use retrackt_format::{PieceId, PieceUid, TrackDocument, piece::catalog_by_id};

use crate::app::theme;

/// A cell in plan view: X and Z only. Y is dropped because a thumbnail shows the
/// plan, and a track's plan is its XZ extent. A loop still reads as a loop.
type Plan = (i16, i16);

/// Cell colours, chosen to match the world so a thumbnail and the scene it
/// summarises agree: road grey, checkpoint cyan, finish orange, boost yellow,
/// start orange, wall near-black.
fn cell_color(id: PieceId) -> Color {
    let def = catalog_by_id(id);
    let hex = if def.is_start {
        0xFFB020
    } else if def.is_finish {
        0xFFB020
    } else if def.is_checkpoint {
        0x45C3F0
    } else if def.is_boost {
        0xFFD23F
    } else if id == PieceId::Wall {
        0x2E333B
    } else {
        0x9BA1A9
    };
    rgb(hex)
}

fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

/// The cells one piece covers in plan view.
fn piece_plan(doc: &TrackDocument, inst: &retrackt_format::PieceInstance) -> Vec<Plan> {
    let shape = retrackt_format::piece_shape(inst.id, &inst.params, doc.cell_size);
    shape
        .reserve_cells()
        .iter()
        .map(|local| {
            let w = retrackt_format::rotate_local_xz(*local, inst.yaw);
            (inst.cell[0].saturating_add(w[0]), inst.cell[2].saturating_add(w[2]))
        })
        .collect()
}

/// The whole track in plan view: one colour per cell.
///
/// Later pieces overwrite earlier ones on a shared cell, so a piece placed on
/// top of another is the one the player sees, which is also the one the road
/// under the car belongs to.
fn plan_of(doc: &TrackDocument) -> BTreeMap<Plan, Color> {
    let mut out = BTreeMap::new();
    for inst in &doc.pieces {
        let color = cell_color(inst.id);
        for cell in piece_plan(doc, inst) {
            out.insert(cell, color);
        }
    }
    out
}

/// A `size`-square schematic of `doc`, or nothing when it has no geometry.
pub fn thumbnail(doc: &TrackDocument, size: f32) -> Option<View> {
    let plan = plan_of(doc);
    if plan.is_empty() {
        return None;
    }
    let (Some(lo_x), Some(hi_x)) = (
        plan.keys().map(|(x, _)| *x).min(),
        plan.keys().map(|(x, _)| *x).max(),
    ) else {
        return None;
    };
    let (Some(lo_z), Some(hi_z)) = (
        plan.keys().map(|(_, z)| *z).min(),
        plan.keys().map(|(_, z)| *z).max(),
    ) else {
        return None;
    };

    let span_x = i32::from(hi_x - lo_x) + 1;
    let span_z = i32::from(hi_z - lo_z) + 1;
    // One scale for both axes, so the drawing keeps the track's proportions: a
    // thumbnail that stretched a long circuit into a square would misdescribe it.
    let cell_px = (size / span_x.max(span_z) as f32).max(1.0);

    // Every cell is a fixed size and must not be squeezed: a flex row would
    // otherwise shrink a long track's cells to fit the box and the drawing would
    // no longer be to scale.
    let cell_modifier = |paint: Option<Color>| {
        let mut m = Modifier::new()
            .width(theme::dp(cell_px))
            .height(theme::dp(cell_px))
            .flex_shrink(0.0);
        if let Some(color) = paint {
            m = m.background(color);
        }
        m
    };

    let mut rows: Vec<View> = Vec::with_capacity(span_z as usize);
    for z in lo_z..=hi_z {
        let mut cells: Vec<View> = Vec::with_capacity(span_x as usize);
        for x in lo_x..=hi_x {
            cells.push(Box(cell_modifier(plan.get(&(x, z)).copied())));
        }
        rows.push(
            Row(Modifier::new().align_items(AlignItems::CENTER).flex_shrink(0.0)).child(cells),
        );
    }

    Some(
        Box(Modifier::new()
            .width(theme::dp(size))
            .height(theme::dp(size))
            .background(theme::background())
            .border(theme::dp(1.0), theme::text_dim(), theme::dp(4.0))
            .clip_rounded(theme::dp(4.0)))
        .child(Column(Modifier::new()).child(rows)),
    )
}

/// The piece covering a plan-view cell, if any. Lowest uid wins so two pieces
/// sharing a cell answer the same way every time.
pub fn piece_at(doc: &TrackDocument, x: i16, z: i16) -> Option<PieceUid> {
    doc.pieces
        .iter()
        .filter(|inst| piece_plan(doc, inst).contains(&(x, z)))
        .min_by_key(|inst| inst.uid)
        .map(|inst| inst.uid)
}

/// How much road a track has, for a list row.
pub fn summary(doc: &TrackDocument) -> String {
    format!("{} pieces", doc.pieces.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrackt_format::demo_track;

    #[test]
    fn a_built_in_track_occupies_cells() {
        let plan = plan_of(&demo_track());
        assert!(plan.len() > 10, "a circuit is more than a few cells");
    }

    #[test]
    fn each_kind_of_piece_is_told_apart_by_colour() {
        for id in [
            PieceId::Checkpoint,
            PieceId::Start,
            PieceId::Finish,
            PieceId::Boost,
            PieceId::Straight,
        ] {
            let road = cell_color(PieceId::Straight);
            assert_ne!(cell_color(id), road, "{id:?} must not look like road");
        }
    }

    #[test]
    fn the_plan_spans_both_ends_of_the_demo_circuit() {
        let plan = plan_of(&demo_track());
        let xs: Vec<i16> = plan.keys().map(|(x, _)| *x).collect();
        let zs: Vec<i16> = plan.keys().map(|(_, z)| *z).collect();
        assert!(xs.iter().max().unwrap() > xs.iter().min().unwrap());
        assert!(zs.iter().max().unwrap() > zs.iter().min().unwrap());
    }

    #[test]
    fn an_empty_track_has_no_thumbnail() {
        assert!(thumbnail(&TrackDocument::empty(), 64.0).is_none());
        assert!(plan_of(&TrackDocument::empty()).is_empty());
    }

    #[test]
    fn a_piece_is_findable_from_a_cell_it_occupies() {
        let doc = demo_track();
        let first = doc.pieces[0];
        let (x, z) = piece_plan(&doc, &first)[0];
        assert_eq!(piece_at(&doc, x, z), Some(first.uid));
    }

    #[test]
    fn a_cell_nothing_occupies_names_no_piece() {
        assert_eq!(piece_at(&demo_track(), 300, 300), None);
    }

    #[test]
    fn two_pieces_on_one_cell_answer_the_same_way_every_time() {
        // Lowest uid, so the answer cannot depend on the order the file stores.
        let mut doc = demo_track();
        let shared = piece_plan(&doc, &doc.pieces[2])[0];
        let uid = doc.next_piece_uid();
        doc.pieces.push(
            retrackt_format::PieceInstance::new(PieceId::Wall, [shared.0, 0, shared.1])
                .with_uid(uid),
        );
        let first = piece_at(&doc, shared.0, shared.1);
        doc.pieces.reverse();
        assert_eq!(piece_at(&doc, shared.0, shared.1), first);
    }

    #[test]
    fn the_summary_counts_pieces() {
        let doc = demo_track();
        assert_eq!(summary(&doc), format!("{} pieces", doc.pieces.len()));
    }
}
