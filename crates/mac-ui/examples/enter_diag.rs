//! Diagnostic: does insertNewline work in the paginated multi-container editor?
//! Run: cargo run -p mac-ui --example enter_diag

use layout_bridge::{AttributedSpan, BridgedBlock, BridgedDocument, BridgedParagraph};
use mac_ui::attributed_to_blocks;
use mac_ui::page_canvas::create_paginated_editor;
use objc2::runtime::AnyObject;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSPoint, NSRect, NSSize};

fn main() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let bridged = BridgedDocument {
        blocks: vec![BridgedBlock::Paragraph(BridgedParagraph {
            spans: vec![AttributedSpan {
                text: "Hello line one".into(),
                bold: false,
                italic: false,
                underline: false,
                font_name: None,
                font_size_pt: Some(12.0),
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
        })],
        header: None,
        footer: None,
        page: Default::default(),
    };

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
    let editor = create_paginated_editor(mtm, frame, &bridged);

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(820.0, 640.0)),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(ns_string!("enter_diag"));
    if let Some(content) = window.contentView() {
        content.addSubview(&editor.scroll_view);
    }
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&editor.primary_text_view));
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    let tv = editor.text_view();
    let before = editor.text_storage.string().to_string();
    eprintln!("before={before:?}");
    eprintln!("editable={}", tv.isEditable());
    eprintln!("in_window={}", tv.window().is_some());
    eprintln!("has_first_responder={}", window.firstResponder().is_some());

    // Path A: raw replace (should always work)
    let len = editor.text_storage.length();
    editor.text_storage.replaceCharactersInRange_withString(
        objc2_foundation::NSRange {
            location: len,
            length: 0,
        },
        ns_string!("\n"),
    );
    eprintln!("after_replace={:?}", editor.text_storage.string().to_string());

    // Path B: insertNewline: (what Return key sends)
    let len = editor.text_storage.length();
    tv.setSelectedRange(objc2_foundation::NSRange {
        location: len,
        length: 0,
    });
    eprintln!("calling insertNewline…");
    unsafe {
        let _: () = objc2::msg_send![&*tv, insertNewline: Option::<&AnyObject>::None];
    }
    eprintln!(
        "after_insertNewline={:?}",
        editor.text_storage.string().to_string()
    );

    let blocks = attributed_to_blocks(mtm, &editor.text_storage);
    eprintln!("block_count={}", blocks.len());
    for (i, b) in blocks.iter().enumerate() {
        match b {
            document_model::Block::Paragraph(p) => {
                eprintln!("  para[{i}]={:?}", p.plain_text());
            }
            document_model::Block::Table(_) => eprintln!("  table[{i}]"),
            _ => eprintln!("  other[{i}]"),
        }
    }
    eprintln!("DONE");
}
