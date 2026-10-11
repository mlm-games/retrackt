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
pub use demo::{ChainBuilder, builtin_tracks, demo_track, stunt_track};
pub use fingerprint::gameplay_fingerprint;
pub use geometry::{Centerline, RawMesh, piece_shape};
pub use piece::{PieceDef, PieceId, PieceParams, catalog, catalog_by_id, rotate_local_xz};
pub use replay::{PackedInput, ReplayError, ReplayTape, decode_replay, encode_replay};
pub use track::{FORMAT_VERSION, MAX_PIECES, PieceInstance, PieceUid, TrackDocument, TrackError};
pub use validate::{Diagnostic, Severity, is_unraceable, validate};

/// Half the drivable road width, in cells.
///
/// A carriageway is wider than one grid cell. At a 5 m cell this is a 13 m
/// road, which is what the pieces are modelled against: a road one cell wide
/// puts the car's wheels within its own body length of both kerbs and makes
/// every piece read as a corridor rather than as a road.
pub const ROAD_HALF_CELLS: f32 = 1.315;
/// Kerb height in metres, drawn either side of the drivable surface.
pub const KERB_HEIGHT: f32 = 0.5;
/// Cell edge length in metres.
///
/// The same in all three axes as the game this is modelled against, so a grid
/// cell is a cube and a piece is a whole number of cells long, wide and tall.
pub const DEFAULT_CELL_SIZE: f32 = 5.0;
