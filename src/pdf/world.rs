//! A minimal, sandboxed Typst `World`: one main source, one JSON data file,
//! and an explicit font list. Templates cannot read anything else from disk.

use std::sync::OnceLock;

use chrono::Datelike;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World, WorldExt};
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

/// Fonts bundled with the binary (Libertinus Serif, New Computer Modern,
/// DejaVu Sans Mono). Parsed once per process.
pub fn embedded_fonts() -> Vec<Font> {
    static FONTS: OnceLock<Vec<Font>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            typst_assets::fonts()
                .flat_map(|data| Font::iter(Bytes::new(data)))
                .collect()
        })
        .clone()
}

pub fn fonts_from(data: Vec<u8>) -> Vec<Font> {
    Font::iter(Bytes::new(data)).collect()
}

fn project_file(path: &str) -> FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new(path).expect("valid virtual path"),
    )
    .intern()
}

struct TallyWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
    data_id: FileId,
    data: Bytes,
    today: chrono::NaiveDate,
}

impl World for TallyWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.data_id {
            Ok(self.data.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        Datetime::from_ymd(
            self.today.year(),
            self.today.month() as u8,
            self.today.day() as u8,
        )
    }
}

/// Compiles `template` (which reads `/data.json`) to PDF bytes.
pub fn compile_pdf(template: &str, data_json: Vec<u8>, fonts: &[Font]) -> Result<Vec<u8>, String> {
    let world = TallyWorld {
        library: LazyHash::new(Library::builder().build()),
        book: LazyHash::new(FontBook::from_fonts(fonts)),
        fonts: fonts.to_vec(),
        main: Source::new(project_file("/main.typ"), template.to_string()),
        data_id: project_file("/data.json"),
        data: Bytes::new(data_json),
        today: crate::domain::dates::today(),
    };
    let document: PagedDocument = typst::compile(&world)
        .output
        .map_err(|errors| describe(&world, &errors))?;
    typst_pdf::pdf(&document, &PdfOptions::default()).map_err(|errors| describe(&world, &errors))
}

fn describe(world: &TallyWorld, errors: &[SourceDiagnostic]) -> String {
    errors
        .iter()
        .map(|e| {
            let line = world
                .range(e.span)
                .and_then(|r| world.main.lines().byte_to_line(r.start))
                .map(|l| format!("line {}: ", l + 1))
                .unwrap_or_default();
            format!("template error: {line}{}", e.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
}
