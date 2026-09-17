//! Paginated Print Layout: letter pages with gaps, headers/footers, and
//! multi-container TextKit (text flows page-to-page).

use std::cell::RefCell;

use layout_bridge::{BridgedDocument, PageMetrics};
use objc2::rc::Retained;
use objc2::{
    class, define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly,
};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBox, NSBoxType, NSColor, NSFont, NSLayoutManager, NSScrollView,
    NSTextAlignment, NSTextContainer, NSTextField, NSTextStorage, NSTextView, NSTitlePosition,
    NSView,
};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString,
};

use crate::attributed::bridged_to_attributed;

const MAX_PAGES: usize = 40;

pub struct PaginatedEditor {
    pub scroll_view: Retained<NSScrollView>,
    pub document_view: Retained<NSView>,
    pub primary_text_view: Retained<NSTextView>,
    pub text_storage: Retained<NSTextStorage>,
    layout_manager: Retained<NSLayoutManager>,
    page_views: RefCell<Vec<Retained<NSTextView>>>,
    page_backs: RefCell<Vec<Retained<NSBox>>>,
    header_fields: RefCell<Vec<Retained<NSTextField>>>,
    footer_fields: RefCell<Vec<Retained<NSTextField>>>,
    page_number_fields: RefCell<Vec<Retained<NSTextField>>>,
    metrics: PageMetrics,
    header: RefCell<String>,
    footer: RefCell<String>,
    _observer: Retained<PageLayoutObserver>,
}

/// Construct a paginated editor hosted in `frame`.
pub fn create_paginated_editor(
    mtm: MainThreadMarker,
    frame: NSRect,
    bridged: &BridgedDocument,
) -> PaginatedEditor {
    let metrics = bridged.metrics();
    let header = bridged.header.clone().unwrap_or_default();
    let footer = bridged
        .footer
        .clone()
        .unwrap_or_else(|| "word-rs".into());

    let scroll = unsafe { NSScrollView::new(mtm) };
    scroll.setFrame(frame);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
    scroll.setDrawsBackground(true);
    scroll.setBackgroundColor(&NSColor::colorWithCalibratedWhite_alpha(0.78, 1.0));
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );

    let doc_view: Retained<NSView> = FlippedView::new(mtm).into_super();

    let storage = NSTextStorage::new();
    let layout_manager = NSLayoutManager::new();
    storage.addLayoutManager(&layout_manager);

    let attr = bridged_to_attributed(mtm, bridged);
    storage.setAttributedString(&attr);

    let observer = PageLayoutObserver::new(mtm);

    // Placeholder primary until first page exists
    let placeholder = unsafe { NSTextView::new(mtm) };

    let mut editor = PaginatedEditor {
        scroll_view: scroll,
        document_view: doc_view,
        primary_text_view: placeholder,
        text_storage: storage,
        layout_manager,
        page_views: RefCell::new(Vec::new()),
        page_backs: RefCell::new(Vec::new()),
        header_fields: RefCell::new(Vec::new()),
        footer_fields: RefCell::new(Vec::new()),
        page_number_fields: RefCell::new(Vec::new()),
        metrics,
        header: RefCell::new(header),
        footer: RefCell::new(footer),
        _observer: observer,
    };

    editor.ensure_pages(mtm, 1);
    editor.ensure_enough_pages(mtm);
    editor.install_text_observer();
    editor
}

impl PaginatedEditor {
    pub fn set_bridged(&self, mtm: MainThreadMarker, bridged: &BridgedDocument) {
        if let Some(h) = &bridged.header {
            *self.header.borrow_mut() = h.clone();
        }
        if let Some(f) = &bridged.footer {
            *self.footer.borrow_mut() = f.clone();
        }
        let attr = bridged_to_attributed(mtm, bridged);
        self.text_storage.setAttributedString(&attr);
        self.ensure_enough_pages(mtm);
        self.update_header_footer_labels();
    }

    pub fn text_view(&self) -> Retained<NSTextView> {
        self.primary_text_view.clone()
    }

    fn install_text_observer(&self) {
        let center = NSNotificationCenter::defaultCenter();
        let name = ns_string!("NSTextStorageDidProcessEditingNotification");
        unsafe {
            center.addObserver_selector_name_object(
                &*(self._observer.as_ref() as *const PageLayoutObserver as *const objc2::runtime::AnyObject),
                sel!(textDidChange:),
                Some(name),
                Some(
                    &*(self.text_storage.as_ref() as *const NSTextStorage
                        as *const objc2::runtime::AnyObject),
                ),
            );
        }
        self._observer
            .set_editor_ptr(self as *const PaginatedEditor as usize);
    }

    fn page_frame(&self, index: usize) -> NSRect {
        let doc_w = self.document_view.frame().size.width;
        let x = ((doc_w - self.metrics.page_width) / 2.0).max(24.0);
        let y =
            self.metrics.page_gap + index as f64 * (self.metrics.page_height + self.metrics.page_gap);
        NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(self.metrics.page_width, self.metrics.page_height),
        )
    }

    fn content_frame_in_page(&self, page_frame: NSRect) -> NSRect {
        NSRect::new(
            NSPoint::new(
                page_frame.origin.x + self.metrics.margin_left,
                page_frame.origin.y + self.metrics.margin_top,
            ),
            NSSize::new(self.metrics.content_width, self.metrics.content_height),
        )
    }

    fn ensure_pages(&mut self, mtm: MainThreadMarker, count: usize) {
        let count = count.clamp(1, MAX_PAGES);
        while self.page_views.borrow().len() < count {
            self.add_page(mtm);
        }
        if let Some(first) = self.page_views.borrow().first() {
            self.primary_text_view = first.clone();
        }
        self.relayout();
    }

    fn add_page(&mut self, mtm: MainThreadMarker) {
        let index = self.page_views.borrow().len();
        let page_frame = self.page_frame(index);

        let back = unsafe { NSBox::new(mtm) };
        back.setBoxType(NSBoxType::Custom);
        back.setFillColor(&NSColor::whiteColor());
        back.setBorderColor(&NSColor::colorWithCalibratedWhite_alpha(0.65, 1.0));
        back.setBorderWidth(1.0);
        back.setTitlePosition(NSTitlePosition::NoTitle);
        back.setFrame(page_frame);
        self.document_view.addSubview(&back);

        let header = {
            let f = NSTextField::textFieldWithString(
                &NSString::from_str(self.header.borrow().as_str()),
                mtm,
            );
            f.setFont(Some(&NSFont::systemFontOfSize(10.0)));
            f.setTextColor(Some(&NSColor::secondaryLabelColor()));
            f.setBordered(false);
            f.setBezeled(false);
            f.setEditable(true);
            f.setDrawsBackground(false);
            f.setFrame(NSRect::new(
                NSPoint::new(
                    page_frame.origin.x + self.metrics.margin_left,
                    page_frame.origin.y + self.metrics.header_height * 0.35,
                ),
                NSSize::new(self.metrics.content_width, 16.0),
            ));
            f
        };
        let footer = {
            let f = NSTextField::textFieldWithString(
                &NSString::from_str(self.footer.borrow().as_str()),
                mtm,
            );
            f.setFont(Some(&NSFont::systemFontOfSize(10.0)));
            f.setTextColor(Some(&NSColor::secondaryLabelColor()));
            f.setAlignment(NSTextAlignment::Center);
            f.setBordered(false);
            f.setBezeled(false);
            f.setEditable(true);
            f.setDrawsBackground(false);
            f.setFrame(NSRect::new(
                NSPoint::new(
                    page_frame.origin.x + self.metrics.margin_left,
                    page_frame.origin.y + self.metrics.page_height - self.metrics.margin_bottom
                        + 4.0,
                ),
                NSSize::new(self.metrics.content_width * 0.7, 16.0),
            ));
            f
        };
        // Page number label (read-only), right of footer text.
        let page_label = {
            let label = format!("— {}", index + 1);
            let f = NSTextField::labelWithString(&NSString::from_str(&label), mtm);
            f.setFont(Some(&NSFont::systemFontOfSize(10.0)));
            f.setTextColor(Some(&NSColor::tertiaryLabelColor()));
            f.setAlignment(NSTextAlignment::Right);
            f.setFrame(NSRect::new(
                NSPoint::new(
                    page_frame.origin.x + self.metrics.margin_left + self.metrics.content_width * 0.72,
                    page_frame.origin.y + self.metrics.page_height - self.metrics.margin_bottom
                        + 4.0,
                ),
                NSSize::new(self.metrics.content_width * 0.28, 16.0),
            ));
            f
        };
        self.document_view.addSubview(&header);
        self.document_view.addSubview(&footer);
        self.document_view.addSubview(&page_label);

        let content = self.content_frame_in_page(page_frame);
        let container = NSTextContainer::initWithSize(
            unsafe { NSTextContainer::alloc() },
            NSSize::new(self.metrics.content_width, self.metrics.content_height),
        );
        container.setWidthTracksTextView(false);
        container.setHeightTracksTextView(false);
        self.layout_manager.addTextContainer(&container);

        let tv = unsafe {
            NSTextView::initWithFrame_textContainer(
                NSTextView::alloc(mtm),
                content,
                Some(&container),
            )
        };
        tv.setEditable(true);
        tv.setSelectable(true);
        tv.setRichText(true);
        tv.setImportsGraphics(true);
        // Only the primary (first) page should own the undo manager registration.
        tv.setAllowsUndo(index == 0);
        tv.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        tv.setBackgroundColor(&NSColor::clearColor());
        tv.setDrawsBackground(false);
        tv.setHorizontallyResizable(false);
        tv.setVerticallyResizable(false);
        tv.setTextContainerInset(NSSize::new(0.0, 0.0));
        // Fixed page containers — do not let the view fight container size.
        tv.setMinSize(NSSize::new(self.metrics.content_width, self.metrics.content_height));
        tv.setMaxSize(NSSize::new(self.metrics.content_width, self.metrics.content_height));
        self.document_view.addSubview(&tv);

        self.page_backs.borrow_mut().push(back);
        self.header_fields.borrow_mut().push(header);
        self.footer_fields.borrow_mut().push(footer);
        self.page_number_fields.borrow_mut().push(page_label);
        self.page_views.borrow_mut().push(tv);
    }

    fn relayout(&self) {
        let count = self.page_views.borrow().len().max(1);
        let width = (self.metrics.page_width + 64.0).max(self.scroll_view.frame().size.width);
        let height = self.metrics.document_height(count);
        self.document_view
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)));
        // Only assign documentView once — re-setting it resets scroll position.
        if self.scroll_view.documentView().is_none() {
            self.scroll_view.setDocumentView(Some(&self.document_view));
        }

        for (i, tv) in self.page_views.borrow().iter().enumerate() {
            let page_frame = self.page_frame(i);
            if let Some(back) = self.page_backs.borrow().get(i) {
                back.setFrame(page_frame);
            }
            if let Some(h) = self.header_fields.borrow().get(i) {
                h.setFrame(NSRect::new(
                    NSPoint::new(
                        page_frame.origin.x + self.metrics.margin_left,
                        page_frame.origin.y + self.metrics.header_height * 0.35,
                    ),
                    NSSize::new(self.metrics.content_width, 16.0),
                ));
            }
            if let Some(f) = self.footer_fields.borrow().get(i) {
                f.setStringValue(&NSString::from_str(self.footer.borrow().as_str()));
                f.setFrame(NSRect::new(
                    NSPoint::new(
                        page_frame.origin.x + self.metrics.margin_left,
                        page_frame.origin.y + self.metrics.page_height
                            - self.metrics.margin_bottom
                            + 4.0,
                    ),
                    NSSize::new(self.metrics.content_width * 0.7, 16.0),
                ));
            }
            if let Some(pn) = self.page_number_fields.borrow().get(i) {
                pn.setStringValue(&NSString::from_str(&format!("— {}", i + 1)));
                pn.setFrame(NSRect::new(
                    NSPoint::new(
                        page_frame.origin.x
                            + self.metrics.margin_left
                            + self.metrics.content_width * 0.72,
                        page_frame.origin.y + self.metrics.page_height
                            - self.metrics.margin_bottom
                            + 4.0,
                    ),
                    NSSize::new(self.metrics.content_width * 0.28, 16.0),
                ));
            }
            tv.setFrame(self.content_frame_in_page(page_frame));
        }
    }

    pub fn ensure_enough_pages(&self, mtm: MainThreadMarker) {
        // Prefer TextKit's idea of how many containers are filled over a char estimate.
        let needed = self.pages_needed_from_layout().clamp(1, MAX_PAGES);

        while self.page_views.borrow().len() < needed {
            let ptr = self as *const PaginatedEditor as *mut PaginatedEditor;
            unsafe {
                (*ptr).add_page(mtm);
            }
        }
        // Refresh primary reference (undo lives on page 0).
        if let Some(first) = self.page_views.borrow().first() {
            let ptr = self as *const PaginatedEditor as *mut PaginatedEditor;
            unsafe {
                (*ptr).primary_text_view = first.clone();
            }
        }
        self.relayout();
    }

    /// How many page containers are required for the current laid-out glyphs.
    /// Always keeps one spare empty page so Return at end-of-page has somewhere to flow.
    fn pages_needed_from_layout(&self) -> usize {
        let lm = &self.layout_manager;
        let storage_len = self.text_storage.length();
        if storage_len == 0 {
            return 1;
        }
        lm.ensureLayoutForCharacterRange(objc2_foundation::NSRange {
            location: 0,
            length: storage_len,
        });

        let containers = lm.textContainers();
        let n_containers = containers.count().max(1);
        let glyph_len = lm.numberOfGlyphs();
        if glyph_len == 0 {
            // Newline-only growth may not produce glyphs yet — fall back to estimate.
            let chars = storage_len as f64;
            let approx =
                (self.metrics.content_height / 16.0) * (self.metrics.content_width / 7.5);
            return ((chars / approx.max(1.0)).ceil() as usize + 1).max(1);
        }

        let last_glyph = glyph_len - 1;
        let container = unsafe {
            lm.textContainerForGlyphAtIndex_effectiveRange(last_glyph, std::ptr::null_mut())
        };
        let last_idx = match container {
            Some(c) => {
                let mut idx = 0usize;
                for i in 0..containers.count() {
                    if std::ptr::eq(
                        containers.objectAtIndex(i).as_ref() as *const NSTextContainer,
                        c.as_ref() as *const NSTextContainer,
                    ) {
                        idx = i;
                        break;
                    }
                }
                idx
            }
            None => n_containers, // needs a brand-new container
        };
        // last_idx is 0-based index of the container holding the final glyph.
        // Request one spare page after it (stable once the spare exists).
        last_idx + 2
    }

    fn update_header_footer_labels(&self) {
        for f in self.footer_fields.borrow().iter() {
            f.setStringValue(&NSString::from_str(self.footer.borrow().as_str()));
        }
        for h in self.header_fields.borrow().iter() {
            h.setStringValue(&NSString::from_str(self.header.borrow().as_str()));
        }
        for (i, pn) in self.page_number_fields.borrow().iter().enumerate() {
            pn.setStringValue(&NSString::from_str(&format!("— {}", i + 1)));
        }
    }

    /// Read editable header/footer fields (uses first page as source of truth).
    pub fn header_footer_text(&self) -> (String, String) {
        let header = self
            .header_fields
            .borrow()
            .first()
            .map(|f| f.stringValue().to_string())
            .unwrap_or_else(|| self.header.borrow().clone());
        let footer = self
            .footer_fields
            .borrow()
            .first()
            .map(|f| f.stringValue().to_string())
            .unwrap_or_else(|| self.footer.borrow().clone());
        *self.header.borrow_mut() = header.clone();
        *self.footer.borrow_mut() = footer.clone();
        // Propagate to other page fields
        for h in self.header_fields.borrow().iter().skip(1) {
            h.setStringValue(&NSString::from_str(&header));
        }
        for f in self.footer_fields.borrow().iter().skip(1) {
            f.setStringValue(&NSString::from_str(&footer));
        }
        (header, footer)
    }
}

#[derive(Default)]
struct FlippedIvars;

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "WordRsFlippedView"]
    #[ivars = FlippedIvars]
    struct FlippedView;

    unsafe impl NSObjectProtocol for FlippedView {}

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FlippedIvars);
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Default)]
struct ObserverIvars {
    editor_ptr: RefCell<usize>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "WordRsPageLayoutObserver"]
    #[ivars = ObserverIvars]
    struct PageLayoutObserver;

    unsafe impl NSObjectProtocol for PageLayoutObserver {}

    impl PageLayoutObserver {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _note: Option<&NSNotification>) {
            // Never mutate text containers / frames during processEditing — that
            // re-enters TextKit and aborts (seen as objc "unknown class" on Return).
            // Coalesce to the next run-loop turn.
            unsafe {
                let _: () = msg_send![
                    class!(NSObject),
                    cancelPreviousPerformRequestsWithTarget: self
                    selector: sel!(deferredEnsurePages:)
                    object: Option::<&NSObject>::None
                ];
                let _: () = msg_send![
                    self,
                    performSelector: sel!(deferredEnsurePages:)
                    withObject: Option::<&NSObject>::None
                    afterDelay: 0.0f64
                ];
            }
        }

        #[unsafe(method(deferredEnsurePages:))]
        fn deferred_ensure_pages(&self, _sender: Option<&NSObject>) {
            let ptr = *self.ivars().editor_ptr.borrow();
            if ptr == 0 {
                return;
            }
            let editor = unsafe { &*(ptr as *const PaginatedEditor) };
            editor.ensure_enough_pages(self.mtm());
        }
    }
);

impl PageLayoutObserver {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ObserverIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    fn set_editor_ptr(&self, ptr: usize) {
        *self.ivars().editor_ptr.borrow_mut() = ptr;
    }
}
