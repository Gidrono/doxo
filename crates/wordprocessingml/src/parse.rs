//! Streaming parse of `word/document.xml` into the semantic model.

use document_model::{
    Alignment, Block, Document, ListKind, NodeId, PageSetup, Paragraph, ParagraphProperties, Run,
    RunContent, RunProperties, Section, TableCell, TableRow,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::WmlError;

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

pub fn parse_document_xml(xml: &[u8]) -> Result<Document, WmlError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut doc = Document::default();
    let mut section = Section {
        id: NodeId::new(),
        page: PageSetup::default(),
        blocks: Vec::new(),
        header: None,
        footer: None,
        header_r_id: None,
        footer_r_id: None,
        header_source_xml: None,
        footer_source_xml: None,
    };

    let mut buf = Vec::new();
    let mut in_body = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(&e);
                if local == "body" {
                    in_body = true;
                } else if in_body && local == "p" {
                    let para = parse_paragraph(&mut reader, &e)?;
                    section.blocks.push(Block::Paragraph(para));
                } else if in_body && local == "sectPr" {
                    let (page, header_r_id, footer_r_id) = parse_sect_pr(&mut reader, &e)?;
                    section.page = page;
                    section.header_r_id = header_r_id;
                    section.footer_r_id = footer_r_id;
                } else if in_body && local == "tbl" {
                    let raw = capture_element(&mut reader, &e)?;
                    // Tier A: keep structured table when possible; always retain raw for fidelity.
                    section.blocks.push(Block::Table(document_model::Table {
                        id: NodeId::new(),
                        rows: parse_table_rows_from_fragment(&raw),
                        source_xml: Some(raw),
                        edited: false,
                    }));
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(&e);
                if in_body && local == "sectPr" {
                    // empty sectPr — keep defaults
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    if section.blocks.is_empty() {
        section.blocks.push(Block::Paragraph(Paragraph::empty()));
    }
    doc.sections.push(section);
    Ok(doc)
}

fn parse_paragraph(
    reader: &mut Reader<&[u8]>,
    _start: &BytesStart<'_>,
) -> Result<Paragraph, WmlError> {
    let mut para = Paragraph {
        id: NodeId::new(),
        style_id: None,
        properties: ParagraphProperties::default(),
        runs: Vec::new(),
        source_xml: None,
        unknown_p_pr: Vec::new(),
        unknown_children: Vec::new(),
    };
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(&e);
                match local.as_str() {
                    "pPr" => {
                        let (props, unknown, style) = parse_p_pr(reader, &e)?;
                        para.properties = props;
                        para.unknown_p_pr = unknown;
                        if style.is_some() {
                            para.style_id = style;
                        }
                    }
                    "r" => {
                        para.runs.push(parse_run(reader, &e)?);
                    }
                    "hyperlink" => {
                        let nested = parse_container_runs(reader, &e)?;
                        para.runs.extend(nested);
                    }
                    other => {
                        let raw = capture_element(reader, &e)?;
                        para.unknown_children
                            .push(format!("<!--unknown:{other}-->{raw}"));
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(&e);
                if local == "pPr" {
                    // empty
                } else if local != "r" {
                    para.unknown_children
                        .push(format!("<w:{local}/>"));
                }
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == "p" => break,
            Ok(Event::Eof) => {
                return Err(WmlError::Structure("unexpected eof in paragraph".into()));
            }
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    if para.runs.is_empty() {
        para.runs.push(Run::text(""));
    }
    Ok(para)
}

fn parse_container_runs(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<Vec<Run>, WmlError> {
    let end_name = local_name(start);
    let mut runs = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(&e) == "r" {
                    runs.push(parse_run(reader, &e)?);
                } else {
                    let _ = capture_element(reader, &e)?;
                }
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == end_name => break,
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    Ok(runs)
}

fn parse_p_pr(
    reader: &mut Reader<&[u8]>,
    _start: &BytesStart<'_>,
) -> Result<(ParagraphProperties, Vec<String>, Option<String>), WmlError> {
    let mut props = ParagraphProperties::default();
    let mut unknown = Vec::new();
    let mut style_id = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(&e);
                match local.as_str() {
                    "bidi" | "jc" | "numPr" | "ilvl" | "numId" => {
                        apply_p_pr_tag(&mut props, &e);
                        let _ = capture_element(reader, &e)?;
                    }
                    "pStyle" => {
                        style_id = attr_val(&e, "val");
                        let _ = capture_element(reader, &e)?;
                    }
                    other => {
                        let raw = capture_element(reader, &e)?;
                        unknown.push(format!("<!--pPr:{other}-->{raw}"));
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(&e);
                match local.as_str() {
                    "bidi" | "jc" | "numPr" | "ilvl" | "numId" => apply_p_pr_tag(&mut props, &e),
                    "pStyle" => style_id = attr_val(&e, "val"),
                    other => unknown.push(format!("<w:{other}/>")),
                }
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == "pPr" => break,
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    Ok((props, unknown, style_id))
}

fn apply_p_pr_tag(props: &mut ParagraphProperties, e: &BytesStart<'_>) {
    match local_name(e).as_str() {
        "bidi" => props.bidirectional = true,
        "jc" => {
            if let Some(v) = attr_val(e, "val") {
                props.alignment = parse_alignment(&v);
            }
        }
        "numPr" => {
            // Presence of numbering properties ⇒ treat as list (bullet default for M2).
            if props.list_kind.is_none() {
                props.list_kind = Some(ListKind::Bullet);
            }
        }
        "ilvl" => {
            if let Some(v) = attr_val(e, "val") {
                props.list_level = v.parse().unwrap_or(0);
            }
        }
        "numId" => {
            if let Some(v) = attr_val(e, "val") {
                // Heuristic: numId 1 → bullet, others → numbered
                props.list_kind = Some(if v == "1" {
                    ListKind::Bullet
                } else {
                    ListKind::Numbered
                });
            }
        }
        _ => {}
    }
}

/// Best-effort extract of cell paragraphs from a captured table fragment.
fn parse_table_rows_from_fragment(raw: &str) -> Vec<TableRow> {
    let mut rows = Vec::new();
    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut current_row: Option<TableRow> = None;
    let mut cell_paras: Vec<Paragraph> = Vec::new();
    let mut current_para_text = String::new();
    let mut in_t = false;
    let mut in_tc = false;
    let mut grid_span = 1u32;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local = local_name(&e);
                match local.as_str() {
                    "tr" => current_row = Some(TableRow { cells: Vec::new() }),
                    "tc" => {
                        in_tc = true;
                        cell_paras.clear();
                        current_para_text.clear();
                        grid_span = 1;
                    }
                    "gridSpan" => {
                        if let Some(v) = attr_val(&e, "val").and_then(|s| s.parse().ok()) {
                            grid_span = v;
                        }
                    }
                    "p" if in_tc => current_para_text.clear(),
                    "t" => in_t = true,
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_t => {
                if let Ok(s) = t.unescape() {
                    current_para_text.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                let local = local_name_from_qname(e.name().as_ref());
                match local.as_str() {
                    "t" => in_t = false,
                    "p" if in_tc => {
                        cell_paras.push(Paragraph::from_text(current_para_text.clone()));
                        current_para_text.clear();
                    }
                    "tc" => {
                        in_tc = false;
                        if cell_paras.is_empty() {
                            cell_paras.push(Paragraph::from_text(current_para_text.clone()));
                        }
                        if let Some(row) = current_row.as_mut() {
                            row.cells.push(TableCell {
                                paragraphs: std::mem::take(&mut cell_paras),
                                grid_span,
                            });
                        }
                        current_para_text.clear();
                    }
                    "tr" => {
                        if let Some(row) = current_row.take() {
                            rows.push(row);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    rows
}

fn parse_run(reader: &mut Reader<&[u8]>, _start: &BytesStart<'_>) -> Result<Run, WmlError> {
    let mut run = Run {
        id: NodeId::new(),
        properties: RunProperties::default(),
        content: RunContent::Text(String::new()),
        source_xml: None,
        unknown_r_pr: Vec::new(),
    };
    let mut text = String::new();
    let mut buf = Vec::new();
    let mut saw_break = false;
    let mut saw_tab = false;
    let mut image_rel: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(&e);
                match local.as_str() {
                    "rPr" => {
                        let (props, unknown) = parse_r_pr(reader, &e)?;
                        run.properties = props;
                        run.unknown_r_pr = unknown;
                    }
                    "t" => {
                        text.push_str(&read_text_element(reader, &e)?);
                    }
                    "br" => {
                        saw_break = true;
                        let _ = capture_element(reader, &e)?;
                    }
                    "tab" => {
                        saw_tab = true;
                        let _ = capture_element(reader, &e)?;
                    }
                    "drawing" | "pict" => {
                        let raw = capture_element(reader, &e)?;
                        if let Some(rid) = extract_blip_embed(&raw) {
                            image_rel = Some(rid);
                        } else {
                            run.content = RunContent::Unsupported(raw);
                        }
                    }
                    _ => {
                        let _ = capture_element(reader, &e)?;
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(&e);
                match local.as_str() {
                    "br" => saw_break = true,
                    "tab" => saw_tab = true,
                    "t" => {}
                    _ => {}
                }
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == "r" => break,
            Ok(Event::Eof) => {
                return Err(WmlError::Structure("unexpected eof in run".into()));
            }
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    if let Some(rid) = image_rel {
        run.content = RunContent::Image(document_model::InlineImage {
            content_type: "image/png".into(),
            data: Vec::new(), // filled by persistence from relationships
            width_px: None,
            height_px: None,
            relationship_id: Some(rid),
        });
    } else if !matches!(run.content, RunContent::Unsupported(_)) {
        run.content = if saw_break && text.is_empty() {
            RunContent::Break
        } else if saw_tab && text.is_empty() {
            RunContent::Tab
        } else {
            RunContent::Text(text)
        };
    }
    Ok(run)
}

fn extract_blip_embed(raw: &str) -> Option<String> {
    // Look for r:embed="rIdN" or r:embed='rIdN'
    for key in ["r:embed=\"", "r:embed='", "embed=\""] {
        if let Some(i) = raw.find(key) {
            let rest = &raw[i + key.len()..];
            let end = rest.find(['"', '\'']).unwrap_or(0);
            if end > 0 {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

fn parse_r_pr(
    reader: &mut Reader<&[u8]>,
    _start: &BytesStart<'_>,
) -> Result<(RunProperties, Vec<String>), WmlError> {
    let mut props = RunProperties::default();
    let mut unknown = Vec::new();
    let mut buf = Vec::new();
    let known = [
        "b", "bCs", "i", "iCs", "u", "strike", "rtl", "rFonts", "sz", "szCs", "color", "lang",
    ];
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(&e);
                if known.contains(&local.as_str()) {
                    apply_r_pr_tag(&mut props, &e);
                    let _ = capture_element(reader, &e)?;
                } else {
                    let raw = capture_element(reader, &e)?;
                    unknown.push(format!("<!--rPr:{local}-->{raw}"));
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(&e);
                if known.contains(&local.as_str()) {
                    apply_r_pr_tag(&mut props, &e);
                } else {
                    unknown.push(format!("<w:{local}/>"));
                }
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == "rPr" => break,
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    Ok((props, unknown))
}

fn apply_r_pr_tag(props: &mut RunProperties, e: &BytesStart<'_>) {
    match local_name(e).as_str() {
        "b" | "bCs" => props.bold = true,
        "i" | "iCs" => props.italic = true,
        "u" => props.underline = true,
        "strike" => props.strike = true,
        "rtl" => props.rtl = true,
        "rFonts" => {
            if let Some(v) = attr_val(e, "ascii") {
                props.font_ascii = Some(v);
            }
            if let Some(v) = attr_val(e, "hAnsi") {
                props.font_h_ansi = Some(v);
            }
            if let Some(v) = attr_val(e, "cs") {
                props.font_cs = Some(v);
            }
            if let Some(v) = attr_val(e, "eastAsia") {
                props.font_east_asia = Some(v);
            }
        }
        "sz" | "szCs" => {
            if let Some(v) = attr_val(e, "val") {
                if let Ok(n) = v.parse::<u32>() {
                    props.font_size_half_points = Some(n);
                }
            }
        }
        "color" => {
            if let Some(v) = attr_val(e, "val") {
                if v != "auto" {
                    props.color_hex = Some(v);
                }
            }
        }
        "lang" => {
            if let Some(v) = attr_val(e, "val") {
                props.language = Some(v);
            } else if let Some(v) = attr_val(e, "bidi") {
                props.language = Some(v);
            }
        }
        _ => {}
    }
}

fn parse_sect_pr(
    reader: &mut Reader<&[u8]>,
    _start: &BytesStart<'_>,
) -> Result<(PageSetup, Option<String>, Option<String>), WmlError> {
    let mut page = PageSetup::default();
    let mut header_r_id = None;
    let mut footer_r_id = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                apply_sect_pr_tag(&mut page, &e);
                apply_hf_reference(&e, &mut header_r_id, &mut footer_r_id);
                let _ = capture_element(reader, &e)?;
            }
            Ok(Event::Empty(e)) => {
                apply_sect_pr_tag(&mut page, &e);
                apply_hf_reference(&e, &mut header_r_id, &mut footer_r_id);
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == "sectPr" => break,
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    Ok((page, header_r_id, footer_r_id))
}

fn apply_hf_reference(
    e: &BytesStart<'_>,
    header_r_id: &mut Option<String>,
    footer_r_id: &mut Option<String>,
) {
    let local = local_name(e);
    match local.as_str() {
        "headerReference" => {
            let ty = attr_val(e, "type").unwrap_or_else(|| "default".into());
            if ty == "default" || header_r_id.is_none() {
                if let Some(id) = attr_val(e, "id") {
                    *header_r_id = Some(id);
                }
            }
        }
        "footerReference" => {
            let ty = attr_val(e, "type").unwrap_or_else(|| "default".into());
            if ty == "default" || footer_r_id.is_none() {
                if let Some(id) = attr_val(e, "id") {
                    *footer_r_id = Some(id);
                }
            }
        }
        _ => {}
    }
}

fn apply_sect_pr_tag(page: &mut PageSetup, e: &BytesStart<'_>) {
    match local_name(e).as_str() {
        "pgSz" => {
            if let Some(w) = attr_val(e, "w").and_then(|s| s.parse().ok()) {
                page.width_twips = w;
            }
            if let Some(h) = attr_val(e, "h").and_then(|s| s.parse().ok()) {
                page.height_twips = h;
            }
        }
        "pgMar" => {
            if let Some(v) = attr_val(e, "top").and_then(|s| s.parse().ok()) {
                page.margin_top_twips = v;
            }
            if let Some(v) = attr_val(e, "bottom").and_then(|s| s.parse().ok()) {
                page.margin_bottom_twips = v;
            }
            if let Some(v) = attr_val(e, "left").and_then(|s| s.parse().ok()) {
                page.margin_left_twips = v;
            }
            if let Some(v) = attr_val(e, "right").and_then(|s| s.parse().ok()) {
                page.margin_right_twips = v;
            }
        }
        _ => {}
    }
}

fn read_text_element(
    reader: &mut Reader<&[u8]>,
    _start: &BytesStart<'_>,
) -> Result<String, WmlError> {
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(t)) => {
                text.push_str(&t.unescape().map_err(|e| WmlError::Xml(e.to_string()))?);
            }
            Ok(Event::CData(t)) => {
                text.push_str(std::str::from_utf8(&t).map_err(|e| WmlError::Xml(e.to_string()))?);
            }
            Ok(Event::End(e)) if local_name_from_qname(e.name().as_ref()) == "t" => break,
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    Ok(text)
}

/// Capture an element and its children as a UTF-8 XML fragment (best-effort).
fn capture_element(reader: &mut Reader<&[u8]>, start: &BytesStart<'_>) -> Result<String, WmlError> {
    let mut out = format!("<{}", qname_str(start.name().as_ref()));
    out.push_str(&attrs_to_string(start));
    out.push('>');
    let end_local = local_name(start);
    let mut depth = 1u32;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                out.push('<');
                out.push_str(&qname_str(e.name().as_ref()));
                out.push_str(&attrs_to_string(&e));
                out.push('>');
            }
            Ok(Event::Empty(e)) => {
                out.push('<');
                out.push_str(&qname_str(e.name().as_ref()));
                out.push_str(&attrs_to_string(&e));
                out.push_str("/>");
            }
            Ok(Event::End(e)) => {
                let n = qname_str(e.name().as_ref());
                out.push_str(&format!("</{n}>"));
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Text(t)) => {
                let s = t.unescape().map_err(|e| WmlError::Xml(e.to_string()))?;
                out.push_str(&escape_xml(&s));
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(WmlError::Xml(e.to_string())),
            _ => {}
        }
        buf.clear();
    }
    let _ = end_local;
    Ok(out)
}

fn qname_str(qname: &[u8]) -> String {
    String::from_utf8_lossy(qname).into_owned()
}

fn attrs_to_string(e: &BytesStart<'_>) -> String {
    let mut out = String::new();
    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref());
        let val = attr
            .unescape_value()
            .map(|v| v.into_owned())
            .unwrap_or_default();
        out.push(' ');
        out.push_str(&key);
        out.push_str("=\"");
        out.push_str(&escape_xml(&val));
        out.push('"');
    }
    out
}

fn parse_alignment(val: &str) -> Option<Alignment> {
    match val {
        "left" => Some(Alignment::Left),
        "center" => Some(Alignment::Center),
        "right" => Some(Alignment::Right),
        "both" | "justify" => Some(Alignment::Justify),
        "start" => Some(Alignment::Start),
        "end" => Some(Alignment::End),
        _ => None,
    }
}

fn local_name(e: &BytesStart<'_>) -> String {
    local_name_from_qname(e.name().as_ref())
}

fn local_name_from_qname(qname: &[u8]) -> String {
    let s = std::str::from_utf8(qname).unwrap_or("");
    s.rsplit(':').next().unwrap_or(s).to_string()
}

fn attr_val(e: &BytesStart<'_>, key: &str) -> Option<String> {
    for attr in e.attributes().flatten() {
        let key_bytes = attr.key.as_ref();
        let local = local_name_from_qname(key_bytes);
        if local == key {
            let v = attr
                .unescape_value()
                .ok()?
                .into_owned();
            return Some(v);
        }
    }
    // Also try prefixed w:key via full scan already covered by local name.
    let _ = W_NS;
    None
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
