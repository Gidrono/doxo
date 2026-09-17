//! Fixture helpers for the word-rs compatibility corpus.

use std::path::{Path, PathBuf};

use ooxml_package::{blank_docx_bytes, OpcPackage, PackageError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

/// Workspace fixtures root (`fixtures/`), walking up from CARGO_MANIFEST_DIR.
pub fn fixtures_root() -> PathBuf {
    // test-fixtures crate lives at crates/test-fixtures
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("../..")
        .join("fixtures")
        .canonicalize()
        .unwrap_or_else(|_| manifest.join("../../fixtures"))
}

pub fn hebrew_fixture() -> PathBuf {
    fixtures_root().join("hebrew/mixed-hebrew-english.docx")
}

pub fn roundtrip_fixture() -> PathBuf {
    fixtures_root().join("roundtrip/simple-formatting.docx")
}

/// Build a minimal Hebrew+English DOCX package in memory.
pub fn make_hebrew_mixed_docx() -> Result<Vec<u8>, FixtureError> {
    let mut pkg = OpcPackage::from_bytes(&blank_docx_bytes())?;
    let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr>
        <w:bidi/>
        <w:jc w:val="right"/>
      </w:pPr>
      <w:r>
        <w:rPr>
          <w:b/>
          <w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>
          <w:sz w:val="28"/>
          <w:rtl/>
          <w:lang w:bidi="he-IL"/>
        </w:rPr>
        <w:t>בדיקת עברית: שלום עולם</w:t>
      </w:r>
      <w:r>
        <w:rPr>
          <w:i/>
          <w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/>
          <w:sz w:val="24"/>
        </w:rPr>
        <w:t xml:space="preserve"> — מסמך בעברית עם English 123</w:t>
      </w:r>
    </w:p>
    <w:p>
      <w:r>
        <w:rPr>
          <w:rFonts w:ascii="Helvetica" w:hAnsi="Helvetica"/>
          <w:sz w:val="22"/>
        </w:rPr>
        <w:t>This sample is intentionally mixed-direction: Hebrew + English + numbers.</w:t>
      </w:r>
    </w:p>
    <w:sectPr>
      <w:pgSz w:w="12240" w:h="15840"/>
      <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>
    </w:sectPr>
  </w:body>
</w:document>"#;
    pkg.set_document_xml(document_xml.as_bytes().to_vec());
    Ok(pkg.write_to_bytes()?)
}

/// Build a simple bold/italic formatting fixture.
pub fn make_simple_formatting_docx() -> Result<Vec<u8>, FixtureError> {
    let mut pkg = OpcPackage::from_bytes(&blank_docx_bytes())?;
    let document_xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr><w:b/><w:sz w:val="32"/></w:rPr>
        <w:t>Bold title</w:t>
      </w:r>
    </w:p>
    <w:p>
      <w:r>
        <w:rPr><w:i/></w:rPr>
        <w:t>Italic body with </w:t>
      </w:r>
      <w:r>
        <w:rPr><w:b/><w:i/></w:rPr>
        <w:t>bold-italic</w:t>
      </w:r>
      <w:r>
        <w:t> text.</w:t>
      </w:r>
    </w:p>
    <w:sectPr>
      <w:pgSz w:w="12240" w:h="15840"/>
      <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>
    </w:sectPr>
  </w:body>
</w:document>"#;
    pkg.set_document_xml(document_xml.to_vec());
    Ok(pkg.write_to_bytes()?)
}

pub fn write_fixture(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), FixtureError> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hebrew_package_opens() {
        let bytes = make_hebrew_mixed_docx().unwrap();
        let pkg = OpcPackage::from_bytes(&bytes).unwrap();
        assert!(pkg.document_xml().unwrap().windows(8).any(|w| w == "שלום".as_bytes()
            || std::str::from_utf8(pkg.document_xml().unwrap()).unwrap().contains("שלום")));
        let xml = std::str::from_utf8(pkg.document_xml().unwrap()).unwrap();
        assert!(xml.contains("שלום"));
        assert!(xml.contains("w:bidi"));
    }
}
