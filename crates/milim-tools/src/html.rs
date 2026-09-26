//! Small HTML-to-readable-text conversion for web tools.
//!
//! This is not a conforming HTML parser. It tokenizes tags and text, drops
//! page chrome (scripts, styles, navigation, footers), and renders the
//! remaining structure as Markdown-ish text that a model can read cheaply.

/// Output flavor for [`html_to_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextFlavor {
    /// Headings, list markers, `[text](url)` links, code fences, and tables.
    Markdown,
    /// The same structure without Markdown markup or link targets.
    Plain,
}

/// Elements whose whole subtree is dropped.
const SKIPPED_ELEMENTS: &[&str] = &[
    "script", "style", "noscript", "svg", "nav", "footer", "template", "iframe", "canvas",
    "select", "button",
];
/// Elements whose content is raw text rather than markup.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "textarea", "title"];
const BLOCK_ELEMENTS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "aside",
    "blockquote",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "details",
    "summary",
    "address",
    "dl",
    "dt",
    "dd",
    "table",
    "caption",
];

/// Convert an HTML document to readable text, resolving relative links
/// against `base` when given.
pub(crate) fn html_to_text(html: &str, base: Option<&reqwest::Url>, flavor: TextFlavor) -> String {
    let mut writer = Writer::new(base, flavor);
    let mut title = None;
    let mut index = 0;
    let bytes = html.as_bytes();
    while index < html.len() {
        let Some(offset) = html[index..].find('<') else {
            writer.text(&html[index..]);
            break;
        };
        let start = index + offset;
        writer.text(&html[index..start]);
        let rest = &html[start..];
        if rest.starts_with("<!--") {
            index = rest
                .find("-->")
                .map(|end| start + end + 3)
                .unwrap_or(html.len());
            continue;
        }
        let next = bytes.get(start + 1).copied().unwrap_or(b' ');
        if !(next.is_ascii_alphabetic() || next == b'/' || next == b'!' || next == b'?') {
            writer.text("<");
            index = start + 1;
            continue;
        }
        let Some(end) = tag_end(html, start) else {
            break;
        };
        index = end + 1;
        if next == b'!' || next == b'?' {
            continue;
        }
        let tag = parse_tag(&html[start + 1..end]);
        if tag.name.is_empty() {
            continue;
        }
        if !tag.closing && RAW_TEXT_ELEMENTS.contains(&tag.name.as_str()) && !tag.self_closing {
            let close = find_closing_tag(html, index, &tag.name);
            let inner = &html[index..close.0];
            index = close.1;
            if tag.name == "title" && title.is_none() {
                let text = collapse_whitespace(&decode_entities(inner));
                if !text.is_empty() {
                    title = Some(text);
                }
            } else if tag.name == "textarea" && writer.skip_depth == 0 {
                writer.text(inner);
            }
            continue;
        }
        writer.tag(&tag);
    }
    let body = writer.finish();
    match title {
        Some(title) if flavor == TextFlavor::Markdown => format!("# {title}\n\n{body}"),
        Some(title) => format!("{title}\n\n{body}"),
        None => body,
    }
    .trim()
    .to_string()
}

/// Strip tags from an HTML fragment and decode entities, collapsing whitespace.
pub(crate) fn strip_tags(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for character in fragment.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    collapse_whitespace(&decode_entities(&out))
        .replace(" ,", ",")
        .replace(" .", ".")
}

/// Index of the `>` that closes the tag starting at `start`, honoring quotes.
fn tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, byte) in html.as_bytes()[start + 1..].iter().enumerate() {
        match (quote, byte) {
            (Some(open), value) if *value == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(*byte),
            (None, b'>') => return Some(start + 1 + offset),
            _ => {}
        }
    }
    html[start..].find('>').map(|end| start + end)
}

/// Find `</name ...>` case-insensitively from `from`; returns the inner-text
/// end and the index after the closing tag (both the document end when absent).
fn find_closing_tag(html: &str, from: usize, name: &str) -> (usize, usize) {
    let bytes = html.as_bytes();
    let mut search = from;
    while let Some(found) = html[search..].find("</") {
        let at = search + found;
        let name_start = at + 2;
        let name_end = name_start + name.len();
        let matches_name = bytes
            .get(name_start..name_end)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name.as_bytes()));
        let terminated = matches!(
            bytes.get(name_end),
            None | Some(b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/')
        );
        if matches_name && terminated {
            let close = html[name_end..]
                .find('>')
                .map(|end| name_end + end + 1)
                .unwrap_or(html.len());
            return (at, close);
        }
        search = name_start;
    }
    (html.len(), html.len())
}

#[derive(Debug, Default)]
struct Tag {
    name: String,
    closing: bool,
    self_closing: bool,
    attributes: Vec<(String, String)>,
}

impl Tag {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn parse_tag(source: &str) -> Tag {
    let mut tag = Tag::default();
    let mut rest = source.trim();
    if let Some(stripped) = rest.strip_prefix('/') {
        tag.closing = true;
        rest = stripped.trim_start();
    }
    if let Some(stripped) = rest.strip_suffix('/') {
        tag.self_closing = true;
        rest = stripped.trim_end();
    }
    let name_end = rest
        .find(|character: char| character.is_whitespace() || character == '/')
        .unwrap_or(rest.len());
    tag.name = rest[..name_end].to_ascii_lowercase();
    let mut attrs = rest[name_end..].trim_start();
    while !attrs.is_empty() {
        attrs = attrs
            .trim_start_matches(|character: char| character.is_whitespace() || character == '/');
        let key_end = attrs
            .find(|character: char| character.is_whitespace() || character == '=')
            .unwrap_or(attrs.len());
        if key_end == 0 {
            break;
        }
        let key = attrs[..key_end].to_ascii_lowercase();
        attrs = attrs[key_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = attrs.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let body = &after[1..];
                let end = body.find(quote).unwrap_or(body.len());
                value = decode_entities(&body[..end]);
                attrs = body.get(end + 1..).unwrap_or("");
            } else {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                value = decode_entities(&after[..end]);
                attrs = &after[end..];
            }
        }
        tag.attributes.push((key, value));
    }
    tag
}

struct ListState {
    ordered: bool,
    next: usize,
}

struct TableState {
    rows: usize,
    cells_in_row: usize,
    header_cells: usize,
    separator_written: bool,
}

struct Writer<'a> {
    out: String,
    base: Option<&'a reqwest::Url>,
    flavor: TextFlavor,
    pending_space: bool,
    skip_depth: usize,
    skip_name: String,
    pre_depth: usize,
    lists: Vec<ListState>,
    links: Vec<(usize, Option<String>)>,
    tables: Vec<TableState>,
}

impl<'a> Writer<'a> {
    fn new(base: Option<&'a reqwest::Url>, flavor: TextFlavor) -> Self {
        Self {
            out: String::new(),
            base,
            flavor,
            pending_space: false,
            skip_depth: 0,
            skip_name: String::new(),
            pre_depth: 0,
            lists: Vec::new(),
            links: Vec::new(),
            tables: Vec::new(),
        }
    }

    fn markdown(&self) -> bool {
        self.flavor == TextFlavor::Markdown
    }

    fn text(&mut self, raw: &str) {
        if self.skip_depth > 0 || raw.is_empty() {
            return;
        }
        let decoded = decode_entities(raw);
        if self.pre_depth > 0 {
            let decoded = if self.out.ends_with('\n') {
                decoded.strip_prefix('\n').unwrap_or(&decoded)
            } else {
                &decoded
            };
            self.out.push_str(decoded);
            return;
        }
        for character in decoded.chars() {
            if character.is_whitespace() {
                self.pending_space = true;
                continue;
            }
            if self.pending_space
                && !self.out.is_empty()
                && !self
                    .out
                    .ends_with(|c: char| c.is_whitespace() || c == '[' || c == '(')
            {
                self.out.push(' ');
            }
            self.pending_space = false;
            self.out.push(character);
        }
    }

    fn raw(&mut self, text: &str) {
        self.pending_space = false;
        self.out.push_str(text);
    }

    fn inline_space(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with(char::is_whitespace) {
            self.pending_space = true;
        }
    }

    fn line_break(&mut self, count: usize) {
        self.pending_space = false;
        while self.out.ends_with([' ', '\t']) {
            self.out.pop();
        }
        if self.out.is_empty() {
            return;
        }
        let existing = self.out.chars().rev().take_while(|c| *c == '\n').count();
        for _ in existing..count {
            self.out.push('\n');
        }
    }

    fn tag(&mut self, tag: &Tag) {
        let name = tag.name.as_str();
        if self.skip_depth > 0 {
            if name == self.skip_name {
                if tag.closing {
                    self.skip_depth -= 1;
                } else if !tag.self_closing {
                    self.skip_depth += 1;
                }
            }
            return;
        }
        if SKIPPED_ELEMENTS.contains(&name) {
            if !tag.closing && !tag.self_closing {
                self.skip_depth = 1;
                self.skip_name = name.to_string();
            }
            return;
        }
        let heading = heading_level(name);
        match (name, tag.closing) {
            (_, false) if heading.is_some() => {
                self.line_break(2);
                if self.markdown() {
                    self.raw(&format!("{} ", "#".repeat(heading.unwrap_or(1))));
                }
            }
            (_, true) if heading.is_some() => self.line_break(2),
            ("br", _) => self.line_break(1),
            ("hr", false) => {
                self.line_break(2);
                if self.markdown() {
                    self.raw("---");
                }
                self.line_break(2);
            }
            ("pre", false) => {
                self.line_break(2);
                if self.markdown() && self.pre_depth == 0 {
                    self.raw("```\n");
                }
                self.pre_depth += 1;
            }
            ("pre", true) if self.pre_depth > 0 => {
                self.pre_depth -= 1;
                if self.pre_depth == 0 {
                    while self.out.ends_with(['\n', ' ', '\t']) {
                        self.out.pop();
                    }
                    if self.markdown() {
                        self.raw("\n```");
                    }
                }
                self.line_break(2);
            }
            ("code", _) if self.pre_depth == 0 && self.markdown() => {
                if !tag.closing {
                    self.inline_space_flush();
                }
                self.raw("`");
            }
            ("ul" | "ol", false) => {
                self.line_break(1);
                let start = tag
                    .attribute("start")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);
                self.lists.push(ListState {
                    ordered: name == "ol",
                    next: start,
                });
            }
            ("ul" | "ol", true) => {
                self.lists.pop();
                self.line_break(if self.lists.is_empty() { 2 } else { 1 });
            }
            ("li", false) => {
                self.line_break(1);
                let depth = self.lists.len().max(1) - 1;
                let marker = match self.lists.last_mut() {
                    Some(list) if list.ordered => {
                        let marker = format!("{}. ", list.next);
                        list.next += 1;
                        marker
                    }
                    _ => "- ".to_string(),
                };
                self.raw(&format!("{}{marker}", "  ".repeat(depth)));
            }
            ("li", true) => self.line_break(1),
            ("a", false) => {
                let href = tag
                    .attribute("href")
                    .and_then(|href| self.resolve_link(href));
                self.inline_space_flush();
                let start = self.out.len();
                if self.markdown() && href.is_some() {
                    self.raw("[");
                }
                self.links.push((start, href));
            }
            ("a", true) => {
                if let Some((start, href)) = self.links.pop() {
                    if let Some(href) = href.filter(|_| self.markdown()) {
                        let text = self.out[start + 1..].trim().to_string();
                        if text.is_empty() {
                            self.out.truncate(start);
                        } else {
                            self.out.truncate(start + 1);
                            self.out.push_str(&text);
                            self.raw(&format!("]({href})"));
                        }
                    }
                }
            }
            ("img", false) => {
                let alt = tag.attribute("alt").map(str::trim).unwrap_or("");
                if !alt.is_empty() {
                    self.inline_space_flush();
                    if self.markdown() {
                        match tag.attribute("src").and_then(|src| self.resolve_link(src)) {
                            Some(src) => self.raw(&format!("![{alt}]({src})")),
                            None => self.raw(&format!("![{alt}]")),
                        }
                    } else {
                        self.raw(alt);
                    }
                }
            }
            ("table", false) => {
                self.line_break(2);
                self.tables.push(TableState {
                    rows: 0,
                    cells_in_row: 0,
                    header_cells: 0,
                    separator_written: false,
                });
            }
            ("table", true) => {
                self.tables.pop();
                self.line_break(2);
            }
            ("tr", false) => {
                self.line_break(1);
                if let Some(table) = self.tables.last_mut() {
                    table.cells_in_row = 0;
                }
            }
            ("tr", true) => {
                let markdown = self.markdown();
                let separator = self.tables.last_mut().and_then(|table| {
                    table.rows += 1;
                    let cells = table.cells_in_row.max(table.header_cells);
                    (markdown && !table.separator_written && table.rows == 1 && cells > 0).then(
                        || {
                            table.separator_written = true;
                            format!("\n|{}", " --- |".repeat(cells))
                        },
                    )
                });
                if let Some(separator) = separator {
                    self.raw(&separator);
                }
                self.line_break(1);
            }
            ("td" | "th", false) => {
                let first = self
                    .tables
                    .last()
                    .map(|table| table.cells_in_row == 0)
                    .unwrap_or(true);
                if first {
                    self.raw(if self.markdown() { "| " } else { "" });
                }
            }
            ("td" | "th", true) => {
                if let Some(table) = self.tables.last_mut() {
                    table.cells_in_row += 1;
                    if name == "th" && table.rows == 0 {
                        table.header_cells += 1;
                    }
                }
                while self.out.ends_with([' ', '\t', '\n']) {
                    self.out.pop();
                }
                self.raw(if self.markdown() { " | " } else { "\t" });
            }
            (_, _) if BLOCK_ELEMENTS.contains(&name) => {
                self.line_break(if name == "dd" || name == "dt" { 1 } else { 2 })
            }
            ("span" | "label", _) => self.inline_space(),
            _ => {}
        }
    }

    fn inline_space_flush(&mut self) {
        if self.pending_space && !self.out.is_empty() && !self.out.ends_with(char::is_whitespace) {
            self.out.push(' ');
        }
        self.pending_space = false;
    }

    fn resolve_link(&self, href: &str) -> Option<String> {
        let href = href.trim();
        if href.is_empty() || href.starts_with('#') {
            return None;
        }
        let lower = href.to_ascii_lowercase();
        if lower.starts_with("javascript:") || lower.starts_with("data:") {
            return None;
        }
        match self.base {
            Some(base) => base.join(href).ok().map(|url| url.to_string()),
            None => Some(href.to_string()),
        }
    }

    fn finish(self) -> String {
        let mut result = String::with_capacity(self.out.len());
        let mut blank_run = 0;
        let mut in_fence = false;
        for line in self.out.lines() {
            let line = line.trim_end();
            if line.starts_with("```") {
                in_fence = !in_fence;
            }
            if line.is_empty() && !in_fence {
                blank_run += 1;
                if blank_run > 1 {
                    continue;
                }
            } else {
                blank_run = 0;
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    }
}

fn heading_level(name: &str) -> Option<usize> {
    match name.as_bytes() {
        [b'h', digit @ b'1'..=b'6'] => Some((digit - b'0') as usize),
        _ => None,
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode named (common subset) and numeric character references.
pub(crate) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest
            .char_indices()
            .take(12)
            .find(|(_, c)| *c == ';')
            .map(|(index, _)| index);
        let decoded = end.and_then(|end| decode_entity(&rest[1..end]).map(|value| (value, end)));
        match decoded {
            Some((value, end)) => {
                out.push(value);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "times" => '×',
        "divide" => '÷',
        "deg" => '°',
        "euro" => '€',
        "pound" => '£',
        "cent" => '¢',
        "yen" => '¥',
        "sect" => '§',
        "para" => '¶',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "shy" => '\u{AD}',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!DOCTYPE html>
<html><head><title>Example &amp; Co</title>
<style>body { color: red; }</style>
<script>var x = "<p>not text</p>";</script></head>
<body>
<nav><a href="/home">Home</a> <a href="/about">About</a></nav>
<main>
<h1>Main   heading</h1>
<p>First <b>para</b> with a <a href="/docs/intro?x=1">relative link</a> and
an <a href="https://other.example/">absolute one</a>.&nbsp;Caf&#233; &#x263A;</p>
<ul><li>One</li><li>Two<ol><li>Nested</li></ol></li></ul>
<pre><code>fn main() {
    println!("hi");
}</code></pre>
<p>Inline <code>let x = 1;</code> code.</p>
<table><tr><th>Name</th><th>Value</th></tr><tr><td>a</td><td>1</td></tr></table>
<noscript>Enable JS</noscript>
<svg><text>chart</text></svg>
<!-- a comment <p>hidden</p> -->
</main>
<footer>Copyright <a href="/legal">Legal</a></footer>
</body></html>"#;

    fn base() -> reqwest::Url {
        reqwest::Url::parse("https://example.com/guide/page.html").unwrap()
    }

    #[test]
    fn markdown_keeps_structure_and_drops_chrome() {
        let text = html_to_text(PAGE, Some(&base()), TextFlavor::Markdown);
        assert!(text.starts_with("# Example & Co\n"), "{text}");
        assert!(text.contains("\n# Main heading\n"), "{text}");
        assert!(text.contains("[relative link](https://example.com/docs/intro?x=1)"));
        assert!(text.contains("[absolute one](https://other.example/)"));
        assert!(text.contains("Café ☺"), "{text}");
        assert!(text.contains("- One\n- Two\n  1. Nested"), "{text}");
        assert!(
            text.contains("```\nfn main() {\n    println!(\"hi\");\n}\n```"),
            "{text}"
        );
        assert!(text.contains("Inline `let x = 1;` code."), "{text}");
        assert!(
            text.contains("| Name | Value |\n| --- | --- |\n| a | 1 |"),
            "{text}"
        );
        for dropped in [
            "color: red",
            "not text",
            "Home",
            "Enable JS",
            "chart",
            "hidden",
            "Copyright",
            "Legal",
        ] {
            assert!(!text.contains(dropped), "{dropped} leaked: {text}");
        }
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn plain_flavor_omits_markup() {
        let text = html_to_text(PAGE, Some(&base()), TextFlavor::Plain);
        assert!(text.starts_with("Example & Co\n"));
        assert!(text.contains("relative link and"));
        assert!(!text.contains("](") && !text.contains("```") && !text.contains("# "));
    }

    #[test]
    fn entities_and_fragments_decode() {
        assert_eq!(
            decode_entities("a &lt;b&gt; &amp;amp; &#39;c&#39; &bogus; &"),
            "a <b> &amp; 'c' &bogus; &"
        );
        assert_eq!(
            strip_tags("The <b>quick</b> fox &amp; <i>dog</i>."),
            "The quick fox & dog."
        );
    }

    #[test]
    fn stray_angle_brackets_and_unclosed_tags_do_not_panic() {
        let text = html_to_text(
            "<p>1 < 2 and 3 > 2</p><p>tail <a href=\"x",
            None,
            TextFlavor::Markdown,
        );
        assert!(text.contains("1 < 2 and 3 > 2"), "{text}");
        let text = html_to_text("<script>never closed", None, TextFlavor::Markdown);
        assert!(text.is_empty());
    }
}
