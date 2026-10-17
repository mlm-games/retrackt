//! Shareable codes: `PT1_` for a track, `PTG1_` for a tape, each followed by
//! base64url(zlib(ron)).
//!
//! The prefix carries the format version, so an old code is rejected with a
//! clear error instead of being misparsed, and the two kinds cannot be fed to
//! each other's reader.

use base64::Engine as _;
use std::io::Write as _;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{ReplayTape, TrackDocument};

const PREFIX: &str = "PT1_";
const GHOST_PREFIX: &str = "PTG1_";

/// Hard caps so a pasted blob cannot exhaust memory on decode.
const MAX_CODE_CHARS: usize = 256 * 1024;
const MAX_RAW: usize = 8 * 1024 * 1024;

/// The same caps for a tape, higher throughout. A tape is one packed entry per
/// tick of a run, so the smallest thing worth sharing is larger than the
/// largest track.
const MAX_GHOST_CODE_CHARS: usize = 2 * 1024 * 1024;
const MAX_GHOST_RAW: usize = 24 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CodeError {
    #[error("this is not a retrackt share code")]
    BadPrefix,
    #[error("share code is too long")]
    TooLong,
    #[error("share code is truncated or corrupt")]
    Corrupt,
    #[error("this was made by a newer version of retrackt")]
    FutureFormat,
    #[error("this is too large to load")]
    TooLarge,
}

/// Envelope so the format version can change without breaking old codes.
#[derive(Serialize, Deserialize)]
struct Envelope {
    format: u32,
    doc: TrackDocument,
}

/// The same, for a kept tape and the name it was kept under.
#[derive(Serialize, Deserialize)]
struct GhostEnvelope {
    format: u32,
    name: String,
    tape: ReplayTape,
}

fn encode_body<T: Serialize>(value: &T, max_raw: usize) -> Result<String, CodeError> {
    let text = ron::ser::to_string(value).map_err(|_| CodeError::Corrupt)?;
    if text.len() > max_raw {
        return Err(CodeError::TooLong);
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(text.as_bytes())
        .map_err(|_| CodeError::Corrupt)?;
    let bytes = enc.finish().map_err(|_| CodeError::Corrupt)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_body<T: DeserializeOwned>(
    prefix: &str,
    code: &str,
    max_code: usize,
    max_raw: usize,
) -> Result<T, CodeError> {
    let body = code.trim();
    if body.len() > max_code {
        return Err(CodeError::TooLong);
    }
    let rest = body.strip_prefix(prefix).ok_or(CodeError::BadPrefix)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(rest)
        .map_err(|_| CodeError::Corrupt)?;

    use std::io::Read;
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(bytes.as_slice())
        .take(max_raw as u64)
        .read_to_end(&mut raw)
        .map_err(|_| CodeError::Corrupt)?;

    ron::from_str(std::str::from_utf8(&raw).map_err(|_| CodeError::Corrupt)?)
        .map_err(|_| CodeError::Corrupt)
}

pub fn export_code(doc: &TrackDocument) -> Result<String, CodeError> {
    let env = Envelope {
        format: crate::FORMAT_VERSION,
        doc: doc.clone(),
    };
    Ok(format!("{PREFIX}{}", encode_body(&env, MAX_RAW)?))
}

pub fn import_code(code: &str) -> Result<TrackDocument, CodeError> {
    let env: Envelope = decode_body(PREFIX, code, MAX_CODE_CHARS, MAX_RAW)?;
    if env.format > crate::FORMAT_VERSION {
        return Err(CodeError::FutureFormat);
    }
    let mut doc = env.doc;
    doc.check_piece_count().map_err(|_| CodeError::TooLarge)?;
    doc.normalize_uids();
    Ok(doc)
}

/// A kept tape as one string: the tape and the name it was saved under.
///
/// The name travels with it because a tape on its own is a file stem and a
/// number, and "ghost_3" is not a thing anybody wants to receive.
pub fn export_ghost_code(name: &str, tape: &ReplayTape) -> Result<String, CodeError> {
    let env = GhostEnvelope {
        format: crate::FORMAT_VERSION,
        name: name.to_string(),
        tape: tape.clone(),
    };
    Ok(format!(
        "{GHOST_PREFIX}{}",
        encode_body(&env, MAX_GHOST_RAW)?
    ))
}

pub fn import_ghost_code(code: &str) -> Result<(String, ReplayTape), CodeError> {
    let env: GhostEnvelope = decode_body(GHOST_PREFIX, code, MAX_GHOST_CODE_CHARS, MAX_GHOST_RAW)?;
    if env.format > crate::FORMAT_VERSION {
        return Err(CodeError::FutureFormat);
    }
    Ok((env.name, env.tape))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_demo_track() {
        let doc = crate::demo_track();
        let code = export_code(&doc).unwrap();
        assert!(code.starts_with(PREFIX));
        let back = import_code(&code).unwrap();
        assert_eq!(back.pieces.len(), doc.pieces.len());
        assert_eq!(back.cell_size, doc.cell_size);
        assert_eq!(
            crate::gameplay_fingerprint(&back),
            crate::gameplay_fingerprint(&doc),
            "a round trip must preserve gameplay identity"
        );
    }

    #[test]
    fn codes_are_url_and_chat_safe() {
        let code = export_code(&crate::demo_track()).unwrap();
        assert!(
            code.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "code must survive being pasted into a URL or chat: {code}"
        );
    }

    #[test]
    fn bad_input_is_rejected_cleanly() {
        assert!(matches!(import_code("nonsense"), Err(CodeError::BadPrefix)));
        assert!(matches!(import_code("PT1_!!!"), Err(CodeError::Corrupt)));
        assert!(matches!(import_code("PT1_"), Err(CodeError::Corrupt)));
        assert!(matches!(
            import_code(&"A".repeat(MAX_CODE_CHARS + 1)),
            Err(CodeError::TooLong)
        ));
    }

    #[test]
    fn whitespace_around_a_pasted_code_is_tolerated() {
        let code = export_code(&crate::demo_track()).unwrap();
        assert!(import_code(&format!("  {code}\n")).is_ok());
    }

    #[test]
    fn a_future_format_is_refused() {
        let env = Envelope {
            format: crate::FORMAT_VERSION + 1,
            doc: TrackDocument::empty(),
        };
        let code = format!("{PREFIX}{}", encode_body(&env, MAX_RAW).unwrap());
        assert!(matches!(import_code(&code), Err(CodeError::FutureFormat)));
    }

    /// A tape with a finish, on the demo track's fingerprints, so it is one a
    /// session would actually have written.
    fn demo_tape() -> ReplayTape {
        let doc = crate::demo_track();
        let mut tape = ReplayTape::new(
            crate::gameplay_fingerprint(&doc),
            crate::gameplay_fingerprint(&doc),
            120,
        );
        for i in 0..240 {
            let steer = if i % 40 < 20 { 0.4 } else { -0.4 };
            tape.push(crate::PackedInput::new(
                steer, 1.0, 0.0, false, false, false,
            ));
        }
        tape.header.finish_tick = 240;
        tape
    }

    #[test]
    fn a_tape_round_trips_with_its_name() {
        let tape = demo_tape();
        let code = export_ghost_code("My ghost", &tape).unwrap();
        assert!(code.starts_with(GHOST_PREFIX));
        let (name, back) = import_ghost_code(&code).unwrap();
        assert_eq!(name, "My ghost");
        assert_eq!(back.tick_len(), tape.tick_len());
        assert_eq!(back.header.finish_tick, tape.header.finish_tick);
        assert_eq!(back.header.track, tape.header.track);
    }

    #[test]
    fn the_two_kinds_of_code_are_not_interchangeable() {
        // Feeding one to the other's reader is the mistake a player makes by
        // pasting the wrong thing, and it has to say so rather than decode a
        // track into the tape reader.
        let track_code = export_code(&crate::demo_track()).unwrap();
        let ghost_code = export_ghost_code("g", &demo_tape()).unwrap();
        assert!(matches!(
            import_ghost_code(&track_code),
            Err(CodeError::BadPrefix)
        ));
        assert!(matches!(
            import_code(&ghost_code),
            Err(CodeError::BadPrefix)
        ));
    }

    #[test]
    fn a_tape_code_is_also_url_and_chat_safe() {
        let code = export_ghost_code("g", &demo_tape()).unwrap();
        assert!(
            code.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "code must survive being pasted into a URL or chat"
        );
    }
}
