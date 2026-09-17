//! UI regression: insertNewline must create paragraphs in Print Layout.
//!
//! Run: `cargo test -p mac-ui --test insert_newline -- --nocapture`
//! (AppKit requires the main thread — this harness starts NSApplication.)

#![cfg(target_os = "macos")]

use layout_bridge::{AttributedSpan, BridgedBlock, BridgedDocument, BridgedParagraph};
use mac_ui::attributed_to_blocks;
use mac_ui::page_canvas::create_paginated_editor;
use objc2::runtime::AnyObject;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSPoint, NSRect, NSSize};

#[test]
fn insert_newline_creates_paragraphs_in_paginated_editor() {
    let Some(mtm) = MainThreadMarker::new() else {
        // Cargo's test harness often runs off the AppKit main thread.
        // Use `cargo run -p mac-ui --example enter_diag` for the GUI path.
        eprintln!("skip: AppKit main thread unavailable in this test runner");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let bridged = BridgedDocument {
        blocks: vec![BridgedBlock::Paragraph(BridgedParagraph {
            spans: vec![AttributedSpan {
                text: "Line A".into(),
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

    let editor = create_paginated_editor(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(700.0, 500.0)),
        &bridged,
    );

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(40.0, 40.0), NSSize::new(720.0, 540.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    if let Some(content) = window.contentView() {
        content.addSubview(&editor.scroll_view);
    }
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&editor.primary_text_view));

    let tv = editor.text_view();
    let len = editor.text_storage.length();
    tv.setSelectedRange(objc2_foundation::NSRange {
        location: len,
        length: 0,
    });

    unsafe {
        let _: () = objc2::msg_send![&*tv, insertNewline: Option::<&AnyObject>::None];
    }
    // Second Return → blank paragraph (common Word behavior).
    unsafe {
        let _: () = objc2::msg_send![&*tv, insertNewline: Option::<&AnyObject>::None];
    }
    unsafe {
        editor.text_storage.replaceCharactersInRange_withString(
            objc2_foundation::NSRange {
                location: editor.text_storage.length(),
                length: 0,
            },
            ns_string!("Line B"),
        );
    }

    let text = editor.text_storage.string().to_string();
    assert!(
        text.contains('\n'),
        "expected newline after insertNewline, got {text:?}"
    );
    assert!(text.contains("Line B"), "{text:?}");

    let blocks = attributed_to_blocks(mtm, &editor.text_storage);
    assert!(
        blocks.len() >= 2,
        "expected multiple paragraphs, got {}",
        blocks.len()
    );
}
