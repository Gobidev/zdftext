use html_escape::decode_html_entities;
use html_parser::Element;
use std::fmt::Display;
use termion::{
    color::{self, Rgb},
    style,
};

mod char_translations;

#[derive(Debug, Clone)]
pub struct TeletextText {
    pub text: String,
    pub fg_color: u32,
    pub bg_color: u32,
}

#[derive(Debug)]
pub enum Error {
    InvalidChild,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidChild => write!(f, "Element has invalid child"),
        }
    }
}

impl std::error::Error for Error {}

fn get_colors_from_classes(classes: &[String]) -> (Option<u32>, Option<u32>) {
    let mut fg = None;
    let mut bg = None;
    for class in classes {
        if class.starts_with("bc") {
            bg = u32::from_str_radix(&class.chars().skip(2).collect::<String>(), 16).ok()
        } else if class.starts_with('c') {
            fg = u32::from_str_radix(&class.chars().skip(1).collect::<String>(), 16).ok()
        }
    }
    (fg, bg)
}

/// get separate red, green and blue color values from a combined rgb u32
fn extract_colors(combined_colors: u32) -> Rgb {
    let r = ((combined_colors >> 16) & 0xFF) as u8;
    let g = ((combined_colors >> 8) & 0xFF) as u8;
    let b = (combined_colors & 0xFF) as u8;
    Rgb(r, g, b)
}

impl Display for TeletextText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{}{}{}",
            color::Bg(extract_colors(self.bg_color)),
            color::Fg(extract_colors(self.fg_color)),
            self.text,
            style::Reset
        )
    }
}

impl Default for TeletextText {
    fn default() -> Self {
        Self::new()
    }
}

fn convert_linedraw_char(linedraw_char: char) -> char {
    if let Some(c) = char_translations::CHAR_TRANSLATIONS.get(&(linedraw_char as u32)) {
        char::from_u32(*c).unwrap()
    } else {
        linedraw_char
    }
}

fn convert_linedraw_string(text: &str) -> String {
    text.chars().map(convert_linedraw_char).collect()
}

impl TeletextText {
    pub fn new() -> Self {
        Self {
            text: "".to_string(),
            fg_color: 0,
            bg_color: 0,
        }
    }

    pub fn from_element(element: &Element) -> Result<Self, Error> {
        if element.name == "br" {
            return Ok(Self::new());
        }
        let mut text: String;
        let mut fg_color;
        let bg_color;
        (fg_color, bg_color) = get_colors_from_classes(&element.classes);

        let child = match element.children.get(0) {
            Some(child) => child,
            None => return Ok(Self::new()),
        };

        match child {
            html_parser::Node::Text(t) => text = t.clone(),
            html_parser::Node::Element(e) => {
                let sub_element = TeletextText::from_element(e)?;
                (fg_color, _) = get_colors_from_classes(&e.classes);
                text = sub_element.text;
            }
            _ => return Err(Error::InvalidChild),
        }
        let fg_color = fg_color.unwrap_or(0xFFFFFF);
        let bg_color = bg_color.unwrap_or(0x000000);

        text = decode_html_entities(&text).to_string();
        // check for special characters
        if element
            .classes
            .contains(&"teletextlinedrawregular".to_string())
        {
            text = convert_linedraw_string(&text);
        }
        Ok(Self {
            text,
            fg_color,
            bg_color,
        })
    }
}
