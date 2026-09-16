use zdftext::{parse_page, Rgb, PAGE_WIDTH};

#[test]
fn parses_real_page_fixture() {
    let html = include_str!("fixtures/page100.html");
    let page = parse_page(html).unwrap();

    assert!(!page.rows.is_empty());
    assert!(page.rows.len() >= 24, "expected at least 24 rows");
    assert!(page.rows.iter().all(|row| row.len() == PAGE_WIDTH));
    assert_eq!(page.next, 101);
    assert!(page.links.iter().any(|link| link.page == 200));

    // No cell should keep the raw HTML entity text.
    let raw = page
        .rows
        .iter()
        .flatten()
        .map(|cell| cell.ch)
        .collect::<String>();
    assert!(!raw.contains("&nbsp;"));
    assert!(!raw.contains('\u{a0}'));

    // `#`-prefixed classes must resolve to real colors.
    let blue = Rgb { r: 0, g: 0, b: 255 };
    assert!(page.rows.iter().flatten().any(|cell| cell.fg == blue));
}
