//! The piece catalog. Ids are stable on the wire; never reorder.

use serde::{Deserialize, Serialize};

/// Per-instance shape parameters. Unset fields fall back to the piece's
/// own default, so the common case is a bare `PieceInstance`.
///
/// `checkpoint_order` is the one field that does not describe geometry: it is
/// the gate sequence a checkpoint takes in the run, which is otherwise read off
/// the connected chain. Unset means "wherever the road puts it", so every
/// document written before the field existed keeps the order it had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PieceParams {
    /// Straight / ramp / bank / jump length in cells.
    #[serde(default)]
    pub length_cells: Option<u8>,
    /// Curve and loop radius in cells.
    #[serde(default)]
    pub radius_cells: Option<u8>,
    /// Bank angle in degrees, signed.
    #[serde(default)]
    pub bank_deg: Option<i8>,
    /// Steepness of a slope, in degrees from the horizontal.
    ///
    /// Unset keeps the catalogue's default, which is the angle every slope had
    /// before the field existed: the piece still lands on the same half-cell the
    /// grid quantises to, so a document that never set it is unchanged.
    #[serde(default)]
    pub slope_deg: Option<u8>,
    /// Paint for this piece, by index into [`PIECE_COLORS`].
    ///
    /// A track is one colour by default, which is right for a circuit and wrong
    /// for a set piece: a red section, an ice bridge and a desert run are the
    /// fastest way to tell where on a track you are. Unset keeps the default.
    #[serde(default)]
    pub color: Option<u8>,
    /// Position of a checkpoint in the sequence the car must pass, 1-based.
    ///
    /// Checkpoints sharing a number are a set: the car may pass any of them to
    /// advance that step. Unset keeps the piece where the chain puts it, which
    /// is what makes the field optional on every existing track.
    #[serde(default)]
    pub checkpoint_order: Option<u8>,
}

impl PieceParams {
    pub fn length(self, cells: u8) -> Self {
        Self {
            length_cells: Some(cells),
            ..self
        }
    }

    pub fn radius(self, cells: u8) -> Self {
        Self {
            radius_cells: Some(cells),
            ..self
        }
    }

    pub fn bank(self, deg: i8) -> Self {
        Self {
            bank_deg: Some(deg),
            ..self
        }
    }

    pub fn slope(self, deg: u8) -> Self {
        Self {
            slope_deg: Some(deg),
            ..self
        }
    }

    pub fn color(self, index: u8) -> Self {
        Self {
            color: Some(index),
            ..self
        }
    }

    pub fn order(self, nth: u8) -> Self {
        Self {
            checkpoint_order: Some(nth),
            ..self
        }
    }
}

/// Paints a piece can be laid in, by index.
///
/// Twelve, matching the reference: the default summer, two named environments
/// that recolour a whole track, and nine flat colours for marking one section
/// of it. A piece with no paint of its own takes the first.
pub const PIECE_COLORS: [u32; 12] = [
    0x4C6B8A, // default summer road
    0x5077B2, // winter
    0x997240, // desert
    0x131313, 0x501B1B, 0x7F4D2B, 0x93862D, 0x2A5E30, 0x236363, 0x20244B, 0x592759, 0x302318,
];

/// Paint of a piece, resolved: the piece's own, or the default.
pub fn resolve_color(params: &PieceParams) -> u32 {
    params
        .color
        .and_then(|index| PIECE_COLORS.get(index as usize).copied())
        .unwrap_or(PIECE_COLORS[0])
}

/// Axis-aligned grid direction. A connector port faces one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GridDir {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl GridDir {
    pub fn opposite(self) -> Self {
        match self {
            Self::PosX => Self::NegX,
            Self::NegX => Self::PosX,
            Self::PosY => Self::NegY,
            Self::NegY => Self::PosY,
            Self::PosZ => Self::NegZ,
            Self::NegZ => Self::PosZ,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u16)]
pub enum PieceId {
    Start = 0,
    Straight = 1,
    CurveRight90 = 2,
    CurveLeft90 = 3,
    Curve180 = 4,
    RampUp = 5,
    RampDown = 6,
    BankLeft = 7,
    BankRight = 8,
    Loop = 9,
    /// A right then left chicane in one piece.
    SCurve = 10,
    Checkpoint = 11,
    Boost = 12,
    Finish = 13,
    Wall = 14,
}

impl PieceId {
    pub const ALL: [PieceId; 15] = [
        Self::Start,
        Self::Straight,
        Self::CurveRight90,
        Self::CurveLeft90,
        Self::Curve180,
        Self::RampUp,
        Self::RampDown,
        Self::BankLeft,
        Self::BankRight,
        Self::Loop,
        Self::SCurve,
        Self::Checkpoint,
        Self::Boost,
        Self::Finish,
        Self::Wall,
    ];

    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(v: u16) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.as_u16() == v)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Straight => "Straight",
            Self::CurveRight90 => "Curve Right",
            Self::CurveLeft90 => "Curve Left",
            Self::Curve180 => "Curve 180",
            Self::RampUp => "Ramp Up",
            Self::RampDown => "Ramp Down",
            Self::BankLeft => "Bank Left",
            Self::BankRight => "Bank Right",
            Self::Loop => "Loop",
            Self::Checkpoint => "Checkpoint",
            Self::Boost => "Boost",
            Self::Finish => "Finish",
            Self::Wall => "Wall",
            Self::SCurve => "S Curve",
        }
    }

    /// Editor palette grouping.
    ///
    /// Named for what the piece is *for* rather than for what it looks like:
    /// an S-curve is road, and a wall ride is a slope, because that is where a
    /// player building a circuit goes looking for one.
    pub fn group(self) -> &'static str {
        match self {
            Self::Start | Self::Finish | Self::Checkpoint | Self::Boost => "Control",
            Self::Straight
            | Self::CurveRight90
            | Self::CurveLeft90
            | Self::Curve180
            | Self::SCurve => "Road",
            Self::RampUp | Self::RampDown => "Slopes",
            Self::BankLeft | Self::BankRight => "Banks",
            Self::Loop => "Stunts",
            Self::Wall => "Props",
        }
    }

    /// Pieces the car physically drives on. `Wall` is decor only.
    pub fn is_driveable(self) -> bool {
        !matches!(self, Self::Wall)
    }

    /// Pieces whose shape is a slope: the steepness knob means something to
    /// them, and they carry a skirt down to the ground so they are not a
    /// surface floating in the air.
    pub fn is_slope(self) -> bool {
        matches!(self, Self::RampUp | Self::RampDown)
    }

    /// Whether the piece's size is a run of road, rather than the sweep of an
    /// arc. A curve or a wall ride is sized by its radius; offering a length
    /// control on one is offering a control that does nothing.
    pub fn has_length(self) -> bool {
        !matches!(
            self,
            Self::CurveRight90 | Self::CurveLeft90 | Self::Curve180 | Self::Loop | Self::SCurve
        )
    }

    /// Whether the piece is sized by a radius, and so offers a radius control.
    pub fn has_radius(self) -> bool {
        matches!(
            self,
            Self::CurveRight90 | Self::CurveLeft90 | Self::Curve180 | Self::Loop | Self::SCurve
        )
    }

    /// Pieces that are a straight stretch with a special role.
    pub fn is_zone(self) -> bool {
        matches!(
            self,
            Self::Start | Self::Finish | Self::Checkpoint | Self::Boost
        )
    }
}

/// Static authoring metadata. Geometry itself is *not* here: it is computed
/// from the id plus per-instance params by [`crate::geometry::piece_shape`].
#[derive(Clone, Copy, Debug)]
pub struct PieceDef {
    pub id: PieceId,
    pub default_length_cells: u8,
    pub default_radius_cells: u8,
    pub default_bank_deg: i8,
    /// Steepness a slope piece has when the author has not set one. Zero for
    /// pieces that are not slopes, where the value means nothing.
    pub default_slope_deg: u8,
    pub is_start: bool,
    pub is_finish: bool,
    pub is_checkpoint: bool,
    pub is_boost: bool,
}

pub const CATALOG: &[PieceDef] = &[
    PieceDef {
        id: PieceId::Start,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: true,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::Straight,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::CurveRight90,
        default_length_cells: 0,
        default_radius_cells: 2,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::CurveLeft90,
        default_length_cells: 0,
        default_radius_cells: 2,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::Curve180,
        default_length_cells: 0,
        default_radius_cells: 2,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::RampUp,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 16,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::RampDown,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 16,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::BankLeft,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 20,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::BankRight,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 20,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::Loop,
        default_length_cells: 0,
        default_radius_cells: 3,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::SCurve,
        default_length_cells: 0,
        default_radius_cells: 2,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::Checkpoint,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: true,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::Boost,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: true,
    },
    PieceDef {
        id: PieceId::Finish,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: true,
        is_checkpoint: false,
        is_boost: false,
    },
    PieceDef {
        id: PieceId::Wall,
        default_length_cells: 4,
        default_radius_cells: 0,
        default_bank_deg: 0,
        default_slope_deg: 0,
        is_start: false,
        is_finish: false,
        is_checkpoint: false,
        is_boost: false,
    },
];

pub fn catalog() -> &'static [PieceDef] {
    CATALOG
}

pub fn catalog_by_id(id: PieceId) -> &'static PieceDef {
    &CATALOG[id.as_u16() as usize]
}

/// Resolve the effective parameters for one instance.
pub fn resolve_params(id: PieceId, params: &PieceParams) -> (u8, u8, i8) {
    let def = catalog_by_id(id);
    (
        params
            .length_cells
            .unwrap_or(def.default_length_cells)
            .clamp(1, 16),
        // Radius 0 means "piece has no meaningful radius" (straights). Only
        // clamp to the minimum for pieces that are actually sized by radius.
        if def.default_radius_cells == 0 {
            0
        } else {
            params
                .radius_cells
                .unwrap_or(def.default_radius_cells)
                .clamp(1, 8)
        },
        params.bank_deg.unwrap_or(def.default_bank_deg),
    )
}

/// Rotate a local cell offset by `yaw` quarter turns about +Y. Matches
/// `Quat::from_rotation_y(yaw * FRAC_PI_2)`: yaw 1 sends local +Z to world +X.
pub fn rotate_local_xz([x, y, z]: [i16; 3], yaw: u8) -> [i16; 3] {
    match yaw % 4 {
        0 => [x, y, z],
        1 => [z, y, -x],
        2 => [-x, y, -z],
        _ => [-z, y, x],
    }
}

/// Rotate a local grid direction by `yaw` quarter turns.
pub fn rotate_local_dir(dir: GridDir, yaw: u8) -> GridDir {
    match dir {
        GridDir::PosZ => match yaw % 4 {
            0 => GridDir::PosZ,
            1 => GridDir::PosX,
            2 => GridDir::NegZ,
            _ => GridDir::NegX,
        },
        GridDir::PosX => match yaw % 4 {
            0 => GridDir::PosX,
            1 => GridDir::NegZ,
            2 => GridDir::NegX,
            _ => GridDir::PosZ,
        },
        GridDir::NegZ => match yaw % 4 {
            0 => GridDir::NegZ,
            1 => GridDir::NegX,
            2 => GridDir::PosZ,
            _ => GridDir::PosX,
        },
        GridDir::NegX => match yaw % 4 {
            0 => GridDir::NegX,
            1 => GridDir::PosZ,
            2 => GridDir::PosX,
            _ => GridDir::NegZ,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matches_variant_order() {
        for (i, id) in PieceId::ALL.iter().enumerate() {
            assert_eq!(*id as usize, i, "catalog order must match ordinals");
            assert_eq!(catalog_by_id(*id).id, *id);
        }
        assert_eq!(PieceId::from_u16(9), Some(PieceId::Loop));
        assert_eq!(PieceId::from_u16(999), None);
    }

    #[test]
    fn yaw_rotation_is_a_group_of_order_four() {
        let p = [1, 2, 3];
        let mut acc = p;
        for i in 0..4 {
            assert_eq!(rotate_local_xz(acc, i), rotate_local_xz(p, i), "yaw {i}");
        }
        acc = p;
        for _ in 0..4 {
            acc = rotate_local_xz(acc, 1);
        }
        assert_eq!(acc, p, "four quarter turns is identity");
        assert_eq!(rotate_local_xz([0, 5, 1], 1), [1, 5, 0]);
    }

    #[test]
    fn directions_round_trip_under_yaw() {
        for yaw in 0..4 {
            for dir in [
                GridDir::PosX,
                GridDir::NegX,
                GridDir::PosZ,
                GridDir::NegZ,
                GridDir::PosY,
                GridDir::NegY,
            ] {
                let back = rotate_local_dir(rotate_local_dir(dir, yaw), 4 - yaw);
                assert_eq!(back, dir, "dir {dir:?} yaw {yaw} must round-trip");
            }
        }
    }

    #[test]
    fn params_fall_back_to_catalog_defaults() {
        let (l, r, b) = resolve_params(PieceId::Straight, &PieceParams::default());
        // Four cells: a straight is a length of road, not a marker one cell
        // long. The catalogue's own value is the expectation, so this moves with
        // it instead of pinning a number that was only ever a default.
        let want = catalog_by_id(PieceId::Straight).default_length_cells;
        assert_eq!((l, r, b), (want, 0, 0));
        let (l, r, _) = resolve_params(PieceId::Straight, &PieceParams::default().length(4));
        assert_eq!((l, r), (4, 0));
        assert_eq!(
            resolve_params(PieceId::BankRight, &PieceParams::default()).2,
            20
        );
    }
}
#[cfg(test)]
mod colour_tests {
    use super::*;

    #[test]
    fn a_piece_can_be_painted_and_the_paint_survives_a_round_trip() {
        let painted = PieceParams::default().color(5);
        assert_eq!(painted.color, Some(5));
        assert_eq!(
            PieceParams::default().color,
            None,
            "an unpainted piece has no paint of its own"
        );
        assert_eq!(resolve_color(&painted), PIECE_COLORS[5]);
        assert_eq!(
            resolve_color(&PieceParams::default()),
            PIECE_COLORS[0],
            "an unpainted piece takes the default"
        );

        let mut doc = crate::demo_track();
        let uid = doc.pieces[1].uid;
        for piece in doc.pieces.iter_mut() {
            if piece.uid == uid {
                piece.params.color = Some(7);
            }
        }
        let back = crate::TrackDocument::from_ron(&doc.to_ron().unwrap()).unwrap();
        assert_eq!(
            back.pieces
                .iter()
                .find(|p| p.uid == uid)
                .map(|p| p.params.color),
            Some(Some(7)),
            "the paint must survive the document's own codec"
        );
    }

    #[test]
    fn an_out_of_range_paint_falls_back_rather_than_panicking() {
        let odd = PieceParams::default().color(200);
        assert_eq!(resolve_color(&odd), PIECE_COLORS[0]);
    }

    #[test]
    fn repainting_a_piece_changes_the_gameplay_fingerprint() {
        let a = crate::demo_track();
        let mut b = a.clone();
        b.pieces[1].params.color = Some(3);
        assert_ne!(
            crate::gameplay_fingerprint(&a),
            crate::gameplay_fingerprint(&b),
            "a repaint is visible in a ghost, so it must version the run"
        );
    }
}
