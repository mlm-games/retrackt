//! Pure data + geometry for retrackt tracks. No engine, no renderer, no ECS.
//!
//! Every piece is described by one analytic centerline
//! ([`geometry::piece_centerline`]). The drivable mesh, the collision
//! triangles and the deterministic guide are all derived from that single
//! curve, so they cannot drift apart.

pub mod code;
pub mod demo;
pub mod fingerprint;
pub mod geometry;
pub mod piece;
pub mod replay;
pub mod track;
pub mod validate;

pub use code::{CodeError, export_code, import_code};
pub use demo::{ChainBuilder, builtin_tracks, demo_track, generated_track, stunt_track};
pub use fingerprint::gameplay_fingerprint;
pub use geometry::{Centerline, RawMesh, piece_shape};
pub use piece::{
    PIECE_COLORS, PieceDef, PieceId, PieceParams, catalog, catalog_by_id, resolve_color,
    rotate_local_xz,
};
pub use replay::{PackedInput, ReplayError, ReplayTape, decode_replay, encode_replay};
pub use track::{
    DEFAULT_SUN_DIRECTION_DEG, ENVIRONMENT_DESERT, ENVIRONMENT_SUMMER, ENVIRONMENT_WINTER,
    ENVIRONMENTS, FORMAT_VERSION, MAX_PIECES, PieceInstance, PieceUid, SUN_DIRECTION_STEP_DEG,
    TrackDocument, TrackError, is_environment,
};
pub use validate::{Diagnostic, Severity, is_unraceable, validate};

/// Half the drivable road width, in cells.
///
/// A carriageway is wider than one grid cell. At a 5 m cell this is a 13 m
/// road, which is what the pieces are modelled against: a road one cell wide
/// puts the car's wheels within its own body length of both kerbs and makes
/// every piece read as a corridor rather than as a road.
pub const ROAD_HALF_CELLS: f32 = 1.315;
/// Half the wide road width, in cells.
///
/// The reference runs two carriageways: 13.133 m and 16.567 m, a ratio of
/// 1.2614. Scaled to our normal half width that is 1.66 cells, a 16.6 m road
/// for start straights and anywhere else a race needs room to breathe.
pub const WIDE_ROAD_HALF_CELLS: f32 = 1.66;
/// Barrier peak height in metres, either side of the drivable surface.
///
/// A rounded hump, not a kerb: steep faced and car-height, so a slow car is
/// stopped by it and only speed carries one over. Measured off the reference
/// Straight, whose barriers peak 0.73 m over a flat 11.1 m surface.
pub const KERB_HEIGHT: f32 = 0.73;
/// Cell edge length in metres.
///
/// The same in all three axes as the game this is modelled against, so a grid
/// cell is a cube and a piece is a whole number of cells long, wide and tall.
pub const DEFAULT_CELL_SIZE: f32 = 5.0;
