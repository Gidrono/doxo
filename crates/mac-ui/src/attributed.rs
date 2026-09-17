//! Convert between BridgedDocument and NSAttributedString (paragraphs, tables, images).

use document_model::{
    Alignment, Block, ListKind, NodeId, Paragraph, ParagraphProperties, Run, RunContent,
    RunProperties, Table, TableCell, TableRow,
};
use layout_bridge::{
    AttributedSpan, BridgedBlock, BridgedDocument, BridgedImage, BridgedParagraph,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSFontManager, NSFontTraitMask,
    NSForegroundColorAttributeName, NSMutableParagraphStyle, NSParagraphStyle,
    NSParagraphStyleAttributeName, NSTextAlignment, NSTextStorage, NSUnderlineStyle,
    NSUnderlineStyleAttributeName, NSWritingDirection,
};
use objc2_app_kit::NSAttributedStringAttachmentConveniences;
use objc2_foundation::{
    ns_string, NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString,
    NSMutableDictionary, NSNumber, NSObjectProtocol, NSRange, NSString,
};

use crate::images::{attachment_from_image, image_from_attrs};
use crate::tables::{table_cell_coords, table_to_attributed};

pub fn bridged_to_attributed(
    mtm: MainThreadMarker,
    bridged: &BridgedDocument,
) -> Retained<NSMutableAttributedString> {
    let out = NSMutableAttributedString::new();
    for (i, block) in bridged.blocks.iter().enumerate() {
        if i > 0 {
            out.appendAttributedString(&NSAttributedString::from_nsstring(ns_string!("\n")));
        }
        match block {
            BridgedBlock::Paragraph(para) => {
                let para_attr = paragraph_to_attributed(mtm, para);
                out.appendAttributedString(&para_attr);
            }
            BridgedBlock::Table(table) => {
                let table_attr = table_to_attributed(mtm, table);
                out.appendAttributedString(&table_attr);
            }
        }
    }
    if bridged.blocks.is_empty() {
        out.appendAttributedString(&NSAttributedString::from_nsstring(ns_string!("")));
    }
    out
}

pub fn paragraph_to_attributed(
    mtm: MainThreadMarker,
    para: &BridgedParagraph,
) -> Retained<NSMutableAttributedString> {
    paragraph_spans_to_attributed(mtm, para)
}

/// Public for tables.rs — builds run content without forcing an outer newline.
pub fn paragraph_spans_to_attributed(
    mtm: MainThreadMarker,
    para: &BridgedParagraph,
) -> Retained<NSMutableAttributedString> {
    let out = NSMutableAttributedString::new();
    let alignment = map_alignment(para.alignment, para.bidirectional);

    let spans: Vec<AttributedSpan> = if para.spans.is_empty() {
        vec![AttributedSpan {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
            font_name: None,
            font_size_pt: None,
            rtl: para.bidirectional,
            style_id: para.style_id.clone(),
            image: None,
        }]
    } else {
        para.spans.clone()
    };

    for span in &spans {
        if let Some(img) = &span.image {
            if img.data.is_empty() {
                continue;
            }
            let attachment = attachment_from_image(img);
            let piece = NSAttributedString::attributedStringWithAttachment(&attachment);
            out.appendAttributedString(&piece);
            continue;
        }
        let s = NSString::from_str(&span.text);
        let attrs = attributes_for_span(mtm, span, alignment, para.bidirectional);
        let piece = unsafe {
            NSAttributedString::new_with_attributes(&s, ClassType::as_super(&*attrs))
        };
        out.appendAttributedString(&piece);
    }
    out
}

fn attributes_for_span(
    mtm: MainThreadMarker,
    span: &AttributedSpan,
    alignment: NSTextAlignment,
    paragraph_rtl: bool,
) -> Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> {
    let dict: Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> =
        NSMutableDictionary::new();

    let size = span.font_size_pt.unwrap_or(14.0);
    let font_name = span.font_name.as_deref().unwrap_or("Helvetica");
    let mut font = NSFont::fontWithName_size(&NSString::from_str(font_name), size)
        .unwrap_or_else(|| NSFont::systemFontOfSize(size));

    let manager = NSFontManager::sharedFontManager(mtm);
    if span.bold {
        font = manager.convertFont_toHaveTrait(&font, NSFontTraitMask::BoldFontMask);
    }
    if span.italic {
        font = manager.convertFont_toHaveTrait(&font, NSFontTraitMask::ItalicFontMask);
    }

    unsafe {
        dict.setObject_forKey(&*font, ProtocolObject::from_ref(NSFontAttributeName));
        dict.setObject_forKey(
            &*NSColor::blackColor(),
            ProtocolObject::from_ref(NSForegroundColorAttributeName),
        );
    }

    if span.underline {
        let style = NSNumber::new_i64(NSUnderlineStyle::Single.0 as i64);
        unsafe {
            dict.setObject_forKey(
                &*style,
                ProtocolObject::from_ref(NSUnderlineStyleAttributeName),
            );
        }
    }

    let para = NSMutableParagraphStyle::new();
    unsafe {
        para.setAlignment(alignment);
        if span.rtl || paragraph_rtl {
            para.setBaseWritingDirection(NSWritingDirection::RightToLeft);
        }
        dict.setObject_forKey(
            &*para,
            ProtocolObject::from_ref(NSParagraphStyleAttributeName),
        );
    }

    dict
}

fn map_alignment(alignment: Option<Alignment>, bidi: bool) -> NSTextAlignment {
    match alignment {
        Some(Alignment::Left) | Some(Alignment::Start) => NSTextAlignment::Left,
        Some(Alignment::Center) => NSTextAlignment::Center,
        Some(Alignment::Right) | Some(Alignment::End) => NSTextAlignment::Right,
        Some(Alignment::Justify) => NSTextAlignment::Justified,
        None if bidi => NSTextAlignment::Right,
        None => NSTextAlignment::Left,
    }
}

/// Snapshot TextKit content into document blocks (paragraphs + edited tables + images).
pub fn attributed_to_blocks(
    mtm: MainThreadMarker,
    attr: &NSAttributedString,
) -> Vec<Block> {
    let full = attr.string().to_string();
    let chars: Vec<char> = full.chars().collect();
    let mut para_infos: Vec<ParaInfo> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= chars.len() {
        if i == chars.len() || chars[i] == '\n' {
            let end = i;
            let para_text: String = chars[start..end].iter().collect();
            let utf16_start = str_utf16_len(&chars[..start].iter().collect::<String>());
            let utf16_len = str_utf16_len(&para_text);
            let runs = extract_runs(mtm, attr, utf16_start, utf16_len, &para_text);
            let pprops = infer_para_props(attr, utf16_start, utf16_len);
            let cell = table_info_at(attr, utf16_start, utf16_len);
            para_infos.push(ParaInfo {
                text: para_text,
                props: pprops,
                runs,
                cell,
            });
            start = i + 1;
        }
        i += 1;
    }
    if para_infos.is_empty() {
        para_infos.push(ParaInfo {
            text: String::new(),
            props: ParagraphProperties::default(),
            runs: vec![],
            cell: None,
        });
    }

    group_paras_into_blocks(para_infos)
}

struct ParaInfo {
    text: String,
    props: ParagraphProperties,
    runs: Vec<(RunPiece, RunProperties)>,
    cell: Option<CellInfo>,
}

#[derive(Clone)]
enum RunPiece {
    Text(String),
    Image(BridgedImage),
}

#[derive(Clone, Copy)]
struct CellInfo {
    cols: usize,
    row: isize,
    col: isize,
    col_span: isize,
}

fn table_info_at(attr: &NSAttributedString, utf16_start: usize, utf16_len: usize) -> Option<CellInfo> {
    if utf16_len == 0 && attr.length() == 0 {
        return None;
    }
    let idx = if utf16_len == 0 {
        utf16_start.saturating_sub(1).min(attr.length().saturating_sub(1))
    } else {
        utf16_start.min(attr.length().saturating_sub(1))
    };
    if attr.length() == 0 {
        return None;
    }
    let mut effective = NSRange {
        location: 0,
        length: 0,
    };
    let attrs = unsafe { attr.attributesAtIndex_effectiveRange(idx, &mut effective) };
    let obj = unsafe { attrs.objectForKey(NSParagraphStyleAttributeName)? };
    let style = obj.downcast::<NSParagraphStyle>().ok()?;
    let (cols, row, col, col_span) = table_cell_coords(&style)?;
    Some(CellInfo {
        cols,
        row,
        col,
        col_span,
    })
}

fn group_paras_into_blocks(paras: Vec<ParaInfo>) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < paras.len() {
        if paras[i].cell.is_some() {
            let start = i;
            while i < paras.len() && paras[i].cell.is_some() {
                i += 1;
            }
            let slice = &paras[start..i];
            if let Some(table) = build_table_from_cells(slice) {
                blocks.push(Block::Table(table));
            } else {
                for p in slice {
                    blocks.push(Block::Paragraph(para_info_to_paragraph(p)));
                }
            }
        } else {
            blocks.push(Block::Paragraph(para_info_to_paragraph(&paras[i])));
            i += 1;
        }
    }
    if blocks.is_empty() {
        blocks.push(Block::Paragraph(Paragraph::empty()));
    }
    blocks
}

fn build_table_from_cells(cells: &[ParaInfo]) -> Option<Table> {
    let first = cells.first()?.cell?;
    let cols = first.cols.max(1);
    let max_row = cells
        .iter()
        .filter_map(|c| c.cell.map(|x| x.row))
        .max()?
        .max(0) as usize;
    let mut rows: Vec<TableRow> = (0..=max_row)
        .map(|_| TableRow {
            cells: (0..cols)
                .map(|_| TableCell {
                    paragraphs: vec![Paragraph::from_text("")],
                    grid_span: 1,
                })
                .collect(),
        })
        .collect();

    for info in cells {
        let Some(ci) = info.cell else {
            continue;
        };
        let r = ci.row.max(0) as usize;
        let c = ci.col.max(0) as usize;
        if r < rows.len() && c < rows[r].cells.len() {
            rows[r].cells[c] = TableCell {
                paragraphs: vec![para_info_to_paragraph(info)],
                grid_span: ci.col_span.max(1) as u32,
            };
        }
    }

    Some(Table {
        id: NodeId::new(),
        rows,
        source_xml: None,
        edited: true,
    })
}

fn para_info_to_paragraph(info: &ParaInfo) -> Paragraph {
    let mut list_kind = info.props.list_kind;
    let mut text_runs = info.runs.clone();
    // Strip visual list prefixes when present.
    if let Some((RunPiece::Text(t), _)) = text_runs.first_mut() {
        if t.starts_with("•\t") {
            list_kind = Some(ListKind::Bullet);
            *t = t.trim_start_matches("•\t").to_string();
        } else if t.starts_with("1.\t") {
            list_kind = Some(ListKind::Numbered);
            *t = t.trim_start_matches("1.\t").to_string();
        }
    }
    let mut props = info.props.clone();
    props.list_kind = list_kind;

    let mut p = Paragraph {
        id: NodeId::new(),
        style_id: Some("Normal".into()),
        properties: props,
        runs: Vec::new(),
        source_xml: None,
        unknown_p_pr: Vec::new(),
        unknown_children: Vec::new(),
    };
    if text_runs.is_empty() {
        p.runs.push(Run::text(info.text.clone()));
    } else {
        for (piece, rp) in text_runs {
            match piece {
                RunPiece::Text(t) => {
                    p.runs.push(Run {
                        id: NodeId::new(),
                        properties: rp,
                        content: RunContent::Text(t),
                        source_xml: None,
                        unknown_r_pr: Vec::new(),
                    });
                }
                RunPiece::Image(img) => {
                    p.runs.push(Run {
                        id: NodeId::new(),
                        properties: rp,
                        content: RunContent::Image(img.into_inline()),
                        source_xml: None,
                        unknown_r_pr: Vec::new(),
                    });
                }
            }
        }
    }
    p
}

/// Legacy flat paragraph extract (no tables). Prefer [`attributed_to_blocks`].
pub fn attributed_to_paragraphs(
    mtm: MainThreadMarker,
    attr: &NSAttributedString,
) -> Vec<(String, ParagraphProperties, Vec<(String, RunProperties)>)> {
    attributed_to_blocks(mtm, attr)
        .into_iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => {
                let text = p.plain_text();
                let runs = p
                    .runs
                    .into_iter()
                    .filter_map(|r| match r.content {
                        RunContent::Text(t) => Some((t, r.properties)),
                        _ => None,
                    })
                    .collect();
                Some((text, p.properties, runs))
            }
            _ => None,
        })
        .collect()
}

fn str_utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn extract_runs(
    mtm: MainThreadMarker,
    attr: &NSAttributedString,
    utf16_start: usize,
    utf16_len: usize,
    para_text: &str,
) -> Vec<(RunPiece, RunProperties)> {
    if utf16_len == 0 {
        return vec![(RunPiece::Text(String::new()), RunProperties::default())];
    }
    let mut runs = Vec::new();
    let mut loc = utf16_start;
    let end = utf16_start + utf16_len;
    let para_utf16: Vec<u16> = para_text.encode_utf16().collect();

    while loc < end {
        let mut effective = NSRange {
            location: 0,
            length: 0,
        };
        let attrs = unsafe { attr.attributesAtIndex_effectiveRange(loc, &mut effective) };
        let run_end = (effective.location + effective.length).min(end);
        let local_start = loc - utf16_start;
        let local_end = run_end - utf16_start;
        let props = run_props_from_attrs(mtm, &attrs);

        if let Some(img) = image_from_attrs(&attrs) {
            runs.push((RunPiece::Image(img), props));
        } else {
            let slice = String::from_utf16_lossy(
                &para_utf16[local_start..local_end.min(para_utf16.len())],
            );
            // Attachment character U+FFFC may appear as text — skip empty-looking object replacement
            if slice.chars().all(|c| c == '\u{FFFC}') {
                // try image again already failed; skip
            } else {
                runs.push((RunPiece::Text(slice), props));
            }
        }
        loc = run_end;
        if effective.length == 0 {
            break;
        }
    }
    if runs.is_empty() {
        runs.push((
            RunPiece::Text(para_text.to_string()),
            RunProperties::default(),
        ));
    }
    runs
}

fn infer_para_props(
    attr: &NSAttributedString,
    utf16_start: usize,
    utf16_len: usize,
) -> ParagraphProperties {
    let mut props = ParagraphProperties::default();
    if utf16_len == 0 || attr.length() == 0 {
        return props;
    }
    let mut effective = NSRange {
        location: 0,
        length: 0,
    };
    let idx = utf16_start.min(attr.length() - 1);
    let attrs = unsafe { attr.attributesAtIndex_effectiveRange(idx, &mut effective) };
    if let Some(obj) = unsafe { attrs.objectForKey(NSParagraphStyleAttributeName) } {
        if let Ok(style) = obj.downcast::<NSParagraphStyle>() {
            let align = unsafe { style.alignment() };
            props.alignment = Some(match align {
                NSTextAlignment::Center => Alignment::Center,
                NSTextAlignment::Right => Alignment::Right,
                NSTextAlignment::Justified => Alignment::Justify,
                NSTextAlignment::Left => Alignment::Left,
                _ => Alignment::Left,
            });
            let dir = unsafe { style.baseWritingDirection() };
            if dir == NSWritingDirection::RightToLeft {
                props.bidirectional = true;
            }
        }
    }
    props
}

fn run_props_from_attrs(
    mtm: MainThreadMarker,
    attrs: &NSDictionary<NSAttributedStringKey, AnyObject>,
) -> RunProperties {
    let mut props = RunProperties::default();
    if let Some(obj) = unsafe { attrs.objectForKey(NSFontAttributeName) } {
        if let Ok(font) = obj.downcast::<NSFont>() {
            let name = font.fontName().to_string();
            props.font_ascii = Some(name.clone());
            props.font_h_ansi = Some(name.clone());
            props.font_cs = Some(name);
            props.font_size_half_points = Some((font.pointSize() * 2.0).round() as u32);
            let traits = NSFontManager::sharedFontManager(mtm).traitsOfFont(&font);
            if traits.contains(NSFontTraitMask::BoldFontMask) {
                props.bold = true;
            }
            if traits.contains(NSFontTraitMask::ItalicFontMask) {
                props.italic = true;
            }
        }
    }
    if let Some(obj) = unsafe { attrs.objectForKey(NSUnderlineStyleAttributeName) } {
        if let Ok(num) = obj.downcast::<NSNumber>() {
            if num.as_i64() != 0 {
                props.underline = true;
            }
        }
    }
    if let Some(obj) = unsafe { attrs.objectForKey(NSParagraphStyleAttributeName) } {
        if let Ok(style) = obj.downcast::<NSParagraphStyle>() {
            let dir = unsafe { style.baseWritingDirection() };
            if dir == NSWritingDirection::RightToLeft {
                props.rtl = true;
            }
        }
    }
    props
}

/// Apply a trait toggle to the selected range in a text view's text storage.
pub fn toggle_trait_in_range(
    mtm: MainThreadMarker,
    storage: &NSTextStorage,
    range: NSRange,
    bold: bool,
    italic: bool,
) {
    if range.length == 0 {
        return;
    }
    let manager = NSFontManager::sharedFontManager(mtm);
    let mut loc = range.location;
    let end = range.location + range.length;
    while loc < end {
        let mut effective = NSRange {
            location: 0,
            length: 0,
        };
        let attrs = unsafe { storage.attributesAtIndex_effectiveRange(loc, &mut effective) };
        let run_end = (effective.location + effective.length).min(end);
        let apply_range = NSRange {
            location: loc,
            length: run_end - loc,
        };
        if let Some(obj) = unsafe { attrs.objectForKey(NSFontAttributeName) } {
            if let Ok(font) = obj.downcast::<NSFont>() {
                let mut new_font = font;
                if bold {
                    let traits = manager.traitsOfFont(&new_font);
                    if traits.contains(NSFontTraitMask::BoldFontMask) {
                        new_font = manager
                            .convertFont_toNotHaveTrait(&new_font, NSFontTraitMask::BoldFontMask);
                    } else {
                        new_font =
                            manager.convertFont_toHaveTrait(&new_font, NSFontTraitMask::BoldFontMask);
                    }
                }
                if italic {
                    let traits = manager.traitsOfFont(&new_font);
                    if traits.contains(NSFontTraitMask::ItalicFontMask) {
                        new_font = manager.convertFont_toNotHaveTrait(
                            &new_font,
                            NSFontTraitMask::ItalicFontMask,
                        );
                    } else {
                        new_font = manager
                            .convertFont_toHaveTrait(&new_font, NSFontTraitMask::ItalicFontMask);
                    }
                }
                unsafe {
                    storage.addAttribute_value_range(NSFontAttributeName, &new_font, apply_range);
                }
            }
        }
        loc = run_end;
        if effective.length == 0 {
            break;
        }
    }
}

/// Merge Tier-C unknown OOXML anchors from a prior document onto newly extracted blocks.
pub fn merge_unknown_anchors(previous: &[Block], new_blocks: &mut [Block]) {
    let prev_paras: Vec<&Paragraph> = previous
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .collect();
    let mut pi = 0usize;
    for block in new_blocks.iter_mut() {
        if let Block::Paragraph(p) = block {
            if let Some(prev) = prev_paras.get(pi) {
                if p.unknown_p_pr.is_empty() {
                    p.unknown_p_pr = prev.unknown_p_pr.clone();
                }
                if p.unknown_children.is_empty() {
                    p.unknown_children = prev.unknown_children.clone();
                }
                // Merge unknown_r_pr by run index when lengths match-ish.
                for (ri, run) in p.runs.iter_mut().enumerate() {
                    if run.unknown_r_pr.is_empty() {
                        if let Some(pr) = prev.runs.get(ri) {
                            run.unknown_r_pr = pr.unknown_r_pr.clone();
                        }
                    }
                }
            }
            pi += 1;
        }
    }
}

#[allow(dead_code)]
fn _keep_traits(_: &dyn NSObjectProtocol) {}
