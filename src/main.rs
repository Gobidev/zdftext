use html_parser::{Dom, Element, Node};
use std::{error::Error, io};
use zdftext::{request_teletext, Channel, Page, TeletextText};

fn get_element(input: Option<&Node>) -> &Element {
    if let Node::Element(e) = input.expect("Parser error") {
        e
    } else {
        panic!("Parser error")
    }
}

fn show_dom(dom: Dom) -> Result<(), Box<dyn Error>> {
    let content = get_element(dom.children.get(0));
    let body = get_element(content.children.get(1));
    let rows = get_element(body.children.get(1)).children.clone();
    let rows_parsed: Vec<Vec<TeletextText>> = rows
        .into_iter()
        .map(|e| match e {
            Node::Element(e) => e
                .children
                .iter()
                .map(|e| match e {
                    Node::Element(e) => TeletextText::from_element(&e.clone()).unwrap(),
                    _ => panic!("Parser error"),
                })
                .collect(),
            _ => panic!("Parser error"),
        })
        .collect();
    for row in rows_parsed {
        for text in row {
            print!("{text}");
        }
        println!();
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut number = 100;
    let mut number_before = number;
    loop {
        let response = request_teletext(Channel::ZDF, Page::new(100)).await;
        let parsed_response = if let Ok(v) = Dom::parse(&response?) {
            v
        } else {
            println!("Error");
            number = number_before;
            continue;
        };
        drop(show_dom(parsed_response));
        println!("Enter number: ");

        number_before = number;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read line");
        number = input.trim().parse()?;
    }
}
