//! Home toolbar: formatting, styles, lists, RTL/LTR, open/save.

use layout_bridge::STYLE_PRESETS;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSPopUpButton, NSStackView, NSStackViewDistribution, NSTextField,
    NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{ns_string, MainThreadMarker, NSEdgeInsets, NSObject, NSSize, NSString};

pub struct HomeToolbar {
    pub root: Retained<NSView>,
    #[allow(dead_code)]
    pub style_popup: Retained<NSPopUpButton>,
    #[allow(dead_code)]
    pub size_field: Retained<NSTextField>,
}

impl HomeToolbar {
    pub fn build(mtm: MainThreadMarker, target: &NSObject) -> Self {
        let stack = NSStackView::new(mtm);
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        stack.setSpacing(6.0);
        stack.setDistribution(NSStackViewDistribution::GravityAreas);
        stack.setEdgeInsets(NSEdgeInsets {
            top: 6.0,
            left: 10.0,
            bottom: 6.0,
            right: 10.0,
        });

        let open_btn = make_button(mtm, "Open", sel!(openDocument:), target);
        let save_btn = make_button(mtm, "Save", sel!(saveDocument:), target);
        let bold = make_button(mtm, "B", sel!(toggleBold:), target);
        let italic = make_button(mtm, "I", sel!(toggleItalic:), target);
        let underline = make_button(mtm, "U", sel!(toggleUnderline:), target);
        let align_left = make_button(mtm, "L", sel!(alignLeft:), target);
        let align_center = make_button(mtm, "C", sel!(alignCenter:), target);
        let align_right = make_button(mtm, "R", sel!(alignRight:), target);
        let rtl = make_button(mtm, "RTL", sel!(setRtl:), target);
        let ltr = make_button(mtm, "LTR", sel!(setLtr:), target);
        let bullet = make_button(mtm, "• List", sel!(insertBulletList:), target);
        let numbered = make_button(mtm, "1. List", sel!(insertNumberedList:), target);
        let table = make_button(mtm, "Table", sel!(insertTable:), target);
        let image = make_button(mtm, "Image", sel!(insertImage:), target);

        let style_label = NSTextField::labelWithString(ns_string!("Style"), mtm);
        style_label.setEditable(false);

        let style_popup = unsafe { NSPopUpButton::new(mtm) };
        unsafe {
            style_popup.setTarget(Some(target));
            style_popup.setAction(Some(sel!(applyStyle:)));
            style_popup.removeAllItems();
            for preset in STYLE_PRESETS {
                style_popup.addItemWithTitle(&NSString::from_str(preset.name));
            }
            style_popup.setFrameSize(NSSize::new(110.0, 26.0));
        }
        style_popup.setRefusesFirstResponder(true);

        let font_label = NSTextField::labelWithString(ns_string!("Size"), mtm);
        font_label.setEditable(false);
        let size_field = NSTextField::new(mtm);
        size_field.setStringValue(ns_string!("12"));
        size_field.setFrameSize(NSSize::new(44.0, 24.0));
        unsafe {
            size_field.setTarget(Some(target));
            size_field.setAction(Some(sel!(changeFontSize:)));
        }

        for v in [
            open_btn.as_ref() as &NSView,
            save_btn.as_ref(),
            style_label.as_ref(),
            style_popup.as_ref(),
            bold.as_ref(),
            italic.as_ref(),
            underline.as_ref(),
            font_label.as_ref(),
            size_field.as_ref(),
            align_left.as_ref(),
            align_center.as_ref(),
            align_right.as_ref(),
            rtl.as_ref(),
            ltr.as_ref(),
            bullet.as_ref(),
            numbered.as_ref(),
            table.as_ref(),
            image.as_ref(),
        ] {
            stack.addView_inGravity(v, objc2_app_kit::NSStackViewGravity::Leading);
        }

        // Keep buttons alive by leaking into stack subviews (retained by hierarchy).
        // Also retain key controls on Self.
        std::mem::forget(open_btn);
        std::mem::forget(save_btn);
        std::mem::forget(bold);
        std::mem::forget(italic);
        std::mem::forget(underline);
        std::mem::forget(align_left);
        std::mem::forget(align_center);
        std::mem::forget(align_right);
        std::mem::forget(rtl);
        std::mem::forget(ltr);
        std::mem::forget(bullet);
        std::mem::forget(numbered);
        std::mem::forget(table);
        std::mem::forget(image);
        std::mem::forget(style_label);
        std::mem::forget(font_label);

        Self {
            root: stack.into_super(),
            style_popup,
            size_field,
        }
    }
}

fn make_button(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    target: &NSObject,
) -> Retained<NSButton> {
    let btn = NSButton::new(mtm);
    btn.setTitle(&NSString::from_str(title));
    btn.setBezelStyle(NSBezelStyle::FlexiblePush);
    // Keep typing focus in the document after clicking formatting controls.
    btn.setRefusesFirstResponder(true);
    unsafe {
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
    }
    btn.setFrameSize(NSSize::new(64.0, 28.0));
    btn
}
