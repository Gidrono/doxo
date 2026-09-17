//! Semantic editing session and command stubs.

use document_model::{
    Alignment, Block, Document, Paragraph, ParagraphProperties, Run, RunContent, RunProperties,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("{0}")]
    Message(String),
}

/// High-level formatting intents applied to the selection / document.
#[derive(Debug, Clone)]
pub enum FormatCommand {
    ToggleBold,
    ToggleItalic,
    ToggleUnderline,
    SetFontSize(f64),
    SetFontFamily(String),
    SetAlignment(Alignment),
    SetParagraphRtl(bool),
}

/// Editing session holding the semantic document and a simple undo stack.
#[derive(Debug)]
pub struct EditorSession {
    pub document: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    pub dirty: bool,
}

impl EditorSession {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
        }
    }

    pub fn blank() -> Self {
        Self::new(Document::blank())
    }

    fn push_undo(&mut self) {
        self.undo.push(self.document.clone());
        self.redo.clear();
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.dirty = true;
    }

    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.document.clone());
            self.document = prev;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.document.clone());
            self.document = next;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Replace document contents from plain paragraphs (used when syncing from TextKit).
    pub fn replace_from_plain_paragraphs(&mut self, paragraphs: Vec<(String, ParagraphProperties, Vec<(String, RunProperties)>)>) {
        self.push_undo();
        let mut blocks = Vec::new();
        for (plain, pprops, runs) in paragraphs {
            let mut p = Paragraph::empty();
            p.properties = pprops;
            if runs.is_empty() {
                p.runs = vec![Run::text(plain)];
            } else {
                p.runs = runs
                    .into_iter()
                    .map(|(text, props)| Run {
                        id: document_model::NodeId::new(),
                        properties: props,
                        content: RunContent::Text(text),
                        source_xml: None,
                        unknown_r_pr: Vec::new(),
                    })
                    .collect();
            }
            blocks.push(Block::Paragraph(p));
        }
        if blocks.is_empty() {
            blocks.push(Block::Paragraph(Paragraph::empty()));
        }
        if let Some(section) = self.document.sections.first_mut() {
            section.blocks = blocks;
        }
    }

    pub fn apply_to_all_runs(&mut self, f: impl Fn(&mut RunProperties)) {
        self.push_undo();
        for section in &mut self.document.sections {
            for block in &mut section.blocks {
                if let Block::Paragraph(p) = block {
                    for run in &mut p.runs {
                        f(&mut run.properties);
                    }
                }
            }
        }
    }

    pub fn apply_to_all_paragraphs(&mut self, f: impl Fn(&mut ParagraphProperties)) {
        self.push_undo();
        for section in &mut self.document.sections {
            for block in &mut section.blocks {
                if let Block::Paragraph(p) = block {
                    f(&mut p.properties);
                }
            }
        }
    }

    pub fn toggle_bold_all(&mut self) {
        let any_not_bold = self.document.paragraphs().any(|p| {
            p.runs.iter().any(|r| !r.properties.bold)
        });
        self.apply_to_all_runs(|rp| rp.bold = any_not_bold);
    }

    pub fn toggle_italic_all(&mut self) {
        let any_not = self.document.paragraphs().any(|p| {
            p.runs.iter().any(|r| !r.properties.italic)
        });
        self.apply_to_all_runs(|rp| rp.italic = any_not);
    }

    pub fn set_alignment_all(&mut self, alignment: Alignment) {
        self.apply_to_all_paragraphs(|pp| pp.alignment = Some(alignment));
    }

    pub fn set_rtl_all(&mut self, rtl: bool) {
        self.apply_to_all_paragraphs(|pp| pp.bidirectional = rtl);
        self.apply_to_all_runs(|rp| rp.rtl = rtl);
        // apply_to_all_runs already pushed undo; avoid double — so do manually:
    }
}

/// Apply a format command globally (selection-aware formatting lives in mac-ui for now).
pub fn apply_format(session: &mut EditorSession, cmd: FormatCommand) {
    match cmd {
        FormatCommand::ToggleBold => session.toggle_bold_all(),
        FormatCommand::ToggleItalic => session.toggle_italic_all(),
        FormatCommand::ToggleUnderline => {
            let any_not = session.document.paragraphs().any(|p| {
                p.runs.iter().any(|r| !r.properties.underline)
            });
            session.apply_to_all_runs(|rp| rp.underline = any_not);
        }
        FormatCommand::SetFontSize(pt) => {
            let half = (pt * 2.0).round() as u32;
            session.apply_to_all_runs(|rp| rp.font_size_half_points = Some(half));
        }
        FormatCommand::SetFontFamily(name) => {
            session.apply_to_all_runs(|rp| {
                rp.font_ascii = Some(name.clone());
                rp.font_h_ansi = Some(name.clone());
                rp.font_cs = Some(name.clone());
            });
        }
        FormatCommand::SetAlignment(a) => session.set_alignment_all(a),
        FormatCommand::SetParagraphRtl(rtl) => {
            session.push_undo();
            for section in &mut session.document.sections {
                for block in &mut section.blocks {
                    if let Block::Paragraph(p) = block {
                        p.properties.bidirectional = rtl;
                        for run in &mut p.runs {
                            run.properties.rtl = rtl;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_text() {
        let mut s = EditorSession::blank();
        s.document.sections[0].blocks = vec![Block::Paragraph(Paragraph::from_text("hi"))];
        s.push_undo();
        s.document.sections[0].blocks = vec![Block::Paragraph(Paragraph::from_text("bye"))];
        assert!(s.undo());
        assert_eq!(s.document.plain_text(), "hi");
        assert!(s.redo());
        assert_eq!(s.document.plain_text(), "bye");
    }
}
