//! OPC/ZIP package handling with part-preserving round-trip.
//!
//! Open the package into an in-memory part map. Unsupported / untouched parts
//! are kept byte-identical when writing back.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const CONTENT_TYPES_PATH: &str = "[Content_Types].xml";
pub const RELS_PATH: &str = "_rels/.rels";
pub const DOCUMENT_PATH: &str = "word/document.xml";
pub const DOCUMENT_RELS_PATH: &str = "word/_rels/document.xml.rels";

pub const REL_TYPE_HEADER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
pub const REL_TYPE_FOOTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
pub const CONTENT_TYPE_HEADER: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
pub const CONTENT_TYPE_FOOTER: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("missing required part: {0}")]
    MissingPart(String),
    #[error("package too large or decompression limit exceeded")]
    SizeLimit,
}

/// Soft limits for hostile ZIP/XML input (Milestone 0 baseline).
const MAX_UNCOMPRESSED_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_PART_SIZE: u64 = 64 * 1024 * 1024;

/// In-memory OPC package: path → raw bytes (preserves order via BTreeMap keys
/// plus an insertion-order list for write-back fidelity).
#[derive(Debug, Clone, Default)]
pub struct OpcPackage {
    /// Part path (forward slashes) → bytes.
    parts: BTreeMap<String, Vec<u8>>,
    /// Original entry order from the ZIP for stable rewrite.
    order: Vec<String>,
}

impl OpcPackage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        let file = File::open(path)?;
        Self::from_reader(file)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PackageError> {
        Self::from_reader(Cursor::new(bytes))
    }

    pub fn from_reader<R: Read + std::io::Seek>(reader: R) -> Result<Self, PackageError> {
        let mut archive = ZipArchive::new(reader)?;
        let mut package = Self::new();
        let mut total: u64 = 0;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().replace('\\', "/");
            if name.ends_with('/') {
                continue;
            }
            let size = file.size();
            if size > MAX_PART_SIZE {
                return Err(PackageError::SizeLimit);
            }
            total = total.saturating_add(size);
            if total > MAX_UNCOMPRESSED_TOTAL {
                return Err(PackageError::SizeLimit);
            }
            let mut data = Vec::with_capacity(size as usize);
            file.read_to_end(&mut data)?;
            package.insert_part(name, data);
        }
        Ok(package)
    }

    pub fn insert_part(&mut self, path: impl Into<String>, data: Vec<u8>) {
        let path = normalize_part_path(path.into());
        if !self.parts.contains_key(&path) {
            self.order.push(path.clone());
        }
        self.parts.insert(path, data);
    }

    pub fn get_part(&self, path: &str) -> Option<&[u8]> {
        self.parts.get(&normalize_part_path(path)).map(|v| v.as_slice())
    }

    pub fn take_part(&mut self, path: &str) -> Option<Vec<u8>> {
        let path = normalize_part_path(path);
        self.parts.remove(&path)
    }

    pub fn set_part(&mut self, path: impl Into<String>, data: Vec<u8>) {
        self.insert_part(path, data);
    }

    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(|s| s.as_str())
    }

    pub fn document_xml(&self) -> Result<&[u8], PackageError> {
        self.get_part(DOCUMENT_PATH)
            .ok_or_else(|| PackageError::MissingPart(DOCUMENT_PATH.to_string()))
    }

    pub fn set_document_xml(&mut self, data: Vec<u8>) {
        self.set_part(DOCUMENT_PATH, data);
    }

    pub fn write_to_path(&self, path: impl AsRef<Path>) -> Result<(), PackageError> {
        let mut file = File::create(path)?;
        self.write_to(&mut file)?;
        file.sync_all()?;
        Ok(())
    }

    pub fn write_to_bytes(&self) -> Result<Vec<u8>, PackageError> {
        let mut buf = Cursor::new(Vec::new());
        self.write_to(&mut buf)?;
        Ok(buf.into_inner())
    }

    pub fn write_to<W: Write + std::io::Seek>(&self, writer: W) -> Result<(), PackageError> {
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);

        for name in &self.order {
            if let Some(data) = self.parts.get(name) {
                zip.start_file(name.as_str(), options)?;
                zip.write_all(data)?;
            }
        }
        // Any parts inserted after open that weren't in order
        for (name, data) in &self.parts {
            if !self.order.contains(name) {
                zip.start_file(name.as_str(), options)?;
                zip.write_all(data)?;
            }
        }
        zip.finish()?;
        Ok(())
    }

    /// True if every part name/bytes matches `other` (order-insensitive).
    pub fn parts_equal(&self, other: &Self) -> bool {
        self.parts == other.parts
    }

    /// Next available relationship id like `rId5` based on existing document rels.
    pub fn next_relationship_id(&self) -> String {
        let mut max = 0u32;
        if let Some(rels) = self.get_part(DOCUMENT_RELS_PATH) {
            let s = String::from_utf8_lossy(rels);
            let mut search: &str = &*s;
            while let Some(idx) = search.find("Id=\"rId") {
                let rest = &search[idx + 7..];
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<u32>() {
                    max = max.max(n);
                }
                search = if digits.is_empty() {
                    &rest[1.min(rest.len())..]
                } else {
                    &rest[digits.len()..]
                };
            }
        }
        format!("rId{}", max + 1)
    }

    /// Resolve a relationship Id in `word/_rels/document.xml.rels` to a part path.
    pub fn resolve_document_relationship(&self, rid: &str) -> Option<String> {
        let rels = self.get_part(DOCUMENT_RELS_PATH)?;
        let s = String::from_utf8_lossy(rels);
        // <Relationship Id="rId5" ... Target="media/image1.png"/>
        let needle = format!("Id=\"{rid}\"");
        let idx = s.find(&needle)?;
        let after = &s[idx..];
        let tkey = "Target=\"";
        let tidx = after.find(tkey)?;
        let rest = &after[tidx + tkey.len()..];
        let end = rest.find('"')?;
        let target = &rest[..end];
        if target.starts_with('/') {
            Some(target.trim_start_matches('/').to_string())
        } else {
            Some(format!("word/{target}"))
        }
    }

    /// Add an image part under `word/media/` and a document relationship. Returns the rId.
    pub fn add_image_part(
        &mut self,
        filename: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> String {
        let path = format!("word/media/{filename}");
        self.set_part(&path, data);
        let rid = self.next_relationship_id();
        self.upsert_document_relationship(
            &rid,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            &format!("media/{filename}"),
        );
        self.ensure_override_content_type(&format!("/{path}"), content_type);
        if content_type == "image/png" {
            self.ensure_default_content_type("png", content_type);
        } else if content_type == "image/jpeg" {
            self.ensure_default_content_type("jpeg", content_type);
            self.ensure_default_content_type("jpg", content_type);
        } else if content_type == "image/gif" {
            self.ensure_default_content_type("gif", content_type);
        }
        rid
    }

    /// Ensure a header part exists; returns the relationship Id used in `sectPr`.
    pub fn ensure_header_part(&mut self, preferred_rid: Option<&str>, xml: Vec<u8>) -> String {
        self.ensure_hf_part(
            preferred_rid,
            REL_TYPE_HEADER,
            CONTENT_TYPE_HEADER,
            "header1.xml",
            xml,
        )
    }

    /// Ensure a footer part exists; returns the relationship Id used in `sectPr`.
    pub fn ensure_footer_part(&mut self, preferred_rid: Option<&str>, xml: Vec<u8>) -> String {
        self.ensure_hf_part(
            preferred_rid,
            REL_TYPE_FOOTER,
            CONTENT_TYPE_FOOTER,
            "footer1.xml",
            xml,
        )
    }

    fn ensure_hf_part(
        &mut self,
        preferred_rid: Option<&str>,
        rel_type: &str,
        content_type: &str,
        default_filename: &str,
        xml: Vec<u8>,
    ) -> String {
        if let Some(rid) = preferred_rid {
            if let Some(path) = self.resolve_document_relationship(rid) {
                self.set_part(&path, xml);
                self.ensure_override_content_type(&format!("/{path}"), content_type);
                return rid.to_string();
            }
        }
        if let Some((rid, path)) = self.find_document_relationship_by_type(rel_type) {
            self.set_part(&path, xml);
            self.ensure_override_content_type(&format!("/{path}"), content_type);
            return rid;
        }
        let path = format!("word/{default_filename}");
        self.set_part(&path, xml);
        let rid = self.next_relationship_id();
        self.upsert_document_relationship(&rid, rel_type, default_filename);
        self.ensure_override_content_type(&format!("/{path}"), content_type);
        rid
    }

    /// Find first document relationship of `rel_type`; returns (rId, part path).
    pub fn find_document_relationship_by_type(&self, rel_type: &str) -> Option<(String, String)> {
        let rels = self.get_part(DOCUMENT_RELS_PATH)?;
        let s = String::from_utf8_lossy(rels);
        let type_needle = format!("Type=\"{rel_type}\"");
        let mut search: &str = &*s;
        while let Some(idx) = search.find(&type_needle) {
            let window_start = search[..idx].rfind('<').unwrap_or(0);
            let window = &search[window_start..];
            let end = match window.find("/>").or_else(|| window.find('>')) {
                Some(e) => e,
                None => {
                    search = &search[idx + type_needle.len()..];
                    continue;
                }
            };
            let elem = &window[..end];
            let Some(rid) = attr_in_fragment(elem, "Id") else {
                search = &search[idx + type_needle.len()..];
                continue;
            };
            let Some(target) = attr_in_fragment(elem, "Target") else {
                search = &search[idx + type_needle.len()..];
                continue;
            };
            let path = if target.starts_with('/') {
                target.trim_start_matches('/').to_string()
            } else {
                format!("word/{target}")
            };
            return Some((rid, path));
        }
        None
    }

    fn upsert_document_relationship(&mut self, rid: &str, rel_type: &str, target: &str) {
        let existing = self
            .get_part(DOCUMENT_RELS_PATH)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| {
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
</Relationships>"#
                    .to_string()
            });
        let entry = format!(
            r#"<Relationship Id="{rid}" Type="{rel_type}" Target="{target}"/>"#
        );
        let id_needle = format!("Id=\"{rid}\"");
        let updated = if let Some(start) = existing.find(&id_needle) {
            // Replace the entire Relationship element containing this Id.
            let elem_start = existing[..start].rfind('<').unwrap_or(0);
            let after = &existing[start..];
            let rel_end = after
                .find("/>")
                .map(|i| start + i + 2)
                .or_else(|| after.find('>').map(|i| start + i + 1))
                .unwrap_or(existing.len());
            let mut s = String::with_capacity(existing.len() + entry.len());
            s.push_str(&existing[..elem_start]);
            s.push_str(&entry);
            s.push_str(&existing[rel_end..]);
            s
        } else {
            existing.replacen(
                "</Relationships>",
                &format!("  {entry}\n</Relationships>"),
                1,
            )
        };
        self.set_part(DOCUMENT_RELS_PATH, updated.into_bytes());
    }

    fn ensure_default_content_type(&mut self, extension: &str, content_type: &str) {
        let Some(ct) = self.get_part(CONTENT_TYPES_PATH) else {
            return;
        };
        let mut s = String::from_utf8_lossy(ct).into_owned();
        let needle = format!("Extension=\"{extension}\"");
        if s.contains(&needle) {
            return;
        }
        let entry = format!(
            r#"<Default Extension="{extension}" ContentType="{content_type}"/>"#
        );
        s = s.replacen("</Types>", &format!("  {entry}\n</Types>"), 1);
        self.set_part(CONTENT_TYPES_PATH, s.into_bytes());
    }

    fn ensure_override_content_type(&mut self, part_name: &str, content_type: &str) {
        let Some(ct) = self.get_part(CONTENT_TYPES_PATH) else {
            return;
        };
        let mut s = String::from_utf8_lossy(ct).into_owned();
        if s.contains(&format!("PartName=\"{part_name}\"")) {
            return;
        }
        let entry = format!(
            r#"<Override PartName="{part_name}" ContentType="{content_type}"/>"#
        );
        s = s.replacen("</Types>", &format!("  {entry}\n</Types>"), 1);
        self.set_part(CONTENT_TYPES_PATH, s.into_bytes());
    }
}

fn normalize_part_path(path: impl AsRef<str>) -> String {
    path.as_ref().trim_start_matches('/').replace('\\', "/")
}

fn attr_in_fragment(elem: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let i = elem.find(&needle)?;
    let rest = &elem[i + needle.len()..];
    let e = rest.find('"')?;
    Some(rest[..e].to_string())
}

/// Minimal well-formed blank DOCX package bytes (for tests / new documents).
pub fn blank_docx_bytes() -> Vec<u8> {
    let mut pkg = OpcPackage::new();
    pkg.set_part(
        CONTENT_TYPES_PATH,
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#.to_vec(),
    );
    pkg.set_part(
        RELS_PATH,
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#.to_vec(),
    );
    pkg.set_part(
        DOCUMENT_PATH,
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:t></w:t>
      </w:r>
    </w:p>
    <w:sectPr>
      <w:pgSz w:w="12240" w:h="15840"/>
      <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>
    </w:sectPr>
  </w:body>
</w:document>"#.to_vec(),
    );
    pkg.set_part(
        DOCUMENT_RELS_PATH,
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#.to_vec(),
    );
    pkg.set_part(
        "word/styles.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
    <w:qFormat/>
  </w:style>
</w:styles>"#.to_vec(),
    );
    pkg.write_to_bytes().expect("blank docx zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_round_trip_preserves_parts() {
        let bytes = blank_docx_bytes();
        let pkg = OpcPackage::from_bytes(&bytes).unwrap();
        assert!(pkg.get_part(DOCUMENT_PATH).is_some());
        assert!(pkg.get_part(CONTENT_TYPES_PATH).is_some());

        let out = pkg.write_to_bytes().unwrap();
        let pkg2 = OpcPackage::from_bytes(&out).unwrap();
        assert!(pkg.parts_equal(&pkg2));
    }

    #[test]
    fn untouched_part_bytes_identical() {
        let bytes = blank_docx_bytes();
        let mut pkg = OpcPackage::from_bytes(&bytes).unwrap();
        let styles_before = pkg.get_part("word/styles.xml").unwrap().to_vec();
        // Only rewrite document.xml
        pkg.set_document_xml(pkg.document_xml().unwrap().to_vec());
        let out = pkg.write_to_bytes().unwrap();
        let pkg2 = OpcPackage::from_bytes(&out).unwrap();
        assert_eq!(
            pkg2.get_part("word/styles.xml").unwrap(),
            styles_before.as_slice()
        );
    }
}
