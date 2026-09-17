//! WordprocessingML subset parser/serializer for `word/document.xml`.
//!
//! Supports paragraphs, runs, text, bold/italic/underline, fonts, size, RTL/bidi,
//! alignment, tables, images, header/footer parts, and opaque unknown anchors.

mod header_footer;
mod parse;
mod serialize;

pub use header_footer::{
    extract_hf_plain_text, merge_hf_xml, serialize_footer_xml, serialize_header_xml,
};
pub use parse::parse_document_xml;
pub use serialize::serialize_document_xml;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WmlError {
    #[error("xml error: {0}")]
    Xml(String),
    #[error("unexpected structure: {0}")]
    Structure(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use document_model::{Alignment, Block, Document, Paragraph, Run, RunContent, RunProperties};

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
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
          <w:i/>
          <w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>
          <w:sz w:val="24"/>
          <w:rtl/>
        </w:rPr>
        <w:t>שלום</w:t>
      </w:r>
      <w:r>
        <w:rPr>
          <w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman"/>
        </w:rPr>
        <w:t xml:space="preserve"> Hello 123</w:t>
      </w:r>
    </w:p>
    <w:sectPr>
      <w:pgSz w:w="12240" w:h="15840"/>
      <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>
    </w:sectPr>
  </w:body>
</w:document>"#;

    #[test]
    fn parse_hebrew_mixed_runs() {
        let doc = parse_document_xml(SAMPLE.as_bytes()).unwrap();
        assert_eq!(doc.sections.len(), 1);
        let Block::Paragraph(p) = &doc.sections[0].blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(p.properties.bidirectional);
        assert_eq!(p.properties.alignment, Some(Alignment::Right));
        assert_eq!(p.runs.len(), 2);
        assert!(matches!(&p.runs[0].content, RunContent::Text(t) if t == "שלום"));
        assert!(p.runs[0].properties.bold);
        assert!(p.runs[0].properties.italic);
        assert!(p.runs[0].properties.rtl);
        assert_eq!(
            p.runs[0].properties.font_ascii.as_deref(),
            Some("Arial")
        );
        assert!(matches!(&p.runs[1].content, RunContent::Text(t) if t == " Hello 123"));
        assert_eq!(doc.plain_text(), "שלום Hello 123");
    }

    #[test]
    fn serialize_round_trip_preserves_semantics() {
        let doc = parse_document_xml(SAMPLE.as_bytes()).unwrap();
        let xml = serialize_document_xml(&doc).unwrap();
        let doc2 = parse_document_xml(xml.as_bytes()).unwrap();
        assert_eq!(doc.plain_text(), doc2.plain_text());
        let Block::Paragraph(p) = &doc2.sections[0].blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(p.properties.bidirectional);
        assert!(p.runs[0].properties.bold);
        assert!(p.runs[0].properties.rtl);
    }

    #[test]
    fn build_and_serialize_programmatic_doc() {
        let mut doc = Document::blank();
        let mut p = Paragraph::empty();
        p.properties.bidirectional = true;
        p.runs = vec![
            {
                let mut r = Run::text("עברית");
                r.properties = RunProperties {
                    bold: true,
                    rtl: true,
                    font_cs: Some("David".into()),
                    font_ascii: Some("David".into()),
                    font_size_half_points: Some(28),
                    ..Default::default()
                };
                r
            },
            Run::text(" and English"),
        ];
        doc.sections[0].blocks = vec![Block::Paragraph(p)];
        let xml = serialize_document_xml(&doc).unwrap();
        assert!(xml.contains("w:bidi"));
        assert!(xml.contains("w:rtl"));
        assert!(xml.contains("עברית"));
        let again = parse_document_xml(xml.as_bytes()).unwrap();
        assert_eq!(again.plain_text(), "עברית and English");
    }

    #[test]
    fn serialize_edited_table() {
        use document_model::{Table, TableCell, TableRow};
        let mut doc = Document::blank();
        let table = Table {
            id: document_model::NodeId::new(),
            rows: vec![TableRow {
                cells: vec![
                    TableCell::from_text("A"),
                    TableCell::from_text("B"),
                ],
            }],
            source_xml: None,
            edited: true,
        };
        doc.sections[0].blocks = vec![Block::Table(table)];
        let xml = serialize_document_xml(&doc).unwrap();
        assert!(xml.contains("<w:tbl>"));
        assert!(xml.contains(">A</w:t>"));
        let again = parse_document_xml(xml.as_bytes()).unwrap();
        assert!(matches!(again.sections[0].blocks[0], Block::Table(_)));
    }

    #[test]
    fn multi_paragraph_enter_simulation_round_trip() {
        // Simulates the document model after the user presses Return between lines.
        let mut doc = Document::blank();
        doc.sections[0].blocks = vec![
            Block::Paragraph(Paragraph::from_text("First line")),
            Block::Paragraph(Paragraph::empty()),
            Block::Paragraph(Paragraph::from_text("שלום after Enter")),
            Block::Paragraph(Paragraph::from_text("Third")),
        ];
        let xml = serialize_document_xml(&doc).unwrap();
        assert!(xml.matches("<w:p>").count() >= 4 || xml.matches("<w:p ").count() + xml.matches("<w:p>").count() >= 4);
        let again = parse_document_xml(xml.as_bytes()).unwrap();
        assert_eq!(again.sections[0].blocks.len(), 4);
        assert_eq!(
            again.document_plain_paras(),
            vec![
                "First line".to_string(),
                "".to_string(),
                "שלום after Enter".to_string(),
                "Third".to_string()
            ]
        );
    }

    trait PlainParas {
        fn document_plain_paras(&self) -> Vec<String>;
    }
    impl PlainParas for Document {
        fn document_plain_paras(&self) -> Vec<String> {
            self.sections[0]
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Paragraph(p) => Some(p.plain_text()),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn empty_paragraph_after_enter_serializes() {
        let mut doc = Document::blank();
        doc.sections[0].blocks = vec![
            Block::Paragraph(Paragraph::from_text("A")),
            Block::Paragraph(Paragraph::empty()),
        ];
        let xml = serialize_document_xml(&doc).unwrap();
        let again = parse_document_xml(xml.as_bytes()).unwrap();
        assert_eq!(again.sections[0].blocks.len(), 2);
        match &again.sections[0].blocks[1] {
            Block::Paragraph(p) => assert_eq!(p.plain_text(), ""),
            _ => panic!("expected empty paragraph"),
        }
    }

    #[test]
    fn unknown_r_pr_round_trip() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r>
        <w:rPr>
          <w:b/>
          <w:vanish/>
          <w:position w:val="2"/>
        </w:rPr>
        <w:t>hi</w:t>
      </w:r>
    </w:p>
    <w:sectPr/>
  </w:body>
</w:document>"#;
        let doc = parse_document_xml(xml).unwrap();
        let Block::Paragraph(p) = &doc.sections[0].blocks[0] else {
            panic!("p");
        };
        assert!(!p.runs[0].unknown_r_pr.is_empty());
        let out = serialize_document_xml(&doc).unwrap();
        assert!(out.contains("vanish") || out.contains("position"));
    }
}
