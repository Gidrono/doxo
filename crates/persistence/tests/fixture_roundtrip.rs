//! Integration: open fixture DOCX, round-trip through persistence.

use std::path::PathBuf;

use persistence::DocumentSession;

fn hebrew_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hebrew/mixed-hebrew-english.docx")
}

#[test]
fn open_hebrew_fixture() {
    let path = hebrew_fixture();
    assert!(path.exists(), "missing fixture at {}", path.display());
    let session = DocumentSession::open_path(&path).expect("open fixture");
    let text = session.document.plain_text();
    assert!(text.contains("שלום"), "text was: {text}");
    assert!(text.contains("English"));
}

#[test]
fn round_trip_hebrew_fixture_to_temp() {
    let path = hebrew_fixture();
    let mut session = DocumentSession::open_path(&path).unwrap();
    let original = session.document.plain_text();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.docx");
    session.save_to_path(&out).unwrap();
    let again = DocumentSession::open_path(&out).unwrap();
    assert_eq!(again.document.plain_text(), original);
    assert!(again.package.get_part("word/styles.xml").is_some());
}

#[test]
fn round_trip_after_enter_edit_simulation() {
    let path = hebrew_fixture();
    let mut session = DocumentSession::open_path(&path).unwrap();
    use document_model::{Block, Paragraph};
    // Simulate Return: split / append paragraphs like TextKit → attributed_to_blocks.
    session.document.sections[0].blocks.push(Block::Paragraph(
        Paragraph::from_text("New paragraph after Enter"),
    ));
    session.document.sections[0]
        .blocks
        .push(Block::Paragraph(Paragraph::from_text("שורה חדשה")));
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("edited.docx");
    session.save_to_path(&out).unwrap();
    let again = DocumentSession::open_path(&out).unwrap();
    let text = again.document.plain_text();
    assert!(text.contains("New paragraph after Enter"), "{text}");
    assert!(text.contains("שורה חדשה"), "{text}");
    assert!(again.preserved_parts_intact());
}

#[test]
fn round_trip_simple_formatting_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/roundtrip/simple-formatting.docx");
    if !path.exists() {
        return;
    }
    let mut session = DocumentSession::open_path(&path).unwrap();
    let original = session.document.plain_text();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("fmt.docx");
    session.save_to_path(&out).unwrap();
    let again = DocumentSession::open_path(&out).unwrap();
    assert_eq!(again.document.plain_text(), original);
}
