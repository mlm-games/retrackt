//! Shareable track codes: `PT1_` + base64url(zlib(ron)).
//!
//! The prefix carries the format version, so an old code is rejected with a
//! clear error instead of being misparsed.

use base64::Engine as _;
use std::io::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::TrackDocument;

const PREFIX: &str = "PT1_";

/// Hard caps so a pasted blob cannot exhaust memory on decode.
const MAX_CODE_CHARS: usize = 256 * 1024;
const MAX_RAW: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CodeError {
    #[error("share code must start with {PREFIX}")]
    BadPrefix,
    #[error("share code is too long")]
    TooLong,
    #[error("share code is truncated or corrupt")]
    Corrupt,
    #[error("this track was made by a newer version of retrackt")]
    FutureFormat,
    #[error("this track is too large to load")]
    TooManyPieces,
}

/// Envelope so the format version can change without breaking old codes.
#[derive(Serialize, Deserialize)]
struct Envelope {
    format: u32,
    doc: TrackDocument,
}

pub fn export_code(doc: &TrackDocument) -> Result<String, CodeError> {
    let env = Envelope {
        format: crate::FORMAT_VERSION,
        doc: doc.clone(),
    };
    let text = ron::ser::to_string(&env).map_err(|_| CodeError::Corrupt)?;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(text.as_bytes())
        .map_err(|_| CodeError::Corrupt)?;
    let bytes = enc.finish().map_err(|_| CodeError::Corrupt)?;
    Ok(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub fn import_code(code: &str) -> Result<TrackDocument, CodeError> {
    let body = code.trim();
    if body.len() > MAX_CODE_CHARS {
        return Err(CodeError::TooLong);
    }
    let rest = body.strip_prefix(PREFIX).ok_or(CodeError::BadPrefix)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(rest)
        .map_err(|_| CodeError::Corrupt)?;

    use std::io::Read;
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(bytes.as_slice())
        .take(MAX_RAW as u64)
        .read_to_end(&mut raw)
        .map_err(|_| CodeError::Corrupt)?;

    let env: Envelope = ron::from_str(std::str::from_utf8(&raw).map_err(|_| CodeError::Corrupt)?)
        .map_err(|_| CodeError::Corrupt)?;

    if env.format > crate::FORMAT_VERSION {
        return Err(CodeError::FutureFormat);
    }
    let mut doc = env.doc;
    doc.check_piece_count().map_err(|_| CodeError::TooManyPieces)?;
    doc.normalize_uids();
    Ok(doc)
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
        let text = ron::ser::to_string(&env).unwrap();
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        {
            use std::io::Write;
            enc.write_all(text.as_bytes()).unwrap();
        }
        let code = format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(enc.finish().unwrap())
        );
        assert!(matches!(import_code(&code), Err(CodeError::FutureFormat)));
    }
}
