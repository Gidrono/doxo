//! Main AppKit application: Print Layout canvas, menus, formatting, open/save.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use layout_bridge::{document_to_bridged, sample_hebrew_english_text, STYLE_PRESETS};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSFont, NSFontAttributeName,
    NSFontManager, NSFontTraitMask, NSMenu, NSMenuItem, NSModalResponseOK,
    NSMutableParagraphStyle, NSOpenPanel, NSParagraphStyleAttributeName, NSPopUpButton,
    NSSavePanel, NSTextAlignment, NSTextView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    NSWritingDirection,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString,
};
use persistence::DocumentSession;

use crate::attributed::{
    attributed_to_blocks, merge_unknown_anchors, toggle_trait_in_range,
};
use crate::images::image_to_attributed;
use crate::page_canvas::{create_paginated_editor, PaginatedEditor};
use crate::tables::table_to_attributed;
use crate::toolbar::HomeToolbar;
use layout_bridge::{BridgedImage, BridgedParagraph, BridgedTable};

struct AppState {
    window: Option<Retained<NSWindow>>,
    editor: Option<PaginatedEditor>,
    session: Option<DocumentSession>,
    session_path: Option<PathBuf>,
    dirty: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            window: None,
            editor: None,
            session: None,
            session_path: None,
            dirty: false,
        }
    }
}

#[derive(Default)]
struct AppDelegateIvars {
    state: RefCell<AppState>,
    toolbar_keep: RefCell<Option<HomeToolbar>>,
    launched: Cell<bool>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "WordRsAppDelegate"]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            let mtm = self.mtm();
            let app = notification
                .object()
                .unwrap()
                .downcast::<NSApplication>()
                .unwrap();

            install_main_menu(mtm, self);

            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(980.0, 780.0)),
                    NSWindowStyleMask::Titled
                        | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable
                        | NSWindowStyleMask::Resizable,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(ns_string!("word-rs — Print Layout"));
            window.center();
            window.setContentMinSize(NSSize::new(720.0, 520.0));

            let content = window.contentView().expect("content view");

            let toolbar = HomeToolbar::build(mtm, self);
            toolbar.root.setFrame(NSRect::new(
                NSPoint::new(0.0, 736.0),
                NSSize::new(980.0, 44.0),
            ));
            toolbar.root.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewMinYMargin,
            );
            content.addSubview(&toolbar.root);

            // Startup document → bridged → paginated editor
            let (bridged, session, path) = {
                let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/hebrew/mixed-hebrew-english.docx");
                if let Ok(session) = DocumentSession::open_path(&fixture) {
                    let bridged = document_to_bridged(&session.document);
                    (bridged, session, Some(fixture))
                } else {
                    let mut session = DocumentSession::new_blank().expect("blank");
                    use document_model::{Block, Paragraph};
                    session.document.sections[0].blocks = vec![Block::Paragraph(
                        Paragraph::from_text(sample_hebrew_english_text()),
                    )];
                    session.document.sections[0].header = Some("Hebrew / English".into());
                    session.document.sections[0].footer = Some("word-rs".into());
                    let bridged = document_to_bridged(&session.document);
                    (bridged, session, None)
                }
            };

            let canvas_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(980.0, 736.0));
            let editor = create_paginated_editor(mtm, canvas_frame, &bridged);
            content.addSubview(&editor.scroll_view);

            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            window.makeKeyAndOrderFront(None);
            window.makeFirstResponder(Some(&editor.primary_text_view));

            {
                let mut st = self.ivars().state.borrow_mut();
                st.window = Some(window);
                st.editor = Some(editor);
                st.session = Some(session);
                st.session_path = path;
            }
            *self.ivars().toolbar_keep.borrow_mut() = Some(toolbar);
            self.ivars().launched.set(true);

            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }

    unsafe impl NSWindowDelegate for AppDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }

    impl AppDelegate {
        #[unsafe(method(openDocument:))]
        fn open_document(&self, _sender: Option<&NSObject>) {
            let mtm = self.mtm();
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            #[allow(deprecated)]
            panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[ns_string!("docx")])));
            if panel.runModal() != NSModalResponseOK {
                return;
            }
            let urls = panel.URLs();
            let Some(url) = urls.firstObject() else {
                return;
            };
            let Some(path) = url.path() else {
                return;
            };
            let path = PathBuf::from(path.to_string());
            match DocumentSession::open_path(&path) {
                Ok(session) => self.apply_session(session, Some(path)),
                Err(e) => show_alert(mtm, &format!("Failed to open:\n{e}")),
            }
        }

        #[unsafe(method(saveDocument:))]
        fn save_document(&self, _sender: Option<&NSObject>) {
            let mtm = self.mtm();
            let path = self.ivars().state.borrow().session_path.clone();
            if let Some(path) = path {
                if let Err(e) = self.save_to(&path) {
                    show_alert(mtm, &format!("Save failed:\n{e}"));
                }
            } else {
                self.perform_save_as();
            }
        }

        #[unsafe(method(saveDocumentAs:))]
        fn save_document_as(&self, _sender: Option<&NSObject>) {
            self.perform_save_as();
        }

        #[unsafe(method(newDocument:))]
        fn new_document(&self, _sender: Option<&NSObject>) {
            match DocumentSession::new_blank() {
                Ok(mut session) => {
                    use document_model::{Block, Paragraph};
                    session.document.sections[0].blocks = vec![Block::Paragraph(
                        Paragraph::from_text(sample_hebrew_english_text()),
                    )];
                    session.document.sections[0].header = Some("Untitled".into());
                    self.apply_session(session, None);
                }
                Err(e) => show_alert(self.mtm(), &format!("New document failed:\n{e}")),
            }
        }

        #[unsafe(method(toggleBold:))]
        fn toggle_bold(&self, _sender: Option<&NSObject>) {
            let mtm = self.mtm();
            self.with_selection_storage(|storage, range| {
                toggle_trait_in_range(mtm, storage, range, true, false);
            });
        }

        #[unsafe(method(toggleItalic:))]
        fn toggle_italic(&self, _sender: Option<&NSObject>) {
            let mtm = self.mtm();
            self.with_selection_storage(|storage, range| {
                toggle_trait_in_range(mtm, storage, range, false, true);
            });
        }

        #[unsafe(method(toggleUnderline:))]
        fn toggle_underline(&self, _sender: Option<&NSObject>) {
            self.with_selection_storage(|storage, range| {
                if range.length == 0 {
                    return;
                }
                use objc2_app_kit::{NSUnderlineStyle, NSUnderlineStyleAttributeName};
                use objc2_foundation::NSNumber;
                let style = NSNumber::new_i64(NSUnderlineStyle::Single.0 as i64);
                unsafe {
                    storage.addAttribute_value_range(NSUnderlineStyleAttributeName, &style, range);
                }
            });
        }

        #[unsafe(method(alignLeft:))]
        fn align_left(&self, _sender: Option<&NSObject>) {
            self.set_alignment(NSTextAlignment::Left);
        }
        #[unsafe(method(alignCenter:))]
        fn align_center(&self, _sender: Option<&NSObject>) {
            self.set_alignment(NSTextAlignment::Center);
        }
        #[unsafe(method(alignRight:))]
        fn align_right(&self, _sender: Option<&NSObject>) {
            self.set_alignment(NSTextAlignment::Right);
        }
        #[unsafe(method(setRtl:))]
        fn set_rtl(&self, _sender: Option<&NSObject>) {
            self.set_writing_direction(NSWritingDirection::RightToLeft, NSTextAlignment::Right);
        }
        #[unsafe(method(setLtr:))]
        fn set_ltr(&self, _sender: Option<&NSObject>) {
            self.set_writing_direction(NSWritingDirection::LeftToRight, NSTextAlignment::Left);
        }

        #[unsafe(method(changeFontSize:))]
        fn change_font_size(&self, sender: Option<&NSObject>) {
            let Some(field) = sender.and_then(|s| s.downcast_ref::<objc2_app_kit::NSTextField>())
            else {
                return;
            };
            let Ok(size) = field.stringValue().to_string().parse::<f64>() else {
                return;
            };
            if !(6.0..=96.0).contains(&size) {
                return;
            }
            let mtm = self.mtm();
            self.with_selection_storage(|storage, range| {
                let manager = NSFontManager::sharedFontManager(mtm);
                if range.length == 0 {
                    return;
                }
                let mut loc = range.location;
                let end = range.location + range.length;
                while loc < end {
                    let mut effective = objc2_foundation::NSRange {
                        location: 0,
                        length: 0,
                    };
                    let attrs =
                        unsafe { storage.attributesAtIndex_effectiveRange(loc, &mut effective) };
                    let run_end = (effective.location + effective.length).min(end);
                    let apply = objc2_foundation::NSRange {
                        location: loc,
                        length: run_end - loc,
                    };
                    if let Some(obj) = unsafe { attrs.objectForKey(NSFontAttributeName) } {
                        if let Ok(font) = obj.downcast::<NSFont>() {
                            let new_font = manager.convertFont_toSize(&font, size);
                            unsafe {
                                storage.addAttribute_value_range(
                                    NSFontAttributeName,
                                    &new_font,
                                    apply,
                                );
                            }
                        }
                    } else {
                        let font = NSFont::systemFontOfSize(size);
                        unsafe {
                            storage.addAttribute_value_range(NSFontAttributeName, &font, apply);
                        }
                    }
                    loc = run_end;
                    if effective.length == 0 {
                        break;
                    }
                }
            });
        }

        #[unsafe(method(applyStyle:))]
        fn apply_style(&self, sender: Option<&NSObject>) {
            let Some(popup) = sender.and_then(|s| s.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let idx = unsafe { popup.indexOfSelectedItem() };
            if idx < 0 {
                return;
            }
            let preset = STYLE_PRESETS.get(idx as usize).copied().unwrap_or(STYLE_PRESETS[0]);
            let mtm = self.mtm();
            self.with_text_view(|tv| {
                let range = expand_to_paragraphs_tv(tv);
                if let Some(storage) = unsafe { tv.textStorage() } {
                    let manager = NSFontManager::sharedFontManager(mtm);
                    let mut font = NSFont::systemFontOfSize(preset.font_size_pt);
                    if preset.bold {
                        font = manager.convertFont_toHaveTrait(&font, NSFontTraitMask::BoldFontMask);
                    }
                    unsafe {
                        storage.addAttribute_value_range(NSFontAttributeName, &font, range);
                    }
                }
            });
        }

        #[unsafe(method(insertBulletList:))]
        fn insert_bullet_list(&self, _sender: Option<&NSObject>) {
            self.insert_list_prefix("•\t");
        }

        #[unsafe(method(insertNumberedList:))]
        fn insert_numbered_list(&self, _sender: Option<&NSObject>) {
            self.insert_list_prefix("1.\t");
        }

        #[unsafe(method(insertTable:))]
        fn insert_table(&self, _sender: Option<&NSObject>) {
            let mtm = self.mtm();
            let table = BridgedTable {
                rows: vec![
                    vec![
                        cell_para("Col A"),
                        cell_para("Col B"),
                        cell_para("Col C"),
                    ],
                    vec![cell_para(""), cell_para(""), cell_para("")],
                ],
                source_xml: None,
                edited: true,
            };
            let attr = table_to_attributed(mtm, &table);
            self.with_text_view(|tv| {
                if let Some(storage) = unsafe { tv.textStorage() } {
                    let range = tv.selectedRange();
                    // Insert a leading newline when not at start
                    if range.location > 0 {
                        unsafe {
                            storage.replaceCharactersInRange_withString(
                                range,
                                ns_string!("\n"),
                            );
                        }
                        let range2 = objc2_foundation::NSRange {
                            location: range.location + 1,
                            length: 0,
                        };
                        unsafe {
                            storage.replaceCharactersInRange_withAttributedString(range2, &attr);
                        }
                    } else {
                        unsafe {
                            storage.replaceCharactersInRange_withAttributedString(range, &attr);
                        }
                    }
                }
            });
        }

        #[unsafe(method(insertImage:))]
        fn insert_image(&self, _sender: Option<&NSObject>) {
            let mtm = self.mtm();
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            #[allow(deprecated)]
            panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[
                ns_string!("png"),
                ns_string!("jpg"),
                ns_string!("jpeg"),
                ns_string!("gif"),
            ])));
            if panel.runModal() != NSModalResponseOK {
                return;
            }
            let Some(url) = panel.URLs().firstObject() else {
                return;
            };
            let Some(path) = url.path() else {
                return;
            };
            let path = PathBuf::from(path.to_string());
            let Ok(data) = std::fs::read(&path) else {
                show_alert(mtm, "Could not read image file.");
                return;
            };
            let content_type = match path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                _ => "image/png",
            };
            let img = BridgedImage {
                content_type: content_type.into(),
                data,
                width_px: None,
                height_px: None,
                relationship_id: None,
            };
            let attr = image_to_attributed(&img);
            self.with_text_view(|tv| {
                if let Some(storage) = unsafe { tv.textStorage() } {
                    let range = tv.selectedRange();
                    unsafe {
                        storage.replaceCharactersInRange_withAttributedString(range, &attr);
                    }
                }
            });
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    fn with_text_view(&self, f: impl FnOnce(&NSTextView)) {
        if let Some(tv) = self.active_text_view() {
            f(&tv);
            self.focus_editor();
        }
    }

    /// Prefer the key-window first responder when it is one of our page text views.
    fn active_text_view(&self) -> Option<Retained<NSTextView>> {
        let st = self.ivars().state.borrow();
        let ed = st.editor.as_ref()?;
        if let Some(window) = st.window.as_ref() {
            if let Some(fr) = window.firstResponder() {
                if let Ok(tv) = fr.downcast::<NSTextView>() {
                    // Any of our page views shares the same text storage.
                    if let Some(storage) = unsafe { tv.textStorage() } {
                        if std::ptr::eq(
                            storage.as_ref() as *const objc2_app_kit::NSTextStorage,
                            ed.text_storage.as_ref() as *const objc2_app_kit::NSTextStorage,
                        ) {
                            return Some(tv);
                        }
                    }
                }
            }
        }
        Some(ed.primary_text_view.clone())
    }

    fn focus_editor(&self) {
        let tv = self.active_text_view();
        let st = self.ivars().state.borrow();
        let Some(window) = st.window.as_ref() else {
            return;
        };
        let Some(ed) = st.editor.as_ref() else {
            return;
        };
        let tv = tv.unwrap_or_else(|| ed.primary_text_view.clone());
        window.makeFirstResponder(Some(&tv));
    }

    fn with_selection_storage(
        &self,
        f: impl FnOnce(&objc2_app_kit::NSTextStorage, objc2_foundation::NSRange),
    ) {
        let tv = match self.active_text_view() {
            Some(tv) => tv,
            None => return,
        };
        let range = tv.selectedRange();
        {
            let st = self.ivars().state.borrow();
            let Some(ed) = st.editor.as_ref() else {
                return;
            };
            f(&ed.text_storage, range);
        }
        self.focus_editor();
    }

    fn insert_list_prefix(&self, prefix: &str) {
        self.with_text_view(|tv| {
            let range = expand_to_paragraphs_tv(tv);
            if let Some(storage) = unsafe { tv.textStorage() } {
                let full = storage.string().to_string();
                let utf16: Vec<u16> = full.encode_utf16().collect();
                let start = range.location.min(utf16.len());
                let peek_end = (start + 4).min(utf16.len());
                let slice = String::from_utf16_lossy(&utf16[start..peek_end]);
                if !slice.starts_with('•') && !slice.starts_with("1.") {
                    unsafe {
                        storage.replaceCharactersInRange_withString(
                            objc2_foundation::NSRange {
                                location: start,
                                length: 0,
                            },
                            &NSString::from_str(prefix),
                        );
                    }
                }
            }
        });
    }

    fn set_alignment(&self, alignment: NSTextAlignment) {
        self.with_text_view(|tv| {
            let range = expand_to_paragraphs_tv(tv);
            if let Some(storage) = unsafe { tv.textStorage() } {
                let style = NSMutableParagraphStyle::new();
                style.setAlignment(alignment);
                unsafe {
                    storage.addAttribute_value_range(
                        NSParagraphStyleAttributeName,
                        &style,
                        range,
                    );
                }
            }
        });
    }

    fn set_writing_direction(&self, dir: NSWritingDirection, alignment: NSTextAlignment) {
        self.with_text_view(|tv| {
            let range = expand_to_paragraphs_tv(tv);
            if let Some(storage) = unsafe { tv.textStorage() } {
                let style = NSMutableParagraphStyle::new();
                style.setBaseWritingDirection(dir);
                style.setAlignment(alignment);
                unsafe {
                    storage.addAttribute_value_range(
                        NSParagraphStyleAttributeName,
                        &style,
                        range,
                    );
                }
            }
        });
    }

    fn perform_save_as(&self) {
        let mtm = self.mtm();
        let panel = NSSavePanel::savePanel(mtm);
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_slice(&[ns_string!("docx")])));
        panel.setNameFieldStringValue(ns_string!("Untitled.docx"));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(url) = panel.URL() else {
            return;
        };
        let Some(path) = url.path() else {
            return;
        };
        let path = PathBuf::from(path.to_string());
        if let Err(e) = self.save_to(&path) {
            show_alert(mtm, &format!("Save failed:\n{e}"));
        } else {
            self.ivars().state.borrow_mut().session_path = Some(path.clone());
            if let Some(w) = self.ivars().state.borrow().window.as_ref() {
                w.setTitle(&NSString::from_str(
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("word-rs"),
                ));
            }
        }
    }

    fn apply_session(&self, session: DocumentSession, path: Option<PathBuf>) {
        let bridged = document_to_bridged(&session.document);
        let mtm = self.mtm();
        {
            let st = self.ivars().state.borrow();
            if let Some(ed) = st.editor.as_ref() {
                ed.set_bridged(mtm, &bridged);
            }
        }
        let mut st = self.ivars().state.borrow_mut();
        st.session = Some(session);
        st.session_path = path.clone();
        st.dirty = false;
        if let Some(w) = st.window.as_ref() {
            let title = path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled");
            w.setTitle(&NSString::from_str(title));
        }
    }

    fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        let mtm = self.mtm();
        let (mut blocks, header, footer, mut session) = {
            let mut st = self.ivars().state.borrow_mut();
            let ed = st
                .editor
                .as_ref()
                .ok_or_else(|| "no editor".to_string())?;
            let blocks = attributed_to_blocks(mtm, &ed.text_storage);
            let (header, footer) = ed.header_footer_text();
            let session = st
                .session
                .take()
                .unwrap_or_else(|| DocumentSession::new_blank().expect("blank session"));
            (blocks, header, footer, session)
        };

        let previous = session.document.sections[0].blocks.clone();
        merge_unknown_anchors(&previous, &mut blocks);
        session.document.sections[0].blocks = blocks;
        session.document.sections[0].header = Some(header);
        session.document.sections[0].footer = Some(footer);

        session.save_to_path(path).map_err(|e| e.to_string())?;

        let mut st = self.ivars().state.borrow_mut();
        st.session = Some(session);
        st.session_path = Some(path.to_path_buf());
        st.dirty = false;
        Ok(())
    }
}

fn cell_para(text: &str) -> BridgedParagraph {
    BridgedParagraph {
        spans: vec![layout_bridge::AttributedSpan {
            text: text.into(),
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
    }
}

fn expand_to_paragraphs_tv(tv: &NSTextView) -> objc2_foundation::NSRange {
    let range = tv.selectedRange();
    let Some(storage) = (unsafe { tv.textStorage() }) else {
        return range;
    };
    let full = storage.string().to_string();
    let utf16: Vec<u16> = full.encode_utf16().collect();
    if utf16.is_empty() {
        return range;
    }
    let mut start = range.location.min(utf16.len());
    let mut end = (range.location + range.length).min(utf16.len());
    while start > 0 && utf16[start - 1] != b'\n' as u16 {
        start -= 1;
    }
    while end < utf16.len() && utf16[end] != b'\n' as u16 {
        end += 1;
    }
    objc2_foundation::NSRange {
        location: start,
        length: end.saturating_sub(start),
    }
}

fn install_main_menu(mtm: MainThreadMarker, delegate: &AppDelegate) {
    let menu = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    unsafe {
        let quit = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Quit word-rs"),
            Some(sel!(terminate:)),
            ns_string!("q"),
        );
        app_menu.addItem(&quit);
        app_item.setSubmenu(Some(&app_menu));
        menu.addItem(&app_item);
    }
    let file_item = NSMenuItem::new(mtm);
    let file_menu = NSMenu::new(mtm);
    unsafe {
        file_menu.setTitle(ns_string!("File"));
        for (title, action, key) in [
            ("New", sel!(newDocument:), "n"),
            ("Open…", sel!(openDocument:), "o"),
            ("Save", sel!(saveDocument:), "s"),
            ("Save As…", sel!(saveDocumentAs:), "S"),
        ] {
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            );
            item.setTarget(Some(delegate));
            file_menu.addItem(&item);
        }
        file_item.setSubmenu(Some(&file_menu));
        file_item.setTitle(ns_string!("File"));
        menu.addItem(&file_item);
    }
    let edit_item = NSMenuItem::new(mtm);
    let edit_menu = NSMenu::new(mtm);
    unsafe {
        edit_menu.setTitle(ns_string!("Edit"));
        for (title, action, key) in [
            ("Undo", sel!(undo:), "z"),
            ("Redo", sel!(redo:), "Z"),
            ("Cut", sel!(cut:), "x"),
            ("Copy", sel!(copy:), "c"),
            ("Paste", sel!(paste:), "v"),
            ("Select All", sel!(selectAll:), "a"),
        ] {
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            );
            edit_menu.addItem(&item);
        }
        edit_item.setSubmenu(Some(&edit_menu));
        edit_item.setTitle(ns_string!("Edit"));
        menu.addItem(&edit_item);
    }
    let app = NSApplication::sharedApplication(mtm);
    app.setMainMenu(Some(&menu));
    let _ = NSColor::blackColor();
}

fn show_alert(mtm: MainThreadMarker, message: &str) {
    let alert = objc2_app_kit::NSAlert::new(mtm);
    alert.setMessageText(ns_string!("word-rs"));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("UI must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);
    app.run();
}
