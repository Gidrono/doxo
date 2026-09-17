//! Header / footer part XML (`word/header*.xml`, `word/footer*.xml`).

use crate::WmlError;

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Extract concatenated plain text from a header or footer part.
pub fn extract_hf_plain_text(xml: &[u8]) -> String {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut in_t = false;
    let mut buf = Vec::new();
    let mut para_sep = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                let local = local_name_bytes(e.name().as_ref());
                if local == "t" {
                    in_t = true;
                } else if local == "p" && !out.is_empty() {
                    para_sep = true;
                } else if local == "br" || local == "cr" {
                    out.push('\n');
                } else if local == "tab" {
                    out.push('\t');
                }
            }
            Ok(quick_xml::events::Event::Empty(e)) => {
                let local = local_name_bytes(e.name().as_ref());
                if local == "br" || local == "cr" {
                    out.push('\n');
                } else if local == "tab" {
                    out.push('\t');
                }
            }
            Ok(quick_xml::events::Event::Text(t)) if in_t => {
                if let Ok(s) = t.unescape() {
                    if para_sep && !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    para_sep = false;
                    out.push_str(&s);
                }
            }
            Ok(quick_xml::events::Event::End(e)) => {
                let local = local_name_bytes(e.name().as_ref());
                if local == "t" {
                    in_t = false;
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Build a minimal `w:hdr` part from plain text (paragraphs split on `\n`).
pub fn serialize_header_xml(text: &str) -> String {
    serialize_hf_xml("hdr", text)
}

/// Build a minimal `w:ftr` part from plain text (paragraphs split on `\n`).
pub fn serialize_footer_xml(text: &str) -> String {
    serialize_hf_xml("ftr", text)
}

fn serialize_hf_xml(root: &str, text: &str) -> String {
    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    out.push('\n');
    out.push_str(&format!(
        r#"<w:{root} xmlns:w="{W_NS}" xmlns:r="{R_NS}">"#
    ));
    let paras: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.split('\n').collect()
    };
    for para in paras {
        out.push_str("<w:p><w:r><w:t");
        if para.starts_with(' ') || para.ends_with(' ') || para.contains('\t') {
            out.push_str(r#" xml:space="preserve""#);
        }
        out.push('>');
        out.push_str(&escape_xml(para));
        out.push_str("</w:t></w:r></w:p>");
    }
    out.push_str(&format!("</w:{root}>"));
    out
}

/// Prefer preserving original header/footer XML when the plain text is unchanged;
/// otherwise rewrite `w:t` contents when a single text node matches, else regenerate.
pub fn merge_hf_xml(
    source_xml: Option<&str>,
    new_text: &str,
    is_header: bool,
) -> Result<String, WmlError> {
    if let Some(src) = source_xml {
        let old = extract_hf_plain_text(src.as_bytes());
        if old == new_text {
            return Ok(src.to_string());
        }
        if let Some(updated) = replace_hf_plain_text(src, &old, new_text) {
            return Ok(updated);
        }
    }
    Ok(if is_header {
        serialize_header_xml(new_text)
    } else {
        serialize_footer_xml(new_text)
    })
}

/// Replace all `w:t` text with `new_text` distributed into the first text run,
/// clearing subsequent `w:t` nodes. Falls back to None on structural mismatch.
fn replace_hf_plain_text(src: &str, old_text: &str, new_text: &str) -> Option<String> {
    if old_text == new_text {
        return Some(src.to_string());
    }
    // Simple path: single w:t containing the entire old plain text (no newlines in part).
    if !old_text.contains('\n') {
        if let Some(idx) = src.find("<w:t") {
            let after = &src[idx..];
            let close_gt = after.find('>')?;
            let content_start = idx + close_gt + 1;
            let content_end = src[content_start..].find("</w:t>")? + content_start;
            let current = &src[content_start..content_end];
            // Unescape comparison is approximate; match escaped or raw
            if unescape_basic(current) == old_text || current == old_text {
                let mut out = String::with_capacity(src.len() + new_text.len());
                out.push_str(&src[..content_start]);
                out.push_str(&escape_xml(new_text));
                out.push_str(&src[content_end..]);
                return Some(out);
            }
        }
    }
    None
}

fn unescape_basic(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn local_name_bytes(qname: &[u8]) -> String {
    let s = std::str::from_utf8(qname).unwrap_or("");
    s.rsplit(':').next().unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple_header() {
        let xml = serialize_header_xml("Hello");
        assert!(xml.contains("<w:hdr"));
        assert_eq!(extract_hf_plain_text(xml.as_bytes()), "Hello");
    }

    #[test]
    fn merge_preserves_unknown_when_text_unchanged() {
        let src = r#"<?xml version="1.0"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:p><w:pPr><w:pStyle w:val="Header"/><w:keepNext/></w:pPr>
  <w:r><w:t>Title</w:t></w:r></w:p>
</w:hdr>"#;
        let out = merge_hf_xml(Some(src), "Title", true).unwrap();
        assert!(out.contains("keepNext"));
        assert!(out.contains("pStyle"));
    }

    #[test]
    fn merge_rewrites_single_t() {
        let src = r#"<?xml version="1.0"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>Old</w:t></w:r></w:p>
</w:hdr>"#;
        let out = merge_hf_xml(Some(src), "New", true).unwrap();
        assert!(out.contains(">New</w:t>"));
        assert!(out.contains("keepNext"));
    }
}
