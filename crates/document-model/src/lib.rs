//! Semantic document model for word-rs.
//!
//! Mirrors WordprocessingML concepts (sections, paragraphs, runs, styles)
//! without being a direct XML DOM. Stable node ids support commands and
//! round-trip anchors back to OOXML.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identity for paragraphs, runs, and other model nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Top-level document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Document {
    pub metadata: DocumentMetadata,
    pub styles: StyleTable,
    pub sections: Vec<Section>,
    /// Opaque notes for compatibility / round-trip bookkeeping.
    pub compatibility_notes: Vec<String>,
}

impl Document {
    pub fn blank() -> Self {
        let mut doc = Self::default();
        doc.styles = StyleTable::builtin();
        doc.sections.push(Section {
            id: NodeId::new(),
            page: PageSetup::default(),
            blocks: vec![Block::Paragraph(Paragraph::empty())],
            header: None,
            footer: Some("word-rs".into()),
            header_r_id: None,
            footer_r_id: None,
            header_source_xml: None,
            footer_source_xml: None,
        });
        doc
    }

    pub fn paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        self.sections.iter().flat_map(|s| {
            s.blocks.iter().filter_map(|b| match b {
                Block::Paragraph(p) => Some(p),
                Block::Table(_) | Block::Unsupported(_) => None,
            })
        })
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.paragraphs().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&p.plain_text());
        }
        out
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub creator: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StyleTable {
    pub styles: Vec<Style>,
}

impl StyleTable {
    pub fn builtin() -> Self {
        Self {
            styles: vec![
                Style {
                    style_id: "Normal".into(),
                    name: Some("Normal".into()),
                    based_on: None,
                },
                Style {
                    style_id: "Title".into(),
                    name: Some("Title".into()),
                    based_on: Some("Normal".into()),
                },
                Style {
                    style_id: "Heading1".into(),
                    name: Some("Heading 1".into()),
                    based_on: Some("Normal".into()),
                },
                Style {
                    style_id: "Heading2".into(),
                    name: Some("Heading 2".into()),
                    based_on: Some("Normal".into()),
                },
                Style {
                    style_id: "Heading3".into(),
                    name: Some("Heading 3".into()),
                    based_on: Some("Normal".into()),
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Style {
    pub style_id: String,
    pub name: Option<String>,
    pub based_on: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: NodeId,
    pub page: PageSetup,
    pub blocks: Vec<Block>,
    /// Plain text shown in the Print Layout header field.
    pub header: Option<String>,
    /// Plain text shown in the Print Layout footer field.
    pub footer: Option<String>,
    /// Document relationship Id for the default header part (`rIdN`).
    pub header_r_id: Option<String>,
    /// Document relationship Id for the default footer part (`rIdN`).
    pub footer_r_id: Option<String>,
    /// Original `word/header*.xml` retained for fidelity when text is unchanged.
    pub header_source_xml: Option<String>,
    /// Original `word/footer*.xml` retained for fidelity when text is unchanged.
    pub footer_source_xml: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSetup {
    pub width_twips: u32,
    pub height_twips: u32,
    pub margin_top_twips: u32,
    pub margin_bottom_twips: u32,
    pub margin_left_twips: u32,
    pub margin_right_twips: u32,
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            width_twips: 12240,
            height_twips: 15840,
            margin_top_twips: 1440,
            margin_bottom_twips: 1440,
            margin_left_twips: 1440,
            margin_right_twips: 1440,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    Unsupported(UnsupportedBlock),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub id: NodeId,
    pub rows: Vec<TableRow>,
    /// Opaque original markup retained when present for maximum fidelity.
    pub source_xml: Option<String>,
    /// Prefer regenerating `w:tbl` from rows when true (after structural edits).
    pub edited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCell {
    pub paragraphs: Vec<Paragraph>,
    pub grid_span: u32,
}

impl TableCell {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            paragraphs: vec![Paragraph::from_text(text)],
            grid_span: 1,
        }
    }
}

impl Table {
    pub fn empty(rows: usize, cols: usize) -> Self {
        let rows = (0..rows)
            .map(|_| TableRow {
                cells: (0..cols).map(|_| TableCell::from_text("")).collect(),
            })
            .collect();
        Self {
            id: NodeId::new(),
            rows,
            source_xml: None,
            edited: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedBlock {
    pub kind: String,
    pub raw_xml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub id: NodeId,
    pub style_id: Option<String>,
    pub properties: ParagraphProperties,
    pub runs: Vec<Run>,
    pub source_xml: Option<String>,
    /// Unknown elements/attributes from `w:pPr` retained for Tier C fidelity.
    pub unknown_p_pr: Vec<String>,
    /// Unknown sibling elements inside `w:p` (not runs) retained opaquely.
    pub unknown_children: Vec<String>,
}

impl Paragraph {
    pub fn empty() -> Self {
        Self {
            id: NodeId::new(),
            style_id: Some("Normal".into()),
            properties: ParagraphProperties::default(),
            runs: vec![Run::text("")],
            source_xml: None,
            unknown_p_pr: Vec::new(),
            unknown_children: Vec::new(),
        }
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            id: NodeId::new(),
            style_id: Some("Normal".into()),
            properties: ParagraphProperties::default(),
            runs: vec![Run::text(text)],
            source_xml: None,
            unknown_p_pr: Vec::new(),
            unknown_children: Vec::new(),
        }
    }

    pub fn plain_text(&self) -> String {
        self.runs
            .iter()
            .filter_map(|r| match &r.content {
                RunContent::Text(t) => Some(t.as_str()),
                RunContent::Break => Some("\n"),
                RunContent::Tab => Some("\t"),
                RunContent::Image(_) => Some("[image]"),
                RunContent::Unsupported(_) => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParagraphProperties {
    pub bidirectional: bool,
    pub alignment: Option<Alignment>,
    pub spacing_before_twips: Option<u32>,
    pub spacing_after_twips: Option<u32>,
    pub line_spacing_twips: Option<u32>,
    pub list_kind: Option<ListKind>,
    pub list_level: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListKind {
    Bullet,
    Numbered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justify,
    Start,
    End,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: NodeId,
    pub properties: RunProperties,
    pub content: RunContent,
    pub source_xml: Option<String>,
    /// Unknown elements inside `w:rPr` retained for Tier C fidelity.
    pub unknown_r_pr: Vec<String>,
}

impl Run {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            id: NodeId::new(),
            properties: RunProperties::default(),
            content: RunContent::Text(text.into()),
            source_xml: None,
            unknown_r_pr: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProperties {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub font_ascii: Option<String>,
    pub font_h_ansi: Option<String>,
    pub font_cs: Option<String>,
    pub font_east_asia: Option<String>,
    pub font_size_half_points: Option<u32>,
    pub rtl: bool,
    pub language: Option<String>,
    pub color_hex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunContent {
    Text(String),
    Break,
    Tab,
    Image(InlineImage),
    Unsupported(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineImage {
    pub content_type: String,
    pub data: Vec<u8>,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub relationship_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_document_has_one_empty_paragraph() {
        let doc = Document::blank();
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.plain_text(), "");
    }

    #[test]
    fn hebrew_mixed_plain_text() {
        let mut doc = Document::blank();
        let section = &mut doc.sections[0];
        section.blocks = vec![Block::Paragraph(Paragraph::from_text(
            "שלום Hello 123",
        ))];
        assert_eq!(doc.plain_text(), "שלום Hello 123");
    }
}
