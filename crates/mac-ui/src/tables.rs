//! NSTextTable helpers for rich table editing.

use layout_bridge::{AttributedSpan, BridgedParagraph, BridgedTable};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSColor, NSMutableParagraphStyle, NSParagraphStyle, NSParagraphStyleAttributeName, NSTextBlock,
    NSTextBlockLayer, NSTextBlockValueType, NSTextTable, NSTextTableBlock,
};
use objc2_foundation::{
    ns_string, NSArray, NSAttributedString, NSAttributedStringKey, NSMutableAttributedString,
    NSMutableDictionary, NSRange, NSString,
};

use crate::attributed::paragraph_spans_to_attributed;

/// Build an attributed string for a table (newline-separated cell paragraphs with NSTextTableBlocks).
pub fn table_to_attributed(
    mtm: MainThreadMarker,
    table: &BridgedTable,
) -> Retained<NSMutableAttributedString> {
    let out = NSMutableAttributedString::new();
    let cols = table.rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let rows = table.rows.len().max(1);

    let nstable = NSTextTable::new();
    nstable.setNumberOfColumns(cols);
    nstable.setCollapsesBorders(true);

    for r in 0..rows {
        for c in 0..cols {
            let cell = table
                .rows
                .get(r)
                .and_then(|row| row.get(c))
                .cloned()
                .unwrap_or_else(empty_cell);

            let block = NSTextTableBlock::initWithTable_startingRow_rowSpan_startingColumn_columnSpan(
                NSTextTableBlock::alloc(),
                &nstable,
                r as isize,
                1,
                c as isize,
                1,
            );
            style_table_block(&block, cols);

            let cell_body = paragraph_spans_to_attributed(mtm, &cell);
            let piece = if cell_body.length() == 0 {
                attributed_cell_placeholder(&block)
            } else {
                let mutable = cell_body;
                attach_block_to_range(&mutable, &block, 0, mutable.length());
                mutable
            };

            if out.length() > 0 {
                out.appendAttributedString(&NSAttributedString::from_nsstring(ns_string!("\n")));
            }
            out.appendAttributedString(&piece);
        }
    }
    out
}

fn empty_cell() -> BridgedParagraph {
    BridgedParagraph {
        spans: vec![AttributedSpan {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
            font_name: None,
            font_size_pt: None,
            rtl: false,
            style_id: None,
            image: None,
        }],
        bidirectional: false,
        alignment: None,
        style_id: None,
        list_kind: None,
        unknown_p_pr: Vec::new(),
        unknown_children: Vec::new(),
    }
}

fn style_table_block(block: &NSTextTableBlock, cols: usize) {
    let black = NSColor::blackColor();
    block.setBorderColor(Some(&black));
    block.setWidth_type_forLayer(
        1.0,
        NSTextBlockValueType::AbsoluteValueType,
        NSTextBlockLayer::Border,
    );
    block.setWidth_type_forLayer(
        4.0,
        NSTextBlockValueType::AbsoluteValueType,
        NSTextBlockLayer::Padding,
    );
    let pct = 100.0 / cols.max(1) as f64;
    block.setContentWidth_type(pct, NSTextBlockValueType::PercentageValueType);
}

fn attributed_cell_placeholder(block: &NSTextTableBlock) -> Retained<NSMutableAttributedString> {
    let dict: Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> =
        NSMutableDictionary::new();
    let para = NSMutableParagraphStyle::new();
    let as_block: &NSTextBlock = ClassType::as_super(block);
    let arr = NSArray::from_slice(&[as_block]);
    unsafe {
        para.setTextBlocks(&arr);
        dict.setObject_forKey(
            &*para,
            ProtocolObject::from_ref(NSParagraphStyleAttributeName),
        );
    }
    let s = NSString::from_str(" ");
    let piece = unsafe {
        NSAttributedString::new_with_attributes(&s, ClassType::as_super(&*dict))
    };
    let out = NSMutableAttributedString::new();
    out.appendAttributedString(&piece);
    out
}

fn attach_block_to_range(
    attr: &NSMutableAttributedString,
    block: &NSTextTableBlock,
    loc: usize,
    len: usize,
) {
    if len == 0 {
        return;
    }
    let mut pos = loc;
    let end = loc + len;
    let mut effective = NSRange {
        location: 0,
        length: 0,
    };
    while pos < end {
        let attrs = unsafe { attr.attributesAtIndex_effectiveRange(pos, &mut effective) };
        let run_end = (effective.location + effective.length).min(end);
        let apply = NSRange {
            location: pos,
            length: run_end - pos,
        };
        let para = if let Some(obj) = unsafe { attrs.objectForKey(NSParagraphStyleAttributeName) }
        {
            if let Ok(existing) = obj.downcast::<NSParagraphStyle>() {
                let m = NSMutableParagraphStyle::new();
                unsafe {
                    m.setParagraphStyle(&existing);
                }
                m
            } else {
                NSMutableParagraphStyle::new()
            }
        } else {
            NSMutableParagraphStyle::new()
        };
        let as_block: &NSTextBlock = ClassType::as_super(block);
        let arr = NSArray::from_slice(&[as_block]);
        unsafe {
            para.setTextBlocks(&arr);
            attr.addAttribute_value_range(NSParagraphStyleAttributeName, &para, apply);
        }
        pos = run_end;
        if effective.length == 0 {
            break;
        }
    }
}

/// If this paragraph style belongs to an NSTextTable cell, return (cols, row, col, col_span).
pub fn table_cell_coords(style: &NSParagraphStyle) -> Option<(usize, isize, isize, isize)> {
    let blocks = unsafe { style.textBlocks() };
    if blocks.count() == 0 {
        return None;
    }
    let first = blocks.objectAtIndex(0);
    let Ok(tb) = first.downcast::<NSTextTableBlock>() else {
        return None;
    };
    let table = tb.table();
    Some((
        table.numberOfColumns(),
        tb.startingRow(),
        tb.startingColumn(),
        tb.columnSpan(),
    ))
}
