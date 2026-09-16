use html_escape::decode_html_entities;
use html_parser::{Element, Node};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fmt::Display;
use std::time::Duration;
use tokio::sync::Mutex;

mod char_translations;

/// Width of a teletext page in characters.
pub const PAGE_WIDTH: usize = 40;

const DEFAULT_FG: Rgb = Rgb {
    r: 0xFF,
    g: 0xFF,
    b: 0xFF,
};
const DEFAULT_BG: Rgb = Rgb {
    r: 0x00,
    g: 0x00,
    b: 0x00,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("zdftext/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
});

static CACHE: Lazy<Mutex<HashMap<(Channel, PageNumber), TeletextPage>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// A single 24-bit color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// A single teletext character cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
}

impl Cell {
    pub fn blank() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        }
    }
}

/// An in-page link to another page. `col` is the column within `row` and `len`
/// the number of cells it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    pub page: u16,
    pub row: usize,
    pub col: usize,
    pub len: usize,
}

/// A fully parsed teletext page.
#[derive(Debug, Clone, Default)]
pub struct TeletextPage {
    pub rows: Vec<Vec<Cell>>,
    pub prev: u16,
    pub next: u16,
    pub sub_pages: u8,
    pub links: Vec<Link>,
}

impl TeletextPage {
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Zdf,
    ZdfInfo,
    ZdfNeo,
    DreiSat,
}

impl Channel {
    pub const ALL: [Channel; 4] = [
        Channel::Zdf,
        Channel::ZdfInfo,
        Channel::ZdfNeo,
        Channel::DreiSat,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Channel::Zdf => "ZDF",
            Channel::ZdfInfo => "ZDFinfo",
            Channel::ZdfNeo => "ZDFneo",
            Channel::DreiSat => "3sat",
        }
    }

    pub fn next(self) -> Self {
        let idx = Channel::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Channel::ALL[(idx + 1) % Channel::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Channel::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Channel::ALL[(idx + Channel::ALL.len() - 1) % Channel::ALL.len()]
    }
}

impl Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Channel::Zdf => "zdf",
                Channel::ZdfInfo => "zdfinfo",
                Channel::ZdfNeo => "zdfneo",
                Channel::DreiSat => "3sat",
            }
        )
    }
}

/// Identifies a page and its sub-page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageNumber {
    pub base: u16,
    pub sub: u8,
}

impl PageNumber {
    pub fn new(base: u16) -> Self {
        Self { base, sub: 0 }
    }

    pub fn with_sub(base: u16, sub: u8) -> Self {
        Self { base, sub }
    }
}

impl Display for PageNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.sub == 0 {
            write!(f, "{}", self.base)
        } else {
            write!(f, "{}_{}", self.base, self.sub)
        }
    }
}

fn parse_hex_color(hex: &str) -> Option<Rgb> {
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgb {
        r: ((value >> 16) & 0xFF) as u8,
        g: ((value >> 8) & 0xFF) as u8,
        b: (value & 0xFF) as u8,
    })
}

/// Foreground color classes are either `cRRGGBB` or `#RRGGBB`.
fn fg_from_classes(classes: &[String]) -> Option<Rgb> {
    classes.iter().find_map(|class| {
        class
            .strip_prefix('c')
            .or_else(|| class.strip_prefix('#'))
            .and_then(parse_hex_color)
    })
}

/// Background color classes are `bcRRGGBB`.
fn bg_from_classes(classes: &[String]) -> Option<Rgb> {
    classes
        .iter()
        .find_map(|class| class.strip_prefix("bc").and_then(parse_hex_color))
}

fn is_linedraw(classes: &[String]) -> bool {
    classes
        .iter()
        .any(|class| class == "teletextlinedrawregular")
}

fn convert_linedraw_char(linedraw_char: char) -> char {
    if let Some(c) = char_translations::CHAR_TRANSLATIONS.get(&(linedraw_char as u32)) {
        char::from_u32(*c).unwrap_or(linedraw_char)
    } else {
        linedraw_char
    }
}

fn push_text(text: &str, fg: Rgb, bg: Rgb, linedraw: bool, out: &mut Vec<Cell>) {
    for ch in decode_html_entities(text).chars() {
        // `&nbsp;` decodes to a non-breaking space which many terminals render
        // with the wrong width/background; normalize it to a regular space.
        let ch = if ch == '\u{a0}' { ' ' } else { ch };
        let ch = if linedraw {
            convert_linedraw_char(ch)
        } else {
            ch
        };
        out.push(Cell { ch, fg, bg });
    }
}

fn parse_href_page(href: &str) -> Option<u16> {
    let path = href.strip_suffix(".html")?;
    // Ignore external links and sub-page suffixes.
    let base = path.split(['_', '/']).next()?;
    base.parse().ok()
}

fn parse_inline(
    element: &Element,
    parent_fg: Option<Rgb>,
    parent_bg: Option<Rgb>,
    parent_linedraw: bool,
    row: usize,
    links: &mut Vec<Link>,
    out: &mut Vec<Cell>,
) {
    let fg = fg_from_classes(&element.classes)
        .or(parent_fg)
        .unwrap_or(DEFAULT_FG);
    let bg = bg_from_classes(&element.classes)
        .or(parent_bg)
        .unwrap_or(DEFAULT_BG);
    let linedraw = parent_linedraw || is_linedraw(&element.classes);

    let link_page = element
        .attributes
        .get("href")
        .and_then(|value| value.as_deref())
        .and_then(parse_href_page);

    let start = out.len();
    for child in &element.children {
        match child {
            Node::Text(text) => push_text(text, fg, bg, linedraw, out),
            Node::Element(child) => {
                parse_inline(child, Some(fg), Some(bg), linedraw, row, links, out)
            }
            Node::Comment(_) => {}
        }
    }

    if let Some(page) = link_page {
        links.push(Link {
            page,
            row,
            col: start,
            len: out.len() - start,
        });
    }
}

fn parse_row(row: &Element, row_index: usize, links: &mut Vec<Link>) -> Vec<Cell> {
    let mut cells = Vec::new();
    for child in &row.children {
        match child {
            Node::Element(element) if element.name == "br" => break,
            Node::Element(element) => {
                parse_inline(element, None, None, false, row_index, links, &mut cells)
            }
            Node::Text(text) => {
                if !text.trim().is_empty() {
                    push_text(text, DEFAULT_FG, DEFAULT_BG, false, &mut cells);
                }
            }
            Node::Comment(_) => {}
        }
    }
    cells.truncate(PAGE_WIDTH);
    while cells.len() < PAGE_WIDTH {
        cells.push(Cell::blank());
    }
    cells
}

fn find_element<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Element> {
    for node in nodes {
        if let Node::Element(element) = node {
            if element.name == name {
                return Some(element);
            }
            if let Some(found) = find_element(&element.children, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Collect all elements that are page rows (`class="row"`) in document order.
fn collect_rows<'a>(nodes: &'a [Node], out: &mut Vec<&'a Element>) {
    for node in nodes {
        if let Node::Element(element) = node {
            if element.classes.iter().any(|class| class == "row") {
                out.push(element);
            } else {
                collect_rows(&element.children, out);
            }
        }
    }
}

fn attribute_u8(element: &Element, name: &str) -> u8 {
    element
        .attributes
        .get(name)
        .and_then(|value| value.as_deref())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn attribute_u16(element: &Element, name: &str) -> u16 {
    element
        .attributes
        .get(name)
        .and_then(|value| value.as_deref())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Parse the HTML returned by ZDF into a [`TeletextPage`].
pub fn parse_page(html: &str) -> Result<TeletextPage> {
    let dom = html_parser::Dom::parse(html)?;
    let body =
        find_element(&dom.children, "body").ok_or("response does not contain a <body> element")?;

    let mut links = Vec::new();
    let mut row_elements = Vec::new();
    collect_rows(&body.children, &mut row_elements);

    let rows = row_elements
        .iter()
        .enumerate()
        .map(|(index, row)| parse_row(row, index, &mut links))
        .collect();

    Ok(TeletextPage {
        rows,
        prev: attribute_u16(body, "prevpg"),
        next: attribute_u16(body, "nextpg"),
        sub_pages: attribute_u8(body, "subpages"),
        links,
    })
}

/// Fetch the raw HTML for a page.
pub async fn request_html(channel: Channel, page: PageNumber) -> Result<String> {
    let url = format!("https://teletext.zdf.de/teletext/{channel}/seiten/klassisch/{page}.html");
    let response = CLIENT.get(url).send().await?.error_for_status()?;
    Ok(response.text().await?)
}

/// Fetch and parse a page.
pub async fn fetch_page(channel: Channel, page: PageNumber) -> Result<TeletextPage> {
    let html = request_html(channel, page).await?;
    parse_page(&html)
}

/// Fetch a page using the in-memory cache unless `force` is set.
pub async fn fetch_page_cached(
    channel: Channel,
    page: PageNumber,
    force: bool,
) -> Result<TeletextPage> {
    let key = (channel, page);
    if !force {
        if let Some(cached) = CACHE.lock().await.get(&key).cloned() {
            return Ok(cached);
        }
    }
    let fetched = fetch_page(channel, page).await?;
    CACHE.lock().await.insert(key, fetched.clone());
    Ok(fetched)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hash_and_c_prefixed_colors() {
        let classes = vec![
            "teletextlinedrawregular".to_string(),
            "#0000FF".to_string(),
            "bc00FF00".to_string(),
        ];
        assert_eq!(fg_from_classes(&classes), Some(Rgb { r: 0, g: 0, b: 255 }));
        assert_eq!(bg_from_classes(&classes), Some(Rgb { r: 0, g: 255, b: 0 }));

        let classes = vec!["teletextregular".to_string(), "cFF0000".to_string()];
        assert_eq!(fg_from_classes(&classes), Some(Rgb { r: 255, g: 0, b: 0 }));
    }

    #[test]
    fn rejects_malformed_colors() {
        assert_eq!(parse_hex_color("FFF"), None);
        assert_eq!(parse_hex_color("GGGGGG"), None);
        assert_eq!(fg_from_classes(&["bc000000".to_string()]), None);
    }

    #[test]
    fn parses_href_pages() {
        assert_eq!(parse_href_page("120.html"), Some(120));
        assert_eq!(parse_href_page("100_1.html"), Some(100));
        assert_eq!(parse_href_page("https://example.com"), None);
        assert_eq!(parse_href_page("not-a-page.html"), None);
    }

    #[test]
    fn inherits_parent_color_into_child() {
        let html = r#"<html><body prevpg="1" nextpg="2" subpages="3">
            <div class="row"><span class="cFF0000"><span>hi</span></span></div>
        </body></html>"#;
        let page = parse_page(html).unwrap();
        assert_eq!(page.prev, 1);
        assert_eq!(page.next, 2);
        assert_eq!(page.sub_pages, 3);
        assert_eq!(page.rows[0][0].fg, Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(page.rows[0].len(), PAGE_WIDTH);
    }

    #[test]
    fn records_links() {
        let html = r#"<html><body><div class="row">
            <span class="bc000000"><a class="cFFFFFF" href="120.html">120</a></span>
        </div></body></html>"#;
        let page = parse_page(html).unwrap();
        assert_eq!(
            page.links,
            vec![Link {
                page: 120,
                row: 0,
                col: 0,
                len: 3
            }]
        );
    }

    #[test]
    fn normalizes_nbsp_and_translates_linedraw() {
        let html = r#"<html><body><div class="row">
            <span class="teletextlinedrawregular cFFFFFF">&nbsp;A</span>
        </div></body></html>"#;
        let page = parse_page(html).unwrap();
        assert_eq!(page.rows[0][0].ch, ' ');
        assert_eq!(page.rows[0][1].ch, '\u{283F}');
    }

    #[test]
    fn missing_body_is_an_error() {
        assert!(parse_page("<html><head></head></html>").is_err());
    }
}
