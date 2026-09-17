//! Serialize the semantic document model back to `word/document.xml`.

use document_model::{
    Alignment, Block, Document, ListKind, Paragraph, Run, RunContent, RunProperties, Table,
};

use crate::WmlError;

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WP_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PIC_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

pub fn serialize_document_xml(doc: &Document) -> Result<String, WmlError> {
    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    out.push('\n');
    out.push_str(&format!(
        r#"<w:document xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:wp="{WP_NS}" xmlns:a="{A_NS}" xmlns:pic="{PIC_NS}">"#
    ));
    out.push_str("<w:body>");

    let section = doc
        .sections
        .first()
        .ok_or_else(|| WmlError::Structure("document has no sections".into()))?;

    for block in &section.blocks {
        match block {
            Block::Paragraph(p) => write_paragraph(&mut out, p),
            Block::Table(t) => write_table(&mut out, t),
            Block::Unsupported(u) => out.push_str(&u.raw_xml),
        }
    }

    let page = &section.page;
    out.push_str("<w:sectPr>");
    if let Some(rid) = &section.header_r_id {
        out.push_str(&format!(
            r#"<w:headerReference w:type="default" r:id="{rid}"/>"#
        ));
    }
    if let Some(rid) = &section.footer_r_id {
        out.push_str(&format!(
            r#"<w:footerReference w:type="default" r:id="{rid}"/>"#
        ));
    }
    out.push_str(&format!(
        r#"<w:pgSz w:w="{}" w:h="{}"/>"#,
        page.width_twips, page.height_twips
    ));
    out.push_str(&format!(
        r#"<w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}"/>"#,
        page.margin_top_twips,
        page.margin_right_twips,
        page.margin_bottom_twips,
        page.margin_left_twips
    ));
    out.push_str("</w:sectPr>");
    out.push_str("</w:body></w:document>");
    Ok(out)
}

fn write_table(out: &mut String, table: &Table) {
    if !table.edited {
        if let Some(raw) = &table.source_xml {
            out.push_str(raw);
            return;
        }
    }
    out.push_str("<w:tbl>");
    out.push_str(
        r#"<w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>
        <w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>
        <w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>
        <w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>
        <w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>
        <w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>
        <w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/>
        </w:tblBorders></w:tblPr>"#,
    );
    let cols = table
        .rows
        .iter()
        .map(|r| r.cells.len())
        .max()
        .unwrap_or(1)
        .max(1);
    out.push_str("<w:tblGrid>");
    for _ in 0..cols {
        out.push_str(r#"<w:gridCol w:w="2400"/>"#);
    }
    out.push_str("</w:tblGrid>");
    for row in &table.rows {
        out.push_str("<w:tr>");
        for cell in &row.cells {
            let span = cell.grid_span.max(1);
            out.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{}" w:type="dxa"/>"#,
                2400 * span
            ));
            if span > 1 {
                out.push_str(&format!(r#"<w:gridSpan w:val="{span}"/>"#));
            }
            out.push_str("</w:tcPr>");
            if cell.paragraphs.is_empty() {
                write_paragraph(out, &Paragraph::empty());
            } else {
                for p in &cell.paragraphs {
                    write_paragraph(out, p);
                }
            }
            out.push_str("</w:tc>");
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
}

fn write_paragraph(out: &mut String, p: &Paragraph) {
    out.push_str("<w:p>");
    write_p_pr(out, p);
    for run in &p.runs {
        write_run(out, run);
    }
    for unk in &p.unknown_children {
        // Strip comment wrappers if present; emit raw XML fragment.
        if let Some(rest) = unk.strip_prefix("<!--unknown:") {
            if let Some(idx) = rest.find("-->") {
                out.push_str(&rest[idx + 3..]);
                continue;
            }
        }
        out.push_str(unk);
    }
    out.push_str("</w:p>");
}

fn write_p_pr(out: &mut String, p: &Paragraph) {
    let props = &p.properties;
    let has = props.bidirectional
        || props.alignment.is_some()
        || props.list_kind.is_some()
        || p.style_id.is_some()
        || !p.unknown_p_pr.is_empty();
    if !has {
        return;
    }
    out.push_str("<w:pPr>");
    if let Some(style) = &p.style_id {
        out.push_str(&format!(r#"<w:pStyle w:val="{style}"/>"#));
    }
    if props.bidirectional {
        out.push_str("<w:bidi/>");
    }
    if let Some(align) = props.alignment {
        let val = match align {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "both",
            Alignment::Start => "start",
            Alignment::End => "end",
        };
        out.push_str(&format!(r#"<w:jc w:val="{val}"/>"#));
    }
    if let Some(kind) = props.list_kind {
        let num_id = match kind {
            ListKind::Bullet => 1,
            ListKind::Numbered => 2,
        };
        out.push_str(&format!(
            r#"<w:numPr><w:ilvl w:val="{}"/><w:numId w:val="{num_id}"/></w:numPr>"#,
            props.list_level
        ));
    }
    for unk in &p.unknown_p_pr {
        if let Some(rest) = unk.strip_prefix("<!--pPr:") {
            if let Some(idx) = rest.find("-->") {
                out.push_str(&rest[idx + 3..]);
                continue;
            }
        }
        out.push_str(unk);
    }
    out.push_str("</w:pPr>");
}

fn write_run(out: &mut String, run: &Run) {
    out.push_str("<w:r>");
    write_r_pr(out, &run.properties, &run.unknown_r_pr);
    match &run.content {
        RunContent::Text(t) => write_t(out, t),
        RunContent::Break => out.push_str("<w:br/>"),
        RunContent::Tab => out.push_str("<w:tab/>"),
        RunContent::Image(img) => write_image_drawing(out, img),
        RunContent::Unsupported(raw) => out.push_str(raw),
    }
    out.push_str("</w:r>");
}

fn write_image_drawing(out: &mut String, img: &document_model::InlineImage) {
    let rid = img
        .relationship_id
        .clone()
        .unwrap_or_else(|| "rIdImage".into());
    let cx = (img.width_px.unwrap_or(320) as u64) * 9525; // px ≈ EMUs at 96dpi
    let cy = (img.height_px.unwrap_or(240) as u64) * 9525;
    out.push_str(&format!(
        r#"<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">
<wp:extent cx="{cx}" cy="{cy}"/>
<wp:docPr id="1" name="Picture"/>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
<pic:pic>
<pic:nvPicPr><pic:cNvPr id="0" name="Picture"/><pic:cNvPicPr/></pic:nvPicPr>
<pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>
<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>
<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>
</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"#
    ));
}

fn write_r_pr(out: &mut String, props: &RunProperties, unknown: &[String]) {
    let has = props.bold
        || props.italic
        || props.underline
        || props.strike
        || props.rtl
        || props.font_ascii.is_some()
        || props.font_h_ansi.is_some()
        || props.font_cs.is_some()
        || props.font_east_asia.is_some()
        || props.font_size_half_points.is_some()
        || props.language.is_some()
        || props.color_hex.is_some()
        || !unknown.is_empty();
    if !has {
        return;
    }
    out.push_str("<w:rPr>");
    if props.bold {
        out.push_str("<w:b/><w:bCs/>");
    }
    if props.italic {
        out.push_str("<w:i/><w:iCs/>");
    }
    if props.underline {
        out.push_str(r#"<w:u w:val="single"/>"#);
    }
    if props.strike {
        out.push_str("<w:strike/>");
    }
    if props.font_ascii.is_some()
        || props.font_h_ansi.is_some()
        || props.font_cs.is_some()
        || props.font_east_asia.is_some()
    {
        out.push_str("<w:rFonts");
        if let Some(v) = &props.font_ascii {
            out.push_str(&format!(r#" w:ascii="{v}""#));
        }
        if let Some(v) = &props.font_h_ansi {
            out.push_str(&format!(r#" w:hAnsi="{v}""#));
        }
        if let Some(v) = &props.font_cs {
            out.push_str(&format!(r#" w:cs="{v}""#));
        }
        if let Some(v) = &props.font_east_asia {
            out.push_str(&format!(r#" w:eastAsia="{v}""#));
        }
        out.push_str("/>");
    }
    if let Some(sz) = props.font_size_half_points {
        out.push_str(&format!(r#"<w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/>"#));
    }
    if let Some(c) = &props.color_hex {
        out.push_str(&format!(r#"<w:color w:val="{c}"/>"#));
    }
    if props.rtl {
        out.push_str("<w:rtl/>");
    }
    if let Some(lang) = &props.language {
        out.push_str(&format!(r#"<w:lang w:val="{lang}"/>"#));
    }
    for unk in unknown {
        if let Some(rest) = unk.strip_prefix("<!--rPr:") {
            if let Some(idx) = rest.find("-->") {
                out.push_str(&rest[idx + 3..]);
                continue;
            }
        }
        out.push_str(unk);
    }
    out.push_str("</w:rPr>");
}

fn write_t(out: &mut String, text: &str) {
    let needs_space = text.starts_with(' ') || text.ends_with(' ') || text.contains('\t');
    if needs_space {
        out.push_str(r#"<w:t xml:space="preserve">"#);
    } else {
        out.push_str("<w:t>");
    }
    out.push_str(&escape_xml(text));
    out.push_str("</w:t>");
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
