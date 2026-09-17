//! Legacy `.doc` conversion bridge — stub until tables/images/anchors are solid.
//!
//! Planned path (Milestone 3+): invoke LibreOffice
//! (`soffice --headless --convert-to docx`) in a sandboxed subprocess, then open
//! the resulting DOCX through `persistence`.
//!
//! Status: **not implemented**. Callers should treat `.doc` as unsupported and
//! surface [`CompatError::NotImplemented`]. Use `.docx` directly.
//!
//! Detection: [`converter_available`] checks for `soffice` / LibreOffice on PATH
//! (including the macOS app bundle path) so the UI can show a clear message when
//! conversion could be enabled later.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompatError {
    #[error(
        "legacy .doc conversion is not implemented yet — install LibreOffice later; use .docx for now"
    )]
    NotImplemented,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

/// Convert a legacy `.doc` file to a temporary `.docx` path.
///
/// Currently always returns [`CompatError::NotImplemented`]. When implemented,
/// this will shell out to LibreOffice only if [`converter_available`] is true.
pub fn convert_doc_to_docx(_path: &Path) -> Result<PathBuf, CompatError> {
    Err(CompatError::NotImplemented)
}

/// Whether a LibreOffice / soffice binary appears available on PATH.
pub fn converter_available() -> bool {
    which_soffice().is_some()
}

fn which_soffice() -> Option<PathBuf> {
    let candidates = [
        "soffice",
        "libreoffice",
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = Path::new(dir).join("soffice");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_rejects_conversion() {
        let err = convert_doc_to_docx(Path::new("/tmp/x.doc")).unwrap_err();
        assert!(matches!(err, CompatError::NotImplemented));
    }
}
