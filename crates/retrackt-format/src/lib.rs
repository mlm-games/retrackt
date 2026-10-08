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

/// Half the drivable road width, in cells. Road is one cell wide.
pub const ROAD_HALF_CELLS: f32 = 0.5;
/// Kerb height in metres, drawn either side of the drivable surface.
pub const KERB_HEIGHT: f32 = 0.16;
/// Cell edge length in metres.
pub const DEFAULT_CELL_SIZE: f32 = 4.0;
