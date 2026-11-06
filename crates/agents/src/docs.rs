//! Office documents without an office suite: read the text of PDF, Word, PowerPoint, Excel and
//! OpenDocument files, create .docx (from Markdown), .xlsx and .pptx files, and replace text inside
//! them. Rich editing (styles, layout) is the Craft apps' job; this covers what an assistant needs to
//! read a document it was given and to hand back a clean new one.

use std::io::{Read, Write};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event as X;
use serde_json::Value;
use workcraft_core::{Error, Result};

/// Lower-case extension of a path.
#[must_use]
pub fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Extensions whose text this module can pull out.
pub const DOCUMENTS: [&str; 15] = [
    "pdf", "docx", "docm", "dotx", "pptx", "pptm", "xlsx", "xlsm", "xls", "xlsb", "ods", "odt",
    "odp", "csv", "tsv",
];

#[must_use]
pub fn is_document(path: &Path) -> bool {
    DOCUMENTS.contains(&ext(path).as_str())
}

/// What kind of file this is, for messages and the UI.
#[must_use]
pub fn kind(path: &Path) -> &'static str {
    match ext(path).as_str() {
        "pdf" => "PDF",
        "docx" | "docm" | "dotx" | "doc" | "odt" | "rtf" => "document",
        "pptx" | "pptm" | "ppt" | "odp" => "presentation",
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" | "csv" | "tsv" => "spreadsheet",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => "image",
        _ => "file",
    }
}

fn zip_open(path: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    zip::ZipArchive::new(f).map_err(|e| {
        Error::Invalid(format!(
            "{} is not a valid Office file: {e}",
            path.display()
        ))
    })
}

fn zip_text(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Text of an XML part: `para` tags end a line, `cell` tags end a table cell, text comes from `text`
/// tags (or from every text node when `text` is empty).
fn xml_text(xml: &str, para: &[&str], cell: &[&str], text: &[&str]) -> String {
    let mut r = Reader::from_str(xml);
    let mut out = String::new();
    let mut inside = 0usize;
    // Inside a table cell, paragraphs are joined with spaces so a row stays on one line.
    let mut in_cell = 0usize;
    loop {
        match r.read_event() {
            Ok(X::Start(e)) => {
                let n = local(e.name().as_ref());
                if text.contains(&n) {
                    inside += 1;
                }
                if cell.contains(&n) {
                    in_cell += 1;
                }
            }
            Ok(X::End(e)) => {
                let n = local(e.name().as_ref());
                if text.contains(&n) {
                    inside = inside.saturating_sub(1);
                }
                if cell.contains(&n) {
                    in_cell = in_cell.saturating_sub(1);
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    out.push('\t');
                } else if para.contains(&n) {
                    if in_cell > 0 {
                        out.push(' ');
                    } else {
                        while out.ends_with('\t') {
                            out.pop();
                        }
                        out.push('\n');
                    }
                }
            }
            Ok(X::Empty(e)) => match local(e.name().as_ref()) {
                "tab" => out.push('\t'),
                "br" | "cr" | "line-break" => out.push('\n'),
                "s" => out.push(' '),
                _ => {}
            },
            Ok(X::Text(t)) => {
                if (text.is_empty() || inside > 0)
                    && let Ok(s) = t.unescape()
                {
                    out.push_str(&s);
                }
            }
            Ok(X::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn local(name: &[u8]) -> &'static str {
    // Map the few local names we care about to static strings; anything else is "".
    let n = match name.iter().rposition(|b| *b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    };
    match n {
        b"p" => "p",
        b"t" => "t",
        b"tab" => "tab",
        b"br" => "br",
        b"cr" => "cr",
        b"tc" => "tc",
        b"tr" => "tr",
        b"h" => "h",
        b"span" => "span",
        b"line-break" => "line-break",
        b"s" => "s",
        b"table-cell" => "table-cell",
        _ => "",
    }
}

/// Number in `ppt/slides/slide12.xml`, for ordering.
fn part_number(name: &str) -> u32 {
    name.chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn cell_text(d: &calamine::Data) -> String {
    use calamine::Data;
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(d) => d.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

/// The text of a document. Spreadsheets come out as one tab-separated table per sheet.
pub fn extract_text(path: &Path) -> Result<String> {
    let e = ext(path);
    match e.as_str() {
        "pdf" => {
            let p = path.to_path_buf();
            // The PDF parser can panic on broken files; keep that inside this call.
            let r = std::panic::catch_unwind(move || pdf_extract::extract_text(&p));
            match r {
                Ok(Ok(t)) if !t.trim().is_empty() => Ok(t),
                Ok(Ok(_)) => Err(Error::Invalid(
                    "this PDF has no text layer (a scan?). Use PdfCraft to read it.".into(),
                )),
                Ok(Err(err)) => Err(Error::Invalid(format!("could not read the PDF: {err}"))),
                Err(_) => Err(Error::Invalid(
                    "could not read the PDF (damaged file)".into(),
                )),
            }
        }
        "docx" | "docm" | "dotx" => {
            let mut z = zip_open(path)?;
            let mut out = String::new();
            for part in ["word/document.xml", "word/footnotes.xml"] {
                if let Some(x) = zip_text(&mut z, part) {
                    out.push_str(&xml_text(&x, &["p", "tr"], &["tc"], &["t"]));
                }
            }
            Ok(out)
        }
        "pptx" | "pptm" => {
            let mut z = zip_open(path)?;
            let mut slides: Vec<String> = (0..z.len())
                .filter_map(|i| {
                    z.by_index(i)
                        .ok()
                        .and_then(|f| f.name().ok().map(|n| n.to_string()))
                })
                .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
                .collect();
            slides.sort_by_key(|n| part_number(n));
            let mut out = String::new();
            for (i, s) in slides.iter().enumerate() {
                if let Some(x) = zip_text(&mut z, s) {
                    out.push_str(&format!("## Slide {}\n", i + 1));
                    out.push_str(xml_text(&x, &["p"], &[], &["t"]).trim());
                    out.push_str("\n\n");
                }
            }
            Ok(out)
        }
        "odt" | "odp" => {
            let mut z = zip_open(path)?;
            let x = zip_text(&mut z, "content.xml")
                .ok_or_else(|| Error::Invalid("no content.xml in this file".into()))?;
            Ok(xml_text(&x, &["p", "h"], &["table-cell"], &[]))
        }
        "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" => {
            use calamine::Reader as _;
            let mut wb = calamine::open_workbook_auto(path)
                .map_err(|e| Error::Invalid(format!("could not read the workbook: {e}")))?;
            let mut out = String::new();
            for name in wb.sheet_names().clone() {
                let Ok(range) = wb.worksheet_range(&name) else {
                    continue;
                };
                out.push_str(&format!("## Sheet {name}\n"));
                for row in range.rows().take(5000) {
                    let cells: Vec<String> = row.iter().map(cell_text).collect();
                    out.push_str(cells.join("\t").trim_end());
                    out.push('\n');
                }
                out.push('\n');
            }
            Ok(out)
        }
        _ => {
            let bytes =
                std::fs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
            if bytes.iter().take(8192).any(|b| *b == 0) {
                return Err(Error::Invalid(format!(
                    "{} is a binary file ({}) this tool cannot read",
                    path.display(),
                    if e.is_empty() {
                        "no extension".to_string()
                    } else {
                        format!(".{e}")
                    }
                )));
            }
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Ok(if matches!(e.as_str(), "html" | "htm") {
                strip_tags(&text)
            } else {
                text
            })
        }
    }
}

#[must_use]
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let lower = html.to_lowercase();
    let mut skip_until: Option<&str> = None;
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if let Some(end) = skip_until {
            if lower[i..].starts_with(end) {
                skip_until = None;
                i += end.len();
            } else {
                i += 1;
            }
            continue;
        }
        if lower[i..].starts_with("<script") {
            skip_until = Some("</script>");
            continue;
        }
        if lower[i..].starts_with("<style") {
            skip_until = Some("</style>");
            continue;
        }
        let c = b[i];
        if c == b'<' {
            in_tag = true;
            if lower[i..].starts_with("<br")
                || lower[i..].starts_with("</p")
                || lower[i..].starts_with("</div")
                || lower[i..].starts_with("</h")
                || lower[i..].starts_with("</li")
            {
                out.push('\n');
            }
        } else if c == b'>' {
            in_tag = false;
        } else if !in_tag {
            // Copy whole UTF-8 characters.
            let ch_len = html[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&html[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        i += 1;
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

// ---------------------------------------------------------------------------------------- writing

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn write_zip(path: &Path, parts: &[(&str, String)]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let f =
        std::fs::File::create(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in parts {
        z.start_file(*name, opt)
            .map_err(|e| Error::Io(e.to_string()))?;
        z.write_all(body.as_bytes())?;
    }
    z.finish().map_err(|e| Error::Io(e.to_string()))?;
    Ok(())
}

/// Runs for one line of Markdown: **bold**, *italic*, `code`.
fn md_runs(s: &str) -> String {
    let mut out = String::new();
    let (mut bold, mut ital, mut code) = (false, false, false);
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    let flush = |cur: &mut String, out: &mut String, bold: bool, ital: bool, code: bool| {
        if cur.is_empty() {
            return;
        }
        let mut rpr = String::new();
        if bold {
            rpr.push_str("<w:b/>");
        }
        if ital {
            rpr.push_str("<w:i/>");
        }
        if code {
            rpr.push_str(r#"<w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:shd w:val="clear" w:fill="F2F2F2"/>"#);
        }
        out.push_str(&format!(
            r#"<w:r>{}<w:t xml:space="preserve">{}</w:t></w:r>"#,
            if rpr.is_empty() {
                String::new()
            } else {
                format!("<w:rPr>{rpr}</w:rPr>")
            },
            esc(cur)
        ));
        cur.clear();
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            flush(&mut cur, &mut out, bold, ital, code);
            code = !code;
        } else if !code && c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut cur, &mut out, bold, ital, code);
            bold = !bold;
            i += 1;
        } else if !code
            && c == '*'
            && (ital || chars.get(i + 1).is_some_and(|n| !n.is_whitespace()))
        {
            flush(&mut cur, &mut out, bold, ital, code);
            ital = !ital;
        } else {
            cur.push(c);
        }
        i += 1;
    }
    flush(&mut cur, &mut out, bold, ital, code);
    out
}

fn para(style: Option<&str>, runs: &str) -> String {
    match style {
        Some(s) => format!(r#"<w:p><w:pPr><w:pStyle w:val="{s}"/></w:pPr>{runs}</w:p>"#),
        None => format!("<w:p>{runs}</w:p>"),
    }
}

fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let w = 9000 / cols;
    let mut x = String::from(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid>"#,
    );
    for _ in 0..cols {
        x.push_str(&format!(r#"<w:gridCol w:w="{w}"/>"#));
    }
    x.push_str("</w:tblGrid>");
    for (ri, r) in rows.iter().enumerate() {
        x.push_str("<w:tr>");
        for ci in 0..cols {
            let t = r.get(ci).map_or("", String::as_str);
            let runs = if ri == 0 {
                md_runs(&format!("**{t}**"))
            } else {
                md_runs(t)
            };
            x.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{w}" w:type="dxa"/></w:tcPr><w:p>{runs}</w:p></w:tc>"#
            ));
        }
        x.push_str("</w:tr>");
    }
    x.push_str("</w:tbl><w:p/>");
    x
}

fn table_row(line: &str) -> Option<Vec<String>> {
    let t = line.trim();
    if !(t.starts_with('|') && t.ends_with('|') && t.len() > 1) {
        return None;
    }
    Some(
        t[1..t.len() - 1]
            .split('|')
            .map(|c| c.trim().to_string())
            .collect(),
    )
}

/// The body of a Word document for Markdown text.
fn docx_body(title: Option<&str>, md: &str) -> String {
    let mut body = String::new();
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        body.push_str(&para(Some("Title"), &md_runs(t)));
    }
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut in_code = false;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if in_code {
            body.push_str(&para(
                Some("Code"),
                &format!(
                    r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
                    esc(line)
                ),
            ));
        } else if let Some(row) = table_row(t) {
            let mut rows = vec![row];
            i += 1;
            while i < lines.len() {
                let l = lines[i].trim();
                match table_row(l) {
                    Some(r)
                        if r.iter()
                            .all(|c| c.chars().all(|ch| matches!(ch, '-' | ':' | ' '))) => {}
                    Some(r) => rows.push(r),
                    None => break,
                }
                i += 1;
            }
            body.push_str(&table(&rows));
            continue;
        } else if t.is_empty() {
            // Paragraph spacing comes from the styles.
        } else if let Some(h) = t.strip_prefix("### ") {
            body.push_str(&para(Some("Heading3"), &md_runs(h)));
        } else if let Some(h) = t.strip_prefix("## ") {
            body.push_str(&para(Some("Heading2"), &md_runs(h)));
        } else if let Some(h) = t.strip_prefix("# ") {
            body.push_str(&para(Some("Heading1"), &md_runs(h)));
        } else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            body.push_str(&para(
                Some("ListBullet"),
                &format!(
                    "<w:r><w:t xml:space=\"preserve\">\u{2022}\t</w:t></w:r>{}",
                    md_runs(b)
                ),
            ));
        } else if let Some((n, rest)) = t
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        {
            body.push_str(&para(
                Some("ListBullet"),
                &format!(
                    "<w:r><w:t xml:space=\"preserve\">{n}.\t</w:t></w:r>{}",
                    md_runs(rest)
                ),
            ));
        } else if let Some(q) = t.strip_prefix("> ") {
            body.push_str(&para(Some("Quote"), &md_runs(q)));
        } else if t.chars().all(|c| c == '-') && t.len() >= 3 {
            body.push_str(r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="BFBFBF"/></w:pBdr></w:pPr></w:p>"#);
        } else {
            body.push_str(&para(None, &md_runs(t)));
        }
        i += 1;
    }
    body
}

const DOCX_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="240"/></w:pPr><w:rPr><w:sz w:val="52"/><w:szCs w:val="52"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="360" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:color w:val="1F3864"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="80"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:color w:val="2F5496"/><w:sz w:val="26"/><w:szCs w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="200" w:after="60"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListBullet"><w:name w:val="List Bullet"/><w:basedOn w:val="Normal"/><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs><w:spacing w:after="60"/><w:ind w:left="720" w:hanging="360"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/></w:pPr><w:rPr><w:i/><w:color w:val="595959"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="0"/><w:shd w:val="clear" w:fill="F2F2F2"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas"/><w:sz w:val="20"/></w:rPr></w:style>
<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:left w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:right w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="BFBFBF"/></w:tblBorders><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>
</w:styles>"#;

fn core_props(title: &str) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>WorkCraft</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">{now}</dcterms:modified></cp:coreProperties>"#,
        esc(title)
    )
}

/// A Word document from Markdown: headings, paragraphs, lists, quotes, code, tables, **bold**, *italic*.
pub fn write_docx(path: &Path, title: Option<&str>, markdown: &str) -> Result<()> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" w:bottom="1134" w:left="1417" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#,
        docx_body(title, markdown)
    );
    write_zip(
        path,
        &[
            ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#.to_string()),
            ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#.to_string()),
            ("word/_rels/document.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_string()),
            ("word/styles.xml", DOCX_STYLES.to_string()),
            ("word/document.xml", doc),
            ("docProps/core.xml", core_props(title.unwrap_or(""))),
        ],
    )
}

/// One sheet for [`write_xlsx`].
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Value>>,
    /// Bold first row and freeze it.
    pub header: bool,
}

/// An Excel workbook. Numbers stay numbers, strings starting with `=` are formulas.
pub fn write_xlsx(path: &Path, sheets: &[Sheet]) -> Result<()> {
    use rust_xlsxwriter::{Format, Workbook};
    let x = |e: rust_xlsxwriter::XlsxError| Error::Invalid(format!("xlsx: {e}"));
    let mut wb = Workbook::new();
    let bold = Format::new().set_bold();
    for (si, s) in sheets.iter().enumerate() {
        let ws = wb.add_worksheet();
        let name = if s.name.trim().is_empty() {
            format!("Sheet{}", si + 1)
        } else {
            s.name.chars().take(31).collect()
        };
        ws.set_name(&name).map_err(x)?;
        for (r, row) in s.rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                let (r, c) = (r as u32, c as u16);
                let head = s.header && r == 0;
                match v {
                    Value::Number(n) => {
                        let f = n.as_f64().unwrap_or(0.0);
                        if head {
                            ws.write_number_with_format(r, c, f, &bold)
                        } else {
                            ws.write_number(r, c, f)
                        }
                        .map_err(x)?;
                    }
                    Value::Bool(b) => {
                        ws.write_boolean(r, c, *b).map_err(x)?;
                    }
                    Value::Null => {}
                    Value::String(t) if t.starts_with('=') && t.len() > 1 => {
                        ws.write_formula(r, c, t.as_str()).map_err(x)?;
                    }
                    other => {
                        let t = match other {
                            Value::String(t) => t.clone(),
                            o => o.to_string(),
                        };
                        // A number written as text ("42", "3.5") becomes a number, so sums work.
                        match t.trim().parse::<f64>() {
                            Ok(f) if !t.starts_with('0') || t == "0" || t.starts_with("0.") => {
                                if head {
                                    ws.write_number_with_format(r, c, f, &bold)
                                } else {
                                    ws.write_number(r, c, f)
                                }
                                .map_err(x)?;
                            }
                            _ => {
                                if head {
                                    ws.write_string_with_format(r, c, &t, &bold)
                                } else {
                                    ws.write_string(r, c, &t)
                                }
                                .map_err(x)?;
                            }
                        };
                    }
                }
            }
        }
        if s.header && !s.rows.is_empty() {
            ws.set_freeze_panes(1, 0).map_err(x)?;
        }
        ws.autofit();
    }
    if sheets.is_empty() {
        wb.add_worksheet();
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    wb.save(path).map_err(x)
}

/// One slide for [`write_pptx`].
pub struct Slide {
    pub title: String,
    pub bullets: Vec<String>,
    pub notes: Option<String>,
}

const SLIDE_W: i64 = 12_192_000;
const SLIDE_H: i64 = 6_858_000;

fn text_box(id: u32, name: &str, x: i64, y: i64, w: i64, h: i64, paras: &str) -> String {
    // The title is a real title placeholder, so slide titles show in outlines and screen readers.
    let (cnv, nvpr) = if name == "Title" {
        (
            r#"<p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr>"#,
            r#"<p:nvPr><p:ph type="title"/></p:nvPr>"#,
        )
    } else {
        (r#"<p:cNvSpPr txBox="1"/>"#, "<p:nvPr/>")
    };
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"/>{cnv}{nvpr}</p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="square" lIns="0" rIns="0" tIns="0" bIns="0"><a:normAutofit/></a:bodyPr><a:lstStyle/>{paras}</p:txBody></p:sp>"#
    )
}

/// A short WorkCraft-blue line under a title (centred on the title slide).
fn accent_bar(x: i64, y: i64, centred: bool) -> String {
    let w = 914_400;
    let x = if centred { (SLIDE_W - w) / 2 } else { x };
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="4" name="Accent"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="45720"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="2C84FC"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>"#
    )
}

fn slide_xml(s: &Slide, first: bool) -> String {
    let m = 640_080; // 0.7 inch margin
    let title_size = if first { 4400 } else { 3200 };
    let align = if first && s.bullets.is_empty() {
        r#"<a:pPr algn="ctr"/>"#
    } else {
        ""
    };
    let title = format!(
        r#"<a:p>{align}<a:r><a:rPr lang="en-US" sz="{title_size}" b="1"><a:solidFill><a:srgbClr val="1F2937"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p>"#,
        esc(&s.title)
    );
    let mut body = String::new();
    for b in &s.bullets {
        body.push_str(&format!(
            r#"<a:p><a:pPr marL="342900" indent="-342900"><a:spcBef><a:spcPts val="900"/></a:spcBef><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/></a:pPr><a:r><a:rPr lang="en-US" sz="2000"><a:solidFill><a:srgbClr val="374151"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p>"#,
            esc(b)
        ));
    }
    if body.is_empty() {
        body.push_str("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
    }
    let (ty, th) = if first && s.bullets.is_empty() {
        (SLIDE_H / 2 - 700_000, 1_400_000)
    } else {
        (457_200, 1_000_000)
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>{}{}{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
        text_box(2, "Title", m, ty, SLIDE_W - 2 * m, th, &title),
        if first && s.bullets.is_empty() {
            accent_bar(m, ty + th + 120_000, true)
        } else {
            // Just under one line of 32 pt title text, well above the bullets.
            accent_bar(m, ty + 640_000, false)
        },
        if s.bullets.is_empty() {
            String::new()
        } else {
            text_box(
                3,
                "Body",
                m,
                1_600_000,
                SLIDE_W - 2 * m,
                SLIDE_H - 2_100_000,
                &body,
            )
        }
    )
}

const PPTX_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="WorkCraft"><a:themeElements><a:clrScheme name="WorkCraft"><a:dk1><a:srgbClr val="111827"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F3F4F6"/></a:lt2><a:accent1><a:srgbClr val="2C84FC"/></a:accent1><a:accent2><a:srgbClr val="10B981"/></a:accent2><a:accent3><a:srgbClr val="F97316"/></a:accent3><a:accent4><a:srgbClr val="8B5CF6"/></a:accent4><a:accent5><a:srgbClr val="EF4444"/></a:accent5><a:accent6><a:srgbClr val="14B8A6"/></a:accent6><a:hlink><a:srgbClr val="2C84FC"/></a:hlink><a:folHlink><a:srgbClr val="6B7280"/></a:folHlink></a:clrScheme><a:fontScheme name="WorkCraft"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="WorkCraft"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="28575"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#;

const PPTX_MASTER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="3200"/></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz="2000"/></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#;

const PPTX_LAYOUT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1"><p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#;

/// A PowerPoint deck: 16:9, one text box for the title and one for the bullets of each slide.
pub fn write_pptx(path: &Path, slides: &[Slide]) -> Result<()> {
    let n = slides.len().max(1);
    let fallback = [Slide {
        title: String::new(),
        bullets: Vec::new(),
        notes: None,
    }];
    let slides: &[Slide] = if slides.is_empty() { &fallback } else { slides };
    let mut ct = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>"#,
    );
    let mut pres_rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#,
    );
    let mut ids = String::new();
    let mut parts: Vec<(String, String)> = Vec::new();
    for (i, s) in slides.iter().enumerate().take(n) {
        let k = i + 1;
        ct.push_str(&format!(r#"<Override PartName="/ppt/slides/slide{k}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#));
        pres_rels.push_str(&format!(r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{k}.xml"/>"#, k + 2));
        ids.push_str(&format!(
            r#"<p:sldId id="{}" r:id="rId{}"/>"#,
            255 + k,
            k + 2
        ));
        parts.push((format!("ppt/slides/slide{k}.xml"), slide_xml(s, i == 0)));
        parts.push((format!("ppt/slides/_rels/slide{k}.xml.rels"), r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#.to_string()));
    }
    ct.push_str("</Types>");
    pres_rels.push_str("</Relationships>");
    let pres = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="{SLIDE_W}" cy="{SLIDE_H}"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
    );
    let title = slides.first().map_or("", |s| s.title.as_str());
    let mut all: Vec<(&str, String)> = vec![
        ("[Content_Types].xml", ct),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#.to_string()),
        ("ppt/presentation.xml", pres),
        ("ppt/_rels/presentation.xml.rels", pres_rels),
        ("ppt/slideMasters/slideMaster1.xml", PPTX_MASTER.to_string()),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#.to_string()),
        ("ppt/slideLayouts/slideLayout1.xml", PPTX_LAYOUT.to_string()),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#.to_string()),
        ("ppt/theme/theme1.xml", PPTX_THEME.to_string()),
        ("docProps/core.xml", core_props(title)),
    ];
    for (n, b) in &parts {
        all.push((n.as_str(), b.clone()));
    }
    write_zip(path, &all)
}

/// Replace `find` with `replace` in the text (never the markup) of an XML string. Returns the count.
fn replace_in_text_nodes(xml: &str, find: &str, replace: &str) -> (String, usize) {
    let (f, r) = (
        esc(find).replace("&quot;", "\""),
        esc(replace).replace("&quot;", "\""),
    );
    let mut out = String::with_capacity(xml.len());
    let mut count = 0;
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let (text, tail) = rest.split_at(lt);
        if !text.is_empty() {
            count += text.matches(&f).count();
            out.push_str(&text.replace(&f, &r));
        }
        match tail.find('>') {
            Some(gt) => {
                out.push_str(&tail[..=gt]);
                rest = &tail[gt + 1..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    count += rest.matches(&f).count();
    out.push_str(&rest.replace(&f, &r));
    (out, count)
}

/// Replace text inside a .docx, .pptx or .xlsx (the text parts only, formatting is kept).
/// Returns how many places changed. Text split across formatting runs is not matched.
pub fn replace_in_office(path: &Path, find: &str, replace: &str) -> Result<usize> {
    if find.is_empty() {
        return Err(Error::Invalid("nothing to find".into()));
    }
    let e = ext(path);
    let wanted = |n: &str| match e.as_str() {
        "docx" | "docm" | "dotx" => {
            n.starts_with("word/") && n.ends_with(".xml") && !n.contains("/_rels/")
        }
        "pptx" | "pptm" => n.starts_with("ppt/slides/slide") && n.ends_with(".xml"),
        "xlsx" | "xlsm" => n == "xl/sharedStrings.xml",
        _ => false,
    };
    let mut src = zip_open(path)?;
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0;
    for i in 0..src.len() {
        let mut f = src.by_index(i).map_err(|e| Error::Io(e.to_string()))?;
        let name = f
            .name()
            .map(|n| n.to_string())
            .map_err(|e| Error::Io(e.to_string()))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        if wanted(&name)
            && let Ok(s) = String::from_utf8(bytes.clone())
        {
            let (new, n) = replace_in_text_nodes(&s, find, replace);
            total += n;
            bytes = new.into_bytes();
        }
        parts.push((name, bytes));
    }
    drop(src);
    if total == 0 {
        return Ok(0);
    }
    let tmp = path.with_extension(format!("{e}.tmp"));
    {
        let f = std::fs::File::create(&tmp)?;
        let mut z = zip::ZipWriter::new(f);
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in &parts {
            z.start_file(n.as_str(), opt)
                .map_err(|e| Error::Io(e.to_string()))?;
            z.write_all(b)?;
        }
        z.finish().map_err(|e| Error::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wc-docs-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    #[test]
    fn docx_round_trips_markdown() {
        let p = tmp("report.docx");
        write_docx(
            &p,
            Some("Quarterly report"),
            "# Summary\n\nSales grew by **12%** in *Q3*.\n\n- North\n- South & West\n\n| Region | Sales |\n|---|---|\n| North | 10 |\n| South | 12 |\n",
        )
        .unwrap();
        let t = extract_text(&p).unwrap();
        for want in [
            "Quarterly report",
            "Summary",
            "Sales grew by 12% in Q3.",
            "South & West",
            "Region\tSales",
            "North\t10",
        ] {
            assert!(t.contains(want), "missing {want:?} in {t:?}");
        }
        assert_eq!(replace_in_office(&p, "North", "Nord").unwrap(), 2);
        assert!(extract_text(&p).unwrap().contains("Nord\t10"));
        assert_eq!(
            replace_in_office(&p, "<w:p>", "x").unwrap(),
            0,
            "markup is never touched"
        );
    }

    #[test]
    fn xlsx_round_trips_with_numbers_and_formulas() {
        let p = tmp("sales.xlsx");
        write_xlsx(
            &p,
            &[Sheet {
                name: "Sales".into(),
                rows: vec![
                    vec![json!("Region"), json!("Sales")],
                    vec![json!("North"), json!(10)],
                    vec![json!("South"), json!("12")],
                    vec![json!("Total"), json!("=SUM(B2:B3)")],
                ],
                header: true,
            }],
        )
        .unwrap();
        let t = extract_text(&p).unwrap();
        assert!(
            t.contains("## Sheet Sales") && t.contains("North\t10") && t.contains("South\t12"),
            "{t}"
        );
    }

    #[test]
    fn pptx_round_trips_slides() {
        let p = tmp("deck.pptx");
        write_pptx(
            &p,
            &[
                Slide {
                    title: "Launch plan".into(),
                    bullets: vec![],
                    notes: None,
                },
                Slide {
                    title: "Goals".into(),
                    bullets: vec!["Ship in May".into(), "Grow 20% & more".into()],
                    notes: None,
                },
            ],
        )
        .unwrap();
        let t = extract_text(&p).unwrap();
        assert!(
            t.contains("## Slide 1\nLaunch plan")
                && t.contains("## Slide 2")
                && t.contains("Grow 20% & more"),
            "{t}"
        );
        assert_eq!(replace_in_office(&p, "May", "June").unwrap(), 1);
    }

    /// `WORKCRAFT_DOC_SAMPLES=<dir> cargo test -p workcraft-agents samples -- --ignored` writes one file
    /// of each kind there, to open in the Craft apps, Word or LibreOffice.
    #[test]
    #[ignore = "writes sample files for checking in real office apps"]
    fn samples() {
        let Some(dir) = std::env::var_os("WORKCRAFT_DOC_SAMPLES").map(std::path::PathBuf::from)
        else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        write_docx(
            &dir.join("sample.docx"),
            Some("WorkCraft sample"),
            "# Heading one\n\nA paragraph with **bold**, *italic* and `code`.\n\n## A list\n\n- First\n- Second\n1. Numbered\n\n> A quote\n\n| Name | Value |\n|---|---|\n| Alpha | 1 |\n| Beta | 2 |\n\n```\nlet x = 1;\n```\n",
        )
        .unwrap();
        write_xlsx(
            &dir.join("sample.xlsx"),
            &[
                Sheet {
                    name: "Data".into(),
                    rows: vec![
                        vec![json!("Item"), json!("Qty"), json!("Price")],
                        vec![json!("Pens"), json!(10), json!(1.5)],
                        vec![json!("Paper"), json!(4), json!(3.25)],
                        vec![
                            json!("Total"),
                            json!("=SUM(B2:B3)"),
                            json!("=SUMPRODUCT(B2:B3,C2:C3)"),
                        ],
                    ],
                    header: true,
                },
                Sheet {
                    name: "Notes".into(),
                    rows: vec![vec![json!("Made by WorkCraft")]],
                    header: false,
                },
            ],
        )
        .unwrap();
        write_pptx(
            &dir.join("sample.pptx"),
            &[
                Slide {
                    title: "WorkCraft sample deck".into(),
                    bullets: vec![],
                    notes: None,
                },
                Slide {
                    title: "Agenda".into(),
                    bullets: vec!["Why".into(), "How".into(), "When".into()],
                    notes: None,
                },
                Slide {
                    title: "Next steps".into(),
                    bullets: vec!["Ship it".into()],
                    notes: None,
                },
            ],
        )
        .unwrap();
    }

    #[test]
    fn text_html_and_binary_files() {
        let p = tmp("page.html");
        std::fs::write(&p, "<html><style>x{}</style><body><h1>Hi</h1><p>A &amp; B</p><script>evil()</script></body></html>").unwrap();
        let t = extract_text(&p).unwrap();
        assert!(
            t.contains("Hi") && t.contains("A & B") && !t.contains("evil"),
            "{t}"
        );
        let b = tmp("blob.bin");
        std::fs::write(&b, [0u8, 1, 2, 3]).unwrap();
        assert!(extract_text(&b).is_err());
        assert_eq!(kind(&p), "file");
        assert_eq!(kind(Path::new("a.xlsx")), "spreadsheet");
        assert!(is_document(Path::new("A.PDF")));
    }
}
