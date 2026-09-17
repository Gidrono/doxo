# word-rs

Rust-native macOS word processor with Hebrew/RTL as a first-class scenario.

**TextKit / AppKit** owns editing, caret, bidi, and shaping. **Rust** owns the document model, OOXML package engine, commands, and persistence.

> This workspace currently lives under the `doxo` folder/path. Rename when you are ready — tooling will not rename remotes.

## Status

| Area | State |
|------|--------|
| Paginated **Print Layout** (US Letter pages, gaps, multi-container flow) | Done |
| Editable header/footer + OOXML `word/header*.xml` / `footer*.xml` + `sectPr` refs | Done |
| Home toolbar (B/I/U, size, align, RTL/LTR, styles, lists, table/image) | Done |
| Selection-accurate bold/italic/underline/size/style | Done |
| Style presets (Normal, Title, Heading 1–3) | Done |
| Bullet / numbered list prefixes | Done |
| **NSTextTable** insert/edit + OOXML `w:tbl` regenerate when edited | Done |
| **Images**: `word/media/` + relationships + `w:drawing`/`a:blip` + NSTextAttachment | Done |
| Per-run/paragraph unknown OOXML anchors (`unknown_p_pr` / `unknown_r_pr` / children) | Done |
| DOCX open/save with non-document part preservation | Done |
| Legacy `.doc` (LibreOffice) | Stub (see `compat-doc`) |
| Signing / notarization / app bundle | Not started |

## Requirements

- macOS + Xcode Command Line Tools
- Rust stable (`rustup`)

## Build & run

```bash
source "$HOME/.cargo/env"   # if needed
cargo build -p app
cargo run -p app
```

Release:

```bash
cargo build -p app --release
./target/release/word-rs
```

On launch the app opens `fixtures/hebrew/mixed-hebrew-english.docx` when present.

**Try Print Layout:** click in the page body (not the header/footer), type, then press **Return** for a new paragraph (Shift-Return soft break). Type enough text to flow onto a second page; pages appear as separate white sheets with gaps. Click the header or footer text to edit; page numbers stay as a separate label. **Save** writes real header/footer package parts.

**Try formatting:** select text → **B** / **I** / **U**, Size, Style popup, **L/C/R**, **RTL** / **LTR**, **• List** / **1. List**, **Table**, **Image**, then **Save**.

## Test

```bash
cargo test --workspace
```

Enter / Print Layout regression (AppKit, main thread):

```bash
cargo run -p mac-ui --example enter_diag
# expect: after_insertNewline contains "\n" and block_count >= 2
```

## Crate layout

```
crates/
  app/                 # word-rs binary
  mac-ui/              # AppKit UI, Print Layout, NSTextTable, attachments
  layout-bridge/       # model ↔ bridged blocks (paragraphs/tables/images) + page metrics
  document-model/      # sections, paragraphs, runs, tables, images, unknown anchors
  ooxml-package/       # ZIP/OPC part-preserving package + media/header/footer/rels helpers
  wordprocessingml/    # document.xml + header/footer part XML
  persistence/         # open/save, image + header/footer hydrate/sync
  editor-core/         # command/undo stubs
  compat-doc/          # LibreOffice .doc stub (documented, not wired)
  test-fixtures/
fixtures/hebrew/ … roundtrip/
```

## OOXML subset

- Paragraphs/runs: bold, italic, underline, fonts, size, RTL/bidi, alignment, styles, list markers
- Tables: structured model; `source_xml` preserved when `edited == false`; regenerated `w:tbl` after structural/UI edits; multi-paragraph cells joined for display
- Images: embedded under `word/media/`, document relationships, inline drawing/blip on serialize; hydrated from package on open
- Headers/footers: `word/header1.xml` / `word/footer1.xml`, relationships, content types, `w:headerReference` / `w:footerReference` in `sectPr`; original part XML preserved when plain text is unchanged
- Unknown `pPr` / `rPr` / paragraph children retained as opaque fragments and re-emitted on save; UI save merges prior anchors onto matching paragraphs
- Package: `document.xml` rewritten; other parts preserved; new media/header/footer/rels/content-types added as needed

## Remaining stubs / next steps

1. LibreOffice `.doc` bridge (`compat-doc`) once conversion is desired
2. App bundle, signing, notarization
3. First-class multi-paragraph table cell editing (beyond joined plain text)
4. Odd/even/first-page header/footer variants

## License

MIT — see `LICENSE`.
