use html_escape::decode_html_entities;
use html_parser::{Dom, Element, Node};
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use std::{fmt::Display, io};

#[derive(Debug, Clone)]
struct Text {
    text: String,
    fg_color: u32,
    bg_color: u32,
}

impl Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fg_r = ((self.fg_color >> 16) & 0xFF) as u8;
        let fg_g = ((self.fg_color >> 8) & 0xFF) as u8;
        let fg_b = (self.fg_color & 0xFF) as u8;
        let bg_r = ((self.bg_color >> 16) & 0xFF) as u8;
        let bg_g = ((self.bg_color >> 8) & 0xFF) as u8;
        let bg_b = (self.bg_color & 0xFF) as u8;

        let colored_text = self.text.on_truecolor(bg_r, bg_g, bg_b);
        let colored_text = colored_text.truecolor(fg_r, fg_g, fg_b);
        write!(f, "{colored_text}")
    }
}

impl Text {
    fn from_element(element: Element) -> Self {
        let use_parent_bg: bool;
        let text_element = if let Some(Node::Element(e)) = element.children.get(0) {
            use_parent_bg = true;
            e.clone()
        } else {
            use_parent_bg = false;
            element.clone()
        };

        let mut fg = 0;
        let mut bg = 0;
        let mut text = "".to_string();
        for class in text_element.classes {
            if class.starts_with("bc") {
                let class_base = class.chars().skip(2).collect::<String>();
                bg = u32::from_str_radix(&class_base, 16).expect("Invalid color");
            } else if class.starts_with('c') {
                let class_base = class.chars().skip(1).collect::<String>();
                fg = u32::from_str_radix(&class_base, 16).expect("Invalid color");
            }
        }
        if use_parent_bg {
            for class in element.classes {
                if class.starts_with("bc") {
                    let class_base = class.chars().skip(2).collect::<String>();
                    bg = u32::from_str_radix(&class_base, 16).expect("Invalid color");
                }
            }
        }

        if let Some(Node::Text(t)) = text_element.children.get(0) {
            text = t.clone();
        }

        let text = decode_html_entities(&text).to_string();
        // let text = text.replace("&nbsp;", " ");

        Self {
            text,
            fg_color: fg,
            bg_color: bg,
        }
    }
}

fn get_element(input: Option<&Node>) -> &Element {
    if let Node::Element(e) = input.expect("Parser error") {
        e
    } else {
        panic!("Parser error")
    }
}

fn show_dom(dom: Dom) {
    let content = get_element(dom.children.get(0));
    let body = get_element(content.children.get(1));
    let rows = get_element(body.children.get(1)).children.clone();
    let ids: Vec<Vec<Text>> = rows
        .into_iter()
        .map(|e| match e {
            Node::Element(e) => e
                .children
                .iter()
                .map(|e| match e {
                    Node::Element(e) => Text::from_element(e.clone()),
                    _ => panic!("asdf2"),
                })
                .collect(),
            _ => panic!("asdf"),
        })
        .collect();
    for line in ids {
        for text in line {
            print!("{text}");
        }
        println!();
    }
}

static CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut number = "100".to_string();
    let mut number_before = number.clone();
    loop {
        let response = CLIENT
            .get(format!(
                "https://teletext.zdf.de/teletext/zdf/seiten/klassisch/{number}.html"
            ))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        let parsed_response = if let Ok(v) = Dom::parse(&response) {
            v
        } else {
            println!("Error");
            number = number_before.clone();
            continue;
        };
        show_dom(parsed_response);

        number_before = number;
        number = String::new();
        io::stdin()
            .read_line(&mut number)
            .expect("Failed to read line");
    }
}
