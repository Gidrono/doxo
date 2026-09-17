//! Open/save orchestration for DOCX documents.

use std::path::{Path, PathBuf};

use document_model::{Block, Document, InlineImage, Run, RunContent};
use ooxml_package::{blank_docx_bytes, OpcPackage, PackageError, DOCUMENT_PATH};
use thiserror::Error;
use wordprocessingml::{
    extract_hf_plain_text, merge_hf_xml, parse_document_xml, serialize_document_xml, WmlError,
};

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error(transparent)]
    Wml(#[from] WmlError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// An open document session: semantic model + package for part preservation.
#[derive(Debug)]
pub struct DocumentSession {
    pub document: Document,
    pub package: OpcPackage,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// Original `word/document.xml` bytes at open time (for loss-preservation audits).
    pub original_document_xml: Option<Vec<u8>>,
    /// Names of package parts other than document.xml (preserved on save).
    pub preserved_part_names: Vec<String>,
}

impl DocumentSession {
    fn from_package(package: OpcPackage, path: Option<PathBuf>) -> Result<Self, PersistenceError> {
        let original_document_xml = package.document_xml().ok().map(|b| b.to_vec());
        let preserved_part_names: Vec<String> = package
            .part_names()
            .filter(|n| *n != DOCUMENT_PATH)
            .map(|s| s.to_string())
            .collect();
        let mut document = parse_document_xml(package.document_xml()?)?;
        hydrate_images(&mut document, &package);
        hydrate_headers_footers(&mut document, &package);
        Ok(Self {
            document,
            package,
            path,
            dirty: false,
            original_document_xml,
            preserved_part_names,
        })
    }

    pub fn new_blank() -> Result<Self, PersistenceError> {
        let package = OpcPackage::from_bytes(&blank_docx_bytes())?;
        Self::from_package(package, None)
    }

    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let path = path.as_ref();
        let package = OpcPackage::open_path(path)?;
        Self::from_package(package, Some(path.to_path_buf()))
    }

    pub fn open_bytes(bytes: &[u8]) -> Result<Self, PersistenceError> {
        let package = OpcPackage::from_bytes(bytes)?;
        Self::from_package(package, None)
    }

    /// Rewrite `word/document.xml` and sync image / header / footer parts.
    pub fn save_to_path(&mut self, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
        sync_images_to_package(&mut self.document, &mut self.package);
        sync_headers_footers(&mut self.document, &mut self.package)?;
        let xml = serialize_document_xml(&self.document)?;
        self.package.set_document_xml(xml.into_bytes());
        self.package.write_to_path(path.as_ref())?;
        self.path = Some(path.as_ref().to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// True when every non-document part from open is still present.
    pub fn preserved_parts_intact(&self) -> bool {
        self.preserved_part_names
            .iter()
            .all(|name| self.package.get_part(name).is_some())
    }

    pub fn save(&mut self) -> Result<(), PersistenceError> {
        let path = self.path.clone().ok_or_else(|| {
            PersistenceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "no path; use save_to_path",
            ))
        })?;
        self.save_to_path(path)
    }

    pub fn to_bytes(&mut self) -> Result<Vec<u8>, PersistenceError> {
        sync_images_to_package(&mut self.document, &mut self.package);
        sync_headers_footers(&mut self.document, &mut self.package)?;
        let xml = serialize_document_xml(&self.document)?;
        self.package.set_document_xml(xml.into_bytes());
        Ok(self.package.write_to_bytes()?)
    }
}

/// Atomic-ish save helper: write to `path.tmp` then rename.
pub fn atomic_save(
    session: &mut DocumentSession,
    path: impl AsRef<Path>,
) -> Result<(), PersistenceError> {
    let path = path.as_ref();
    let tmp = path.with_extension("docx.tmp");
    session.save_to_path(&tmp)?;
    std::fs::rename(&tmp, path)?;
    session.path = Some(path.to_path_buf());
    Ok(())
}

fn hydrate_images(doc: &mut Document, package: &OpcPackage) {
    for section in &mut doc.sections {
        for block in &mut section.blocks {
            match block {
                Block::Paragraph(p) => hydrate_runs(&mut p.runs, package),
                Block::Table(t) => {
                    for row in &mut t.rows {
                        for cell in &mut row.cells {
                            for p in &mut cell.paragraphs {
                                hydrate_runs(&mut p.runs, package);
                            }
                        }
                    }
                }
                Block::Unsupported(_) => {}
            }
        }
    }
}

fn hydrate_headers_footers(doc: &mut Document, package: &OpcPackage) {
    for section in &mut doc.sections {
        if let Some(rid) = section.header_r_id.clone() {
            if let Some(path) = package.resolve_document_relationship(&rid) {
                if let Some(bytes) = package.get_part(&path) {
                    let text = extract_hf_plain_text(bytes);
                    section.header_source_xml = Some(String::from_utf8_lossy(bytes).into_owned());
                    if section.header.is_none() || section.header.as_deref() == Some("") {
                        section.header = Some(text);
                    } else if section.header.as_deref() != Some(text.as_str()) {
                        // Prefer package part text as source of truth on open.
                        section.header = Some(text);
                    }
                }
            }
        }
        if let Some(rid) = section.footer_r_id.clone() {
            if let Some(path) = package.resolve_document_relationship(&rid) {
                if let Some(bytes) = package.get_part(&path) {
                    let text = extract_hf_plain_text(bytes);
                    section.footer_source_xml = Some(String::from_utf8_lossy(bytes).into_owned());
                    section.footer = Some(text);
                }
            }
        }
    }
}

fn sync_headers_footers(
    doc: &mut Document,
    package: &mut OpcPackage,
) -> Result<(), PersistenceError> {
    for section in &mut doc.sections {
        if let Some(text) = section.header.clone() {
            let xml = merge_hf_xml(section.header_source_xml.as_deref(), &text, true)?;
            let rid =
                package.ensure_header_part(section.header_r_id.as_deref(), xml.clone().into_bytes());
            section.header_r_id = Some(rid);
            section.header_source_xml = Some(xml);
        }
        if let Some(text) = section.footer.clone() {
            let xml = merge_hf_xml(section.footer_source_xml.as_deref(), &text, false)?;
            let rid =
                package.ensure_footer_part(section.footer_r_id.as_deref(), xml.clone().into_bytes());
            section.footer_r_id = Some(rid);
            section.footer_source_xml = Some(xml);
        }
    }
    Ok(())
}

fn hydrate_runs(runs: &mut [Run], package: &OpcPackage) {
    for run in runs {
        if let RunContent::Image(img) = &mut run.content {
            hydrate_image(img, package);
        }
    }
}

fn hydrate_image(img: &mut InlineImage, package: &OpcPackage) {
    let Some(rid) = img.relationship_id.as_deref() else {
        return;
    };
    let Some(part_path) = package.resolve_document_relationship(rid) else {
        return;
    };
    if let Some(bytes) = package.get_part(&part_path) {
        img.data = bytes.to_vec();
        if let Some(ct) = guess_content_type(&part_path) {
            img.content_type = ct.into();
        }
    }
}

fn sync_images_to_package(doc: &mut Document, package: &mut OpcPackage) {
    let mut image_counter = 0u32;
    for section in &mut doc.sections {
        for block in &mut section.blocks {
            match block {
                Block::Paragraph(p) => {
                    sync_runs(&mut p.runs, package, &mut image_counter);
                }
                Block::Table(t) => {
                    for row in &mut t.rows {
                        for cell in &mut row.cells {
                            for p in &mut cell.paragraphs {
                                sync_runs(&mut p.runs, package, &mut image_counter);
                            }
                        }
                    }
                }
                Block::Unsupported(_) => {}
            }
        }
    }
}

fn sync_runs(runs: &mut [Run], package: &mut OpcPackage, counter: &mut u32) {
    for run in runs {
        if let RunContent::Image(img) = &mut run.content {
            if img.data.is_empty() {
                continue;
            }
            let needs_new = match img.relationship_id.as_deref() {
                None => true,
                Some(rid) => package.resolve_document_relationship(rid).is_none(),
            };
            if needs_new {
                *counter += 1;
                let ext = extension_for_content_type(&img.content_type);
                let filename = format!("image{counter}.{ext}");
                let rid =
                    package.add_image_part(&filename, img.data.clone(), &img.content_type);
                img.relationship_id = Some(rid);
            } else if let Some(rid) = img.relationship_id.as_deref() {
                if let Some(part_path) = package.resolve_document_relationship(rid) {
                    package.set_part(&part_path, img.data.clone());
                }
            }
        }
    }
}

fn guess_content_type(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        Some("image/png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if lower.ends_with(".gif") {
        Some("image/gif")
    } else if lower.ends_with(".emf") {
        Some("image/x-emf")
    } else {
        None
    }
}

fn extension_for_content_type(ct: &str) -> &'static str {
    match ct {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_model::{Block, Paragraph, Run, RunProperties, Table};

    #[test]
    fn open_blank_and_round_trip() {
        let mut session = DocumentSession::new_blank().unwrap();
        assert_eq!(session.document.plain_text(), "");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.docx");
        session.save_to_path(&path).unwrap();
        let again = DocumentSession::open_path(&path).unwrap();
        assert_eq!(again.document.plain_text(), "");
    }

    #[test]
    fn hebrew_edit_round_trip() {
        let mut session = DocumentSession::new_blank().unwrap();
        let mut p = Paragraph::from_text("");
        p.properties.bidirectional = true;
        p.runs = vec![
            {
                let mut r = Run::text("שלום עולם");
                r.properties = RunProperties {
                    bold: true,
                    rtl: true,
                    font_ascii: Some("Arial".into()),
                    font_cs: Some("Arial".into()),
                    ..Default::default()
                };
                r
            },
            Run::text(" — Hello 123"),
        ];
        session.document.sections[0].blocks = vec![Block::Paragraph(p)];
        session.dirty = true;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hebrew.docx");
        session.save_to_path(&path).unwrap();

        let again = DocumentSession::open_path(&path).unwrap();
        assert_eq!(again.document.plain_text(), "שלום עולם — Hello 123");
        let Block::Paragraph(p2) = &again.document.sections[0].blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(p2.properties.bidirectional);
        assert!(p2.runs[0].properties.bold);
        assert!(p2.runs[0].properties.rtl);
        assert!(again.package.get_part("word/styles.xml").is_some());
    }

    #[test]
    fn non_document_parts_preserved_after_save() {
        let mut session = DocumentSession::new_blank().unwrap();
        assert!(!session.preserved_part_names.is_empty());
        let styles_before = session
            .package
            .get_part("word/styles.xml")
            .unwrap()
            .to_vec();
        session.document.sections[0].blocks =
            vec![Block::Paragraph(Paragraph::from_text("edit"))];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preserve.docx");
        session.save_to_path(&path).unwrap();
        let again = DocumentSession::open_path(&path).unwrap();
        assert_eq!(
            again.package.get_part("word/styles.xml").unwrap(),
            styles_before.as_slice()
        );
        assert!(again.package.get_part("word/styles.xml").is_some());
    }

    #[test]
    fn edited_table_round_trips_as_w_tbl() {
        let mut session = DocumentSession::new_blank().unwrap();
        let table = Table::empty(2, 2);
        session.document.sections[0].blocks = vec![
            Block::Paragraph(Paragraph::from_text("before")),
            Block::Table(table),
            Block::Paragraph(Paragraph::from_text("after")),
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.docx");
        session.save_to_path(&path).unwrap();
        let again = DocumentSession::open_path(&path).unwrap();
        assert!(matches!(
            again.document.sections[0].blocks[1],
            Block::Table(_)
        ));
        let xml = String::from_utf8_lossy(again.package.document_xml().unwrap());
        assert!(xml.contains("<w:tbl>"));
        assert!(xml.contains("<w:tc>"));
    }

    #[test]
    fn image_embedded_in_package_on_save() {
        // Minimal 1x1 PNG
        let png: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE,
            0xD4, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let mut session = DocumentSession::new_blank().unwrap();
        let mut p = Paragraph::empty();
        p.runs = vec![Run {
            id: document_model::NodeId::new(),
            properties: RunProperties::default(),
            content: RunContent::Image(InlineImage {
                content_type: "image/png".into(),
                data: png.clone(),
                width_px: Some(1),
                height_px: Some(1),
                relationship_id: None,
            }),
            source_xml: None,
            unknown_r_pr: Vec::new(),
        }];
        session.document.sections[0].blocks = vec![Block::Paragraph(p)];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.docx");
        session.save_to_path(&path).unwrap();

        let again = DocumentSession::open_path(&path).unwrap();
        let media: Vec<_> = again
            .package
            .part_names()
            .filter(|n| n.starts_with("word/media/"))
            .collect();
        assert!(!media.is_empty(), "expected media part");
        let Block::Paragraph(p2) = &again.document.sections[0].blocks[0] else {
            panic!("expected paragraph");
        };
        match &p2.runs[0].content {
            RunContent::Image(img) => {
                assert!(!img.data.is_empty());
                assert!(img.relationship_id.is_some());
            }
            other => panic!("expected image, got {other:?}"),
        }
        let xml = String::from_utf8_lossy(again.package.document_xml().unwrap());
        assert!(xml.contains("w:drawing") || xml.contains("a:blip"));
    }

    #[test]
    fn unknown_p_pr_round_trips() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr>
        <w:bidi/>
        <w:keepNext/>
        <w:outlineLvl w:val="0"/>
      </w:pPr>
      <w:r><w:t>x</w:t></w:r>
    </w:p>
    <w:sectPr/>
  </w:body>
</w:document>"#;
        let mut session = DocumentSession::new_blank().unwrap();
        session.document = wordprocessingml::parse_document_xml(xml).unwrap();
        let Block::Paragraph(p) = &session.document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        assert!(!p.unknown_p_pr.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unk.docx");
        session.save_to_path(&path).unwrap();
        let again = DocumentSession::open_path(&path).unwrap();
        let Block::Paragraph(p2) = &again.document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        assert!(!p2.unknown_p_pr.is_empty());
        let out = String::from_utf8_lossy(again.package.document_xml().unwrap());
        assert!(out.contains("keepNext") || out.contains("outlineLvl"));
    }

    #[test]
    fn header_footer_parts_round_trip() {
        let mut session = DocumentSession::new_blank().unwrap();
        session.document.sections[0].header = Some("Doc Header".into());
        session.document.sections[0].footer = Some("Doc Footer".into());
        session.document.sections[0].blocks =
            vec![Block::Paragraph(Paragraph::from_text("body"))];

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hf.docx");
        session.save_to_path(&path).unwrap();

        let again = DocumentSession::open_path(&path).unwrap();
        assert_eq!(again.document.sections[0].header.as_deref(), Some("Doc Header"));
        assert_eq!(again.document.sections[0].footer.as_deref(), Some("Doc Footer"));
        assert!(again.document.sections[0].header_r_id.is_some());
        assert!(again.document.sections[0].footer_r_id.is_some());
        assert!(again.package.get_part("word/header1.xml").is_some());
        assert!(again.package.get_part("word/footer1.xml").is_some());

        let doc_xml = String::from_utf8_lossy(again.package.document_xml().unwrap());
        assert!(doc_xml.contains("w:headerReference"));
        assert!(doc_xml.contains("w:footerReference"));

        let ct = String::from_utf8_lossy(again.package.get_part("[Content_Types].xml").unwrap());
        assert!(ct.contains("header+xml"));
        assert!(ct.contains("footer+xml"));

        let rels = String::from_utf8_lossy(
            again
                .package
                .get_part("word/_rels/document.xml.rels")
                .unwrap(),
        );
        assert!(rels.contains("/relationships/header"));
        assert!(rels.contains("/relationships/footer"));
    }

    #[test]
    fn header_preserves_unknown_markup_when_text_unchanged() {
        let mut session = DocumentSession::new_blank().unwrap();
        let rich = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:p>
    <w:pPr><w:pStyle w:val="Header"/><w:keepNext/></w:pPr>
    <w:r><w:t>KeepMe</w:t></w:r>
  </w:p>
</w:hdr>"#;
        let rid = session
            .package
            .ensure_header_part(None, rich.as_bytes().to_vec());
        session.document.sections[0].header = Some("KeepMe".into());
        session.document.sections[0].header_r_id = Some(rid);
        session.document.sections[0].header_source_xml = Some(rich.into());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hf-preserve.docx");
        session.save_to_path(&path).unwrap();
        let again = DocumentSession::open_path(&path).unwrap();
        assert_eq!(again.document.sections[0].header.as_deref(), Some("KeepMe"));
        let part = again.package.get_part("word/header1.xml").unwrap();
        let s = String::from_utf8_lossy(part);
        assert!(s.contains("keepNext"));
        assert!(s.contains("pStyle"));
    }

    #[test]
    fn header_text_edit_updates_part() {
        let mut session = DocumentSession::new_blank().unwrap();
        session.document.sections[0].header = Some("Before".into());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hf-edit.docx");
        session.save_to_path(&path).unwrap();

        let mut again = DocumentSession::open_path(&path).unwrap();
        again.document.sections[0].header = Some("After".into());
        again.save_to_path(&path).unwrap();

        let third = DocumentSession::open_path(&path).unwrap();
        assert_eq!(third.document.sections[0].header.as_deref(), Some("After"));
        let part = String::from_utf8_lossy(third.package.get_part("word/header1.xml").unwrap());
        assert!(part.contains("After"));
        assert!(!part.contains("Before"));
    }
}
