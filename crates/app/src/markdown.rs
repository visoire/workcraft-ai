//! A small Markdown reader for chat text: headings, lists, quotes, code blocks, bold, italic and inline
//! code. It reads partial text too, so a half-streamed answer renders while it arrives.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    /// Nesting level, marker (`-` or `1.`), content.
    Item(usize, String, Vec<Span>),
    Quote(Vec<Span>),
    Code {
        lang: String,
        text: String,
    },
    Rule,
}

#[must_use]
pub fn parse_inline(s: &str) -> Vec<Span> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let (mut bold, mut italic, mut code) = (false, false, false);
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<Span>, bold: bool, italic: bool, code: bool| {
        if !cur.is_empty() {
            out.push(Span {
                text: std::mem::take(cur),
                bold,
                italic,
                code,
            });
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            flush(&mut cur, &mut out, bold, italic, code);
            code = !code;
            i += 1;
        } else if code {
            cur.push(c);
            i += 1;
        } else if c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(&mut cur, &mut out, bold, italic, code);
            bold = !bold;
            i += 2;
        } else if c == '*' && chars.get(i + 1).is_some_and(|n| !n.is_whitespace())
            || c == '*' && italic
        {
            flush(&mut cur, &mut out, bold, italic, code);
            italic = !italic;
            i += 1;
        } else {
            cur.push(c);
            i += 1;
        }
    }
    flush(&mut cur, &mut out, bold, italic, code);
    out
}

fn plain(s: &str) -> Vec<Span> {
    parse_inline(s)
}

#[must_use]
pub fn parse_blocks(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut code: Option<(String, Vec<&str>)> = None;

    let flush_para = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(plain(&para.join(" "))));
            para.clear();
        }
    };

    for line in src.lines() {
        if let Some((lang, lines)) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                blocks.push(Block::Code {
                    lang: std::mem::take(lang),
                    text: lines.join("\n"),
                });
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("```") {
            flush_para(&mut para, &mut blocks);
            code = Some((rest.trim().to_string(), Vec::new()));
        } else if t.is_empty() {
            flush_para(&mut para, &mut blocks);
        } else if t.chars().all(|c| c == '-' || c == '*' || c == '_') && t.len() >= 3 {
            flush_para(&mut para, &mut blocks);
            blocks.push(Block::Rule);
        } else if t.starts_with('#') {
            let level = t.chars().take_while(|c| *c == '#').count();
            if level <= 6 && t[level..].starts_with(' ') {
                flush_para(&mut para, &mut blocks);
                blocks.push(Block::Heading(level as u8, plain(t[level..].trim())));
            } else {
                para.push(t);
            }
        } else if let Some(rest) = t.strip_prefix("> ") {
            flush_para(&mut para, &mut blocks);
            blocks.push(Block::Quote(plain(rest)));
        } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush_para(&mut para, &mut blocks);
            blocks.push(Block::Item(
                (line.len() - t.len()) / 2,
                "\u{2022}".to_string(),
                plain(rest),
            ));
        } else if let Some((num, rest)) = t
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()))
        {
            flush_para(&mut para, &mut blocks);
            blocks.push(Block::Item(
                (line.len() - t.len()) / 2,
                format!("{num}."),
                plain(rest),
            ));
        } else {
            para.push(t);
        }
    }
    // An unclosed fence is still code: the answer is probably streaming.
    if let Some((lang, lines)) = code {
        blocks.push(Block::Code {
            lang,
            text: lines.join("\n"),
        });
    }
    if !para.is_empty() {
        blocks.push(Block::Paragraph(plain(&para.join(" "))));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn inline_styles() {
        let s = parse_inline("a **bold** and *it* and `x*y` end");
        assert_eq!(text(&s), "a bold and it and x*y end");
        assert!(s.iter().any(|x| x.bold && x.text == "bold"));
        assert!(s.iter().any(|x| x.italic && x.text == "it"));
        assert!(s.iter().any(|x| x.code && x.text == "x*y"));
        // A lone star between spaces is just a star.
        assert_eq!(text(&parse_inline("2 * 3")), "2 * 3");
        assert!(parse_inline("").is_empty());
    }

    #[test]
    fn blocks() {
        let b = parse_blocks(
            "# Title\n\nSome *text*\nmore\n\n- one\n- two\n1. first\n\n> quoted\n\n---\n```rust\nfn main() {}\n```\nafter",
        );
        assert!(matches!(&b[0], Block::Heading(1, _)));
        assert!(matches!(&b[1], Block::Paragraph(p) if text(p) == "Some text more"));
        assert!(matches!(&b[2], Block::Item(0, m, _) if m == "\u{2022}"));
        assert!(matches!(&b[4], Block::Item(0, m, _) if m == "1."));
        assert!(matches!(&b[5], Block::Quote(_)));
        assert!(matches!(&b[6], Block::Rule));
        assert!(
            matches!(&b[7], Block::Code { lang, text } if lang == "rust" && text == "fn main() {}")
        );
        assert!(matches!(&b[8], Block::Paragraph(p) if text(p) == "after"));
    }

    #[test]
    fn unclosed_fence_renders_as_code_while_streaming() {
        let b = parse_blocks("Here:\n```python\nprint('hi')");
        assert!(matches!(&b[1], Block::Code { text, .. } if text == "print('hi')"));
    }

    #[test]
    fn hash_without_space_is_text_and_odd_input_is_safe() {
        assert!(matches!(&parse_blocks("#hashtag")[0], Block::Paragraph(_)));
        assert!(parse_blocks("").is_empty());
        let _ = parse_blocks("```\n```\n****\n**\n`");
    }
}
