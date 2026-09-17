//! Bridge between the Rust document model and TextKit attributed content,
//! plus page geometry for Print Layout.

use document_model::{
    Alignment, Block, Document, InlineImage, PageSetup, RunContent, RunProperties, Table,
};

/// Points per twip (1 twip = 1/20 pt).
pub fn twips_to_points(twips: u32) -> f64 {
    twips as f64 / 20.0
}

pub fn points_to_twips(points: f64) -> u32 {
    (points * 20.0).round() as u32
}

/// Resolved page metrics in screen points for Print Layout.
#[derive(Debug, Clone, Copy)]
pub struct PageMetrics {
    pub page_width: f64,
    pub page_height: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub content_width: f64,
    pub content_height: f64,
    pub page_gap: f64,
    pub header_height: f64,
    pub footer_height: f64,
}

impl PageMetrics {
    pub const DEFAULT_PAGE_GAP: f64 = 28.0;
    pub const DEFAULT_HEADER_FOOTER: f64 = 28.0;

    pub fn from_page_setup(page: &PageSetup) -> Self {
        let page_width = twips_to_points(page.width_twips);
        let page_height = twips_to_points(page.height_twips);
        let margin_top = twips_to_points(page.margin_top_twips);
        let margin_bottom = twips_to_points(page.margin_bottom_twips);
        let margin_left = twips_to_points(page.margin_left_twips);
        let margin_right = twips_to_points(page.margin_right_twips);
        let header_height = Self::DEFAULT_HEADER_FOOTER.min(margin_top * 0.55);
        let footer_height = Self::DEFAULT_HEADER_FOOTER.min(margin_bottom * 0.55);
        let content_width = (page_width - margin_left - margin_right).max(120.0);
        let content_height = (page_height - margin_top - margin_bottom).max(120.0);
        Self {
            page_width,
            page_height,
            margin_top,
            margin_bottom,
            margin_left,
            margin_right,
            content_width,
            content_height,
            page_gap: Self::DEFAULT_PAGE_GAP,
            header_height,
            footer_height,
        }
    }

    pub fn document_height(&self, page_count: usize) -> f64 {
        let n = page_count.max(1) as f64;
        n * self.page_height + (n - 1.0) * self.page_gap + self.page_gap * 2.0
    }

    pub fn page_origin_y(&self, page_index: usize, doc_height: f64) -> f64 {
        let top_pad = self.page_gap;
        top_pad + page_index as f64 * (self.page_height + self.page_gap) + 0.0 * doc_height
    }

    pub fn content_frame_in_page(&self, page_origin: (f64, f64)) -> (f64, f64, f64, f64) {
        let (px, py) = page_origin;
        (
            px + self.margin_left,
            py + self.margin_top,
            self.content_width,
            self.content_height,
        )
    }
}

/// Portable attributed span for UI bridging (UTF-8 ranges).
#[derive(Debug, Clone)]
pub struct AttributedSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub font_name: Option<String>,
    pub font_size_pt: Option<f64>,
    pub rtl: bool,
    pub style_id: Option<String>,
    /// When set, this span is an inline image (text is ignored for display).
    pub image: Option<BridgedImage>,
}

#[derive(Debug, Clone)]
pub struct BridgedImage {
    pub content_type: String,
    pub data: Vec<u8>,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub relationship_id: Option<String>,
}

impl From<&InlineImage> for BridgedImage {
    fn from(img: &InlineImage) -> Self {
        Self {
            content_type: img.content_type.clone(),
            data: img.data.clone(),
            width_px: img.width_px,
            height_px: img.height_px,
            relationship_id: img.relationship_id.clone(),
        }
    }
}

impl BridgedImage {
    pub fn into_inline(self) -> InlineImage {
        InlineImage {
            content_type: self.content_type,
            data: self.data,
            width_px: self.width_px,
            height_px: self.height_px,
            relationship_id: self.relationship_id,
        }
    }
}

/// Paragraph-level view model for the editor surface.
#[derive(Debug, Clone)]
pub struct BridgedParagraph {
    pub spans: Vec<AttributedSpan>,
    pub bidirectional: bool,
    pub alignment: Option<Alignment>,
    pub style_id: Option<String>,
    pub list_kind: Option<ListKind>,
    /// Opaque unknown `w:pPr` fragments for Tier C merge on save.
    pub unknown_p_pr: Vec<String>,
    pub unknown_children: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BridgedTable {
    pub rows: Vec<Vec<BridgedParagraph>>,
    pub source_xml: Option<String>,
    pub edited: bool,
}

#[derive(Debug, Clone)]
pub enum BridgedBlock {
    Paragraph(BridgedParagraph),
    Table(BridgedTable),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Numbered,
}

#[derive(Debug, Clone, Default)]
pub struct BridgedDocument {
    pub blocks: Vec<BridgedBlock>,
    pub page: PageSetup,
    pub header: Option<String>,
    pub footer: Option<String>,
}

impl BridgedDocument {
    /// Flat paragraph list for callers that ignore tables (legacy).
    pub fn paragraphs(&self) -> Vec<&BridgedParagraph> {
        let mut out = Vec::new();
        for b in &self.blocks {
            match b {
                BridgedBlock::Paragraph(p) => out.push(p),
                BridgedBlock::Table(t) => {
                    for row in &t.rows {
                        for cell in row {
                            out.push(cell);
                        }
                    }
                }
            }
        }
        out
    }

    pub fn plain_text(&self) -> String {
        let mut parts = Vec::new();
        for b in &self.blocks {
            match b {
                BridgedBlock::Paragraph(p) => {
                    parts.push(
                        p.spans
                            .iter()
                            .map(|s| {
                                if s.image.is_some() {
                                    "[image]"
                                } else {
                                    s.text.as_str()
                                }
                            })
                            .collect::<String>(),
                    );
                }
                BridgedBlock::Table(t) => {
                    for row in &t.rows {
                        let cells: Vec<String> = row
                            .iter()
                            .map(|c| {
                                c.spans
                                    .iter()
                                    .map(|s| s.text.as_str())
                                    .collect::<String>()
                            })
                            .collect();
                        parts.push(cells.join("\t"));
                    }
                }
            }
        }
        parts.join("\n")
    }

    pub fn metrics(&self) -> PageMetrics {
        PageMetrics::from_page_setup(&self.page)
    }
}

/// Map semantic model → bridged attributed blocks (paragraphs + tables).
pub fn document_to_bridged(doc: &Document) -> BridgedDocument {
    let mut bridged = BridgedDocument::default();
    if let Some(section) = doc.sections.first() {
        bridged.page = section.page.clone();
        bridged.header = section.header.clone();
        bridged.footer = section.footer.clone();
        for block in &section.blocks {
            match block {
                Block::Paragraph(para) => {
                    bridged
                        .blocks
                        .push(BridgedBlock::Paragraph(bridge_paragraph(para)));
                }
                Block::Table(table) => {
                    bridged.blocks.push(BridgedBlock::Table(bridge_table(table)));
                }
                Block::Unsupported(_) => {}
            }
        }
    }
    if bridged.blocks.is_empty() {
        bridged.blocks.push(BridgedBlock::Paragraph(BridgedParagraph {
            spans: vec![empty_span(None)],
            bidirectional: false,
            alignment: None,
            style_id: Some("Normal".into()),
            list_kind: None,
            unknown_p_pr: Vec::new(),
            unknown_children: Vec::new(),
        }));
    }
    bridged
}

fn bridge_table(table: &Table) -> BridgedTable {
    BridgedTable {
        rows: table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        if cell.paragraphs.is_empty() {
                            return BridgedParagraph {
                                spans: vec![empty_span(None)],
                                bidirectional: false,
                                alignment: None,
                                style_id: None,
                                list_kind: None,
                                unknown_p_pr: Vec::new(),
                                unknown_children: Vec::new(),
                            };
                        }
                        if cell.paragraphs.len() == 1 {
                            return bridge_paragraph(&cell.paragraphs[0]);
                        }
                        // Multi-paragraph cell: join plain text with newlines for the editor.
                        let joined = cell
                            .paragraphs
                            .iter()
                            .map(|p| p.plain_text())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let mut bp = bridge_paragraph(&cell.paragraphs[0]);
                        if let Some(first) = bp.spans.first_mut() {
                            first.text = joined;
                            first.image = None;
                        }
                        bp.spans.truncate(1);
                        bp
                    })
                    .collect()
            })
            .collect(),
        source_xml: table.source_xml.clone(),
        edited: table.edited,
    }
}

fn bridge_paragraph(para: &document_model::Paragraph) -> BridgedParagraph {
    let mut bp = BridgedParagraph {
        spans: Vec::new(),
        bidirectional: para.properties.bidirectional,
        alignment: para.properties.alignment,
        style_id: para.style_id.clone(),
        list_kind: para.properties.list_kind.map(|k| match k {
            document_model::ListKind::Bullet => ListKind::Bullet,
            document_model::ListKind::Numbered => ListKind::Numbered,
        }),
        unknown_p_pr: para.unknown_p_pr.clone(),
        unknown_children: para.unknown_children.clone(),
    };
    for run in &para.runs {
        match &run.content {
            RunContent::Text(t) => {
                let mut span = span_from_props(t.clone(), &run.properties);
                span.style_id = para.style_id.clone();
                bp.spans.push(span);
            }
            RunContent::Break => {
                let mut span = span_from_props("\n".into(), &run.properties);
                span.style_id = para.style_id.clone();
                bp.spans.push(span);
            }
            RunContent::Tab => {
                let mut span = span_from_props("\t".into(), &run.properties);
                span.style_id = para.style_id.clone();
                bp.spans.push(span);
            }
            RunContent::Image(img) => {
                let mut span = span_from_props(String::new(), &run.properties);
                span.image = Some(BridgedImage::from(img));
                span.style_id = para.style_id.clone();
                bp.spans.push(span);
            }
            RunContent::Unsupported(_) => continue,
        }
    }
    if bp.spans.is_empty() {
        bp.spans.push(empty_span(para.style_id.clone()));
    }
    if let Some(kind) = bp.list_kind {
        let prefix = match kind {
            ListKind::Bullet => "•\t",
            ListKind::Numbered => "1.\t",
        };
        if let Some(first) = bp.spans.first_mut() {
            if first.image.is_none() {
                let already = first.text.starts_with('•')
                    || first
                        .text
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false);
                if !already {
                    first.text = format!("{prefix}{}", first.text);
                }
            }
        }
    }
    bp
}

fn empty_span(style_id: Option<String>) -> AttributedSpan {
    AttributedSpan {
        text: String::new(),
        bold: false,
        italic: false,
        underline: false,
        font_name: None,
        font_size_pt: None,
        rtl: false,
        style_id,
        image: None,
    }
}

fn span_from_props(text: String, props: &RunProperties) -> AttributedSpan {
    let font_name = props
        .font_cs
        .clone()
        .or_else(|| props.font_ascii.clone())
        .or_else(|| props.font_h_ansi.clone());
    let font_size_pt = props.font_size_half_points.map(|hp| hp as f64 / 2.0);
    AttributedSpan {
        text,
        bold: props.bold,
        italic: props.italic,
        underline: props.underline,
        font_name,
        font_size_pt,
        rtl: props.rtl,
        style_id: None,
        image: None,
    }
}

/// Sample mixed Hebrew/English text used by the UI spike / New Document.
pub fn sample_hebrew_english_text() -> &'static str {
    "בדיקת עברית: שלום עולם — מסמך בעברית עם English 123, סימני פיסוק וגופנים.\n\
This sample is intentionally mixed-direction: Hebrew + English + numbers.\n\
Type here to verify caret, selection, and IME with TextKit.\n\
\n\
Page layout: keep typing to flow across visible page boundaries in Print Layout."
}

/// Built-in paragraph style presets for the styles UI.
#[derive(Debug, Clone, Copy)]
pub struct StylePreset {
    pub id: &'static str,
    pub name: &'static str,
    pub font_size_pt: f64,
    pub bold: bool,
}

pub const STYLE_PRESETS: &[StylePreset] = &[
    StylePreset {
        id: "Normal",
        name: "Normal",
        font_size_pt: 12.0,
        bold: false,
    },
    StylePreset {
        id: "Title",
        name: "Title",
        font_size_pt: 28.0,
        bold: true,
    },
    StylePreset {
        id: "Heading1",
        name: "Heading 1",
        font_size_pt: 20.0,
        bold: true,
    },
    StylePreset {
        id: "Heading2",
        name: "Heading 2",
        font_size_pt: 16.0,
        bold: true,
    },
    StylePreset {
        id: "Heading3",
        name: "Heading 3",
        font_size_pt: 14.0,
        bold: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use document_model::{Block, Document, Paragraph, Run, RunProperties};

    #[test]
    fn letter_metrics() {
        let m = PageMetrics::from_page_setup(&PageSetup::default());
        assert!((m.page_width - 612.0).abs() < 0.1);
        assert!((m.page_height - 792.0).abs() < 0.1);
        assert!(m.content_width > 400.0);
    }

    #[test]
    fn bridges_bold_hebrew_run() {
        let mut doc = Document::blank();
        let mut p = Paragraph::empty();
        p.properties.bidirectional = true;
        let mut r = Run::text("שלום");
        r.properties = RunProperties {
            bold: true,
            rtl: true,
            font_ascii: Some("Arial".into()),
            font_size_half_points: Some(24),
            ..Default::default()
        };
        p.runs = vec![r, Run::text(" Hello")];
        doc.sections[0].blocks = vec![Block::Paragraph(p)];

        let bridged = document_to_bridged(&doc);
        assert_eq!(bridged.blocks.len(), 1);
        let BridgedBlock::Paragraph(para) = &bridged.blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(para.bidirectional);
        assert!(para.spans[0].bold);
        assert_eq!(para.spans[0].font_size_pt, Some(12.0));
        assert_eq!(bridged.plain_text(), "שלום Hello");
    }

    #[test]
    fn bridges_table_block() {
        let mut doc = Document::blank();
        doc.sections[0].blocks = vec![Block::Table(Table::empty(2, 3))];
        let bridged = document_to_bridged(&doc);
        let BridgedBlock::Table(t) = &bridged.blocks[0] else {
            panic!("expected table");
        };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].len(), 3);
    }
}
