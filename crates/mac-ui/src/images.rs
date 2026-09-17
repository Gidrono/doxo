//! NSTextAttachment helpers for inline images.

use layout_bridge::BridgedImage;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::AnyThread;
use objc2_app_kit::{
    NSAttributedStringAttachmentConveniences, NSImage, NSTextAttachment, NSAttachmentAttributeName,
};
use objc2_foundation::{
    ns_string, NSAttributedString, NSAttributedStringKey, NSData, NSDictionary, NSMutableAttributedString,
    NSMutableDictionary, NSSize, NSString,
};

/// Create an attributed string containing a single image attachment.
pub fn image_to_attributed(img: &BridgedImage) -> Retained<NSMutableAttributedString> {
    let attachment = attachment_from_image(img);
    let piece = NSAttributedString::attributedStringWithAttachment(&attachment);
    let out = NSMutableAttributedString::new();
    out.appendAttributedString(&piece);
    out
}

pub fn attachment_from_image(img: &BridgedImage) -> Retained<NSTextAttachment> {
    let data = NSData::with_bytes(&img.data);
    let uti: &NSString = match img.content_type.as_str() {
        "image/jpeg" => ns_string!("public.jpeg"),
        "image/gif" => ns_string!("public.gif"),
        _ => ns_string!("public.png"),
    };
    let attachment =
        NSTextAttachment::initWithData_ofType(NSTextAttachment::alloc(), Some(&data), Some(uti));
    if let Some(nsimg) = NSImage::initWithData(NSImage::alloc(), &data) {
        let w = img
            .width_px
            .unwrap_or_else(|| nsimg.size().width.max(1.0) as u32);
        let h = img
            .height_px
            .unwrap_or_else(|| nsimg.size().height.max(1.0) as u32);
        let max_w = 480.0_f64;
        let scale = if w as f64 > max_w {
            max_w / w as f64
        } else {
            1.0
        };
        nsimg.setSize(NSSize::new(w as f64 * scale, h as f64 * scale));
        attachment.setImage(Some(&nsimg));
    }
    attachment.setContents(Some(&data));
    attachment
}

/// Extract a BridgedImage from attributes at a location, if an attachment is present.
pub fn image_from_attrs(
    attrs: &NSDictionary<NSAttributedStringKey, AnyObject>,
) -> Option<BridgedImage> {
    let obj = unsafe { attrs.objectForKey(NSAttachmentAttributeName)? };
    let attachment = obj.downcast::<NSTextAttachment>().ok()?;
    let data = if let Some(contents) = attachment.contents() {
        contents.to_vec()
    } else if let Some(image) = attachment.image() {
        image.TIFFRepresentation()?.to_vec()
    } else {
        return None;
    };
    if data.is_empty() {
        return None;
    }
    let (w, h) = attachment
        .image()
        .map(|img| {
            let sz = img.size();
            (
                Some(sz.width.max(1.0) as u32),
                Some(sz.height.max(1.0) as u32),
            )
        })
        .unwrap_or((None, None));
    let content_type = if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/png"
    };
    Some(BridgedImage {
        content_type: content_type.into(),
        data,
        width_px: w,
        height_px: h,
        relationship_id: None,
    })
}

#[allow(dead_code)]
pub fn attachment_attribute_dict(
    attachment: &NSTextAttachment,
) -> Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> {
    let dict: Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> =
        NSMutableDictionary::new();
    unsafe {
        dict.setObject_forKey(
            attachment,
            ProtocolObject::from_ref(NSAttachmentAttributeName),
        );
    }
    dict
}
