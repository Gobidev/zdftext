use std::collections::BTreeSet;
use std::time::Duration;

use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use futures::StreamExt;
use ratatui::{
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use tokio::sync::mpsc;
use zdftext::{fetch_page_cached, Channel, PageNumber, Result, Rgb, TeletextPage, PAGE_WIDTH};

const TICK: Duration = Duration::from_millis(150);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const HELP: [(&str, &str); 15] = [
    ("0-9", "Seite eingeben (3 Ziffern = sofort)"),
    ("Enter", "Eingegebene Seite aufrufen"),
    ("Backspace", "Ziffer löschen"),
    ("← / h / -", "Vorherige Seite"),
    ("→ / l / +", "Nächste Seite"),
    ("↑ / k", "Nächste Unterseite"),
    ("↓ / j", "Vorherige Unterseite"),
    ("g", "Seiteneingabe leeren"),
    ("r", "Neu laden"),
    ("c / C", "Nächster / vorheriger Sender"),
    ("Home", "Zur Übersicht (100)"),
    ("?", "Hilfe ein-/ausblenden"),
    ("q / Esc", "Beenden"),
    ("", ""),
    ("", "Tipp: Seitenzahlen im Text sind anklickbar"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Input,
    Help,
}

struct FetchResult {
    id: u64,
    channel: Channel,
    page: PageNumber,
    result: Result<TeletextPage>,
}

struct App {
    channel: Channel,
    page: PageNumber,
    pending_page: Option<PageNumber>,
    current: Option<TeletextPage>,
    input: String,
    mode: Mode,
    loading: bool,
    error: Option<String>,
    should_quit: bool,
    next_id: u64,
    pending_id: Option<u64>,
    tick: usize,
    page_area: Option<Rect>,
    sub_pages: u8,
    max_valid_sub: u8,
    invalid_subs: BTreeSet<u8>,
    last_base: Option<(Channel, u16)>,
    tx: mpsc::UnboundedSender<FetchResult>,
}

impl App {
    fn new(tx: mpsc::UnboundedSender<FetchResult>) -> Self {
        Self {
            channel: Channel::Zdf,
            page: PageNumber::new(100),
            pending_page: None,
            current: None,
            input: String::new(),
            mode: Mode::Normal,
            loading: false,
            error: None,
            should_quit: false,
            next_id: 0,
            pending_id: None,
            tick: 0,
            page_area: None,
            sub_pages: 0,
            max_valid_sub: 0,
            invalid_subs: BTreeSet::new(),
            last_base: None,
            tx,
        }
    }

    fn load(&mut self, page: PageNumber, force: bool) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending_id = Some(id);
        self.pending_page = Some(page);
        self.loading = true;
        self.error = None;

        let tx = self.tx.clone();
        let channel = self.channel;
        tokio::spawn(async move {
            let result = fetch_page_cached(channel, page, force).await;
            let _ = tx.send(FetchResult {
                id,
                channel,
                page,
                result,
            });
        });
    }

    fn prefetch(&self, base: u16) {
        if base == 0 {
            return;
        }
        let channel = self.channel;
        tokio::spawn(async move {
            let _ = fetch_page_cached(channel, PageNumber::new(base), false).await;
        });
    }

    fn on_fetch(&mut self, fetched: FetchResult) {
        if Some(fetched.id) != self.pending_id || fetched.channel != self.channel {
            return;
        }
        self.loading = false;
        self.pending_page = None;
        match fetched.result {
            Ok(page) => {
                self.reset_sub_state_if_moved(fetched.channel, fetched.page);
                self.sub_pages = self.sub_pages.max(page.sub_pages);
                if fetched.page.sub > 0 {
                    self.max_valid_sub = self.max_valid_sub.max(fetched.page.sub);
                }
                self.page = fetched.page;
                self.prefetch(page.prev);
                self.prefetch(page.next);
                self.current = Some(page);
            }
            Err(error) => {
                let message = error.to_string();
                // Remember sub-pages that do not exist so navigation can skip
                // them instead of repeatedly hitting the same 404.
                if fetched.page.sub > 0 && message.contains("404") {
                    self.invalid_subs.insert(fetched.page.sub);
                }
                self.error = Some(short_error(&message));
            }
        }
    }

    fn reset_sub_state_if_moved(&mut self, channel: Channel, page: PageNumber) {
        let base = (channel, page.base);
        if self.last_base != Some(base) {
            self.last_base = Some(base);
            self.sub_pages = 0;
            self.max_valid_sub = 0;
            self.invalid_subs.clear();
        }
    }

    /// Highest sub-page index that is worth offering. `0` is the base page.
    fn max_sub_index(&self) -> u8 {
        // The `subpages` attribute is unreliable: base pages often report `1`
        // even when sub-pages exist, while snapshots report a total that may
        // exceed the pages that actually resolve. Probe one step past the
        // highest sub-page we have successfully loaded so far.
        let declared = self.sub_pages.saturating_sub(1);
        declared.min(self.max_valid_sub.saturating_add(1)).max(1)
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        if self.mode != Mode::Normal
            || !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        {
            return;
        }
        let Some(area) = self.page_area else {
            return;
        };
        if mouse.column < area.x
            || mouse.column >= area.x + area.width
            || mouse.row < area.y
            || mouse.row >= area.y + area.height
        {
            return;
        }
        let col = (mouse.column - area.x) as usize;
        let row = (mouse.row - area.y) as usize;
        let target = self.current.as_ref().and_then(|page| {
            page.links
                .iter()
                .find(|link| link.row == row && col >= link.col && col < link.col + link.len)
                .map(|link| link.page)
        });
        if let Some(page) = target {
            self.load(PageNumber::new(page), false);
        }
    }

    fn submit_input(&mut self) {
        let number: u16 = match self.input.parse() {
            Ok(number) => number,
            Err(_) => {
                self.input.clear();
                self.mode = Mode::Normal;
                return;
            }
        };
        self.input.clear();
        self.mode = Mode::Normal;
        self.load(PageNumber::new(number), false);
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match self.mode {
            Mode::Help => self.on_key_help(key),
            Mode::Input => self.on_key_input(key),
            Mode::Normal => self.on_key_normal(key),
        }
    }

    fn on_key_help(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => self.mode = Mode::Normal,
            _ => {}
        }
    }

    fn on_key_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => {
                self.input.pop();
                if self.input.is_empty() {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if self.input.len() < 3 {
                    self.input.push(c);
                }
                if self.input.len() == 3 {
                    self.submit_input();
                }
            }
            _ => {}
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) {
        let current = self.current.as_ref();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('r') => self.load(self.page, true),
            KeyCode::Char('g') => {
                self.input.clear();
                self.mode = Mode::Input;
            }
            KeyCode::Char('c') => {
                self.channel = self.channel.next();
                self.load(self.page, false);
            }
            KeyCode::Char('C') => {
                self.channel = self.channel.prev();
                self.load(self.page, false);
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.input.clear();
                self.input.push(c);
                self.mode = Mode::Input;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
                if let Some(prev) = current.map(|p| p.prev).filter(|p| *p != 0) {
                    self.load(PageNumber::new(prev), false);
                }
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') => {
                if let Some(next) = current.map(|p| p.next).filter(|p| *p != 0) {
                    self.load(PageNumber::new(next), false);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.change_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.change_sub(-1),
            KeyCode::Home => self.load(PageNumber::new(100), false),
            _ => {}
        }
    }

    fn change_sub(&mut self, direction: i8) {
        if self.current.is_none() {
            return;
        }
        let max = self.max_sub_index() as i16;
        let size = max + 1;
        let start = self.page.sub.min(max as u8) as i16;
        let mut current = start;
        // Step until a sub-page is found that has not previously returned 404.
        for _ in 0..size {
            current = (current + direction as i16).rem_euclid(size);
            if current == start {
                // Wrapped all the way around without finding a valid sub-page.
                return;
            }
            if self.invalid_subs.contains(&(current as u8)) {
                continue;
            }
            let target = if current == 0 {
                PageNumber::new(self.page.base)
            } else {
                PageNumber::with_sub(self.page.base, current as u8)
            };
            self.load(target, false);
            return;
        }
    }

    fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn status_line(&self) -> Line<'static> {
        let shown = self.pending_page.unwrap_or(self.page);
        let mut spans = vec![
            Span::styled(
                format!(" {} ", self.channel.name()),
                Style::new().fg(Color::Black).bg(Color::White).bold(),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" Seite {}", shown.base),
                Style::new().fg(Color::White).bold(),
            ),
        ];
        if shown.sub > 0 {
            spans.push(Span::styled(
                format!("/{}", shown.sub),
                Style::new().fg(Color::Cyan),
            ));
        }
        let max_sub = self.max_sub_index();
        if max_sub > 1 || self.max_valid_sub > 0 {
            spans.push(Span::styled(
                format!(" ({}/{})", self.page.sub.min(max_sub), max_sub),
                Style::new().fg(Color::DarkGray),
            ));
        }
        if self.loading {
            spans.push(Span::styled(
                format!("  {} lädt…", SPINNER[self.tick % SPINNER.len()]),
                Style::new().fg(Color::Yellow),
            ));
        }
        if let Some(error) = &self.error {
            spans.push(Span::styled(
                format!("  Fehler: {error}"),
                Style::new().fg(Color::Red),
            ));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "? Hilfe · q Beenden",
            Style::new().fg(Color::DarkGray),
        ));
        Line::from(spans)
    }

    fn input_line(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled(" Seite: ", Style::new().fg(Color::Yellow)),
            Span::styled(
                format!("{}_", self.input),
                Style::new().fg(Color::White).bold(),
            ),
        ])
    }
}

fn rgb(color: Rgb) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

fn short_error(message: &str) -> String {
    let message = message.lines().next().unwrap_or("").trim();
    let lower = message.to_ascii_lowercase();
    let friendly = if lower.contains("404") {
        "Seite nicht gefunden".to_string()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "Zeitüberschreitung".to_string()
    } else if lower.contains("connect") {
        "Keine Verbindung".to_string()
    } else {
        message.to_string()
    };
    let mut truncated: String = friendly.chars().take(56).collect();
    if friendly.chars().count() > 56 {
        truncated.push('…');
    }
    truncated
}

fn page_lines(page: &TeletextPage) -> Vec<Line<'static>> {
    page.rows
        .iter()
        .map(|row| {
            if row.is_empty() {
                return Line::from("");
            }
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut text = String::new();
            let mut fg = row[0].fg;
            let mut bg = row[0].bg;
            for cell in row {
                if cell.fg != fg || cell.bg != bg {
                    spans.push(Span::styled(
                        std::mem::take(&mut text),
                        Style::new().fg(rgb(fg)).bg(rgb(bg)),
                    ));
                    fg = cell.fg;
                    bg = cell.bg;
                }
                text.push(cell.ch);
            }
            spans.push(Span::styled(text, Style::new().fg(rgb(fg)).bg(rgb(bg))));
            Line::from(spans)
        })
        .collect()
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn page_area_for(area: Rect, page: &TeletextPage) -> Option<Rect> {
    let height = page.rows() as u16;
    if area.width < PAGE_WIDTH as u16 || area.height < height {
        return None;
    }
    Some(centered_rect(area, PAGE_WIDTH as u16, height))
}

fn render_page(frame: &mut Frame, app: &App, area: Rect, page_area: Option<Rect>) {
    let Some(page) = &app.current else {
        if !app.loading {
            let message = Paragraph::new("Keine Seite geladen")
                .alignment(Alignment::Center)
                .style(Style::new().fg(Color::DarkGray));
            frame.render_widget(message, area);
        }
        return;
    };

    let Some(page_area) = page_area else {
        let message = Paragraph::new("Fenster zu klein für ZDFtext (min. 40x25)")
            .alignment(Alignment::Center)
            .style(Style::new().fg(Color::Yellow));
        frame.render_widget(message, area);
        return;
    };

    let paragraph = Paragraph::new(page_lines(page)).style(Style::new().bg(Color::Black));
    frame.render_widget(paragraph, page_area);
}

fn render_help(frame: &mut Frame, area: Rect) {
    let width = 56.min(area.width);
    let height = (HELP.len() as u16 + 4).min(area.height);
    let popup = centered_rect(area, width, height);
    frame.render_widget(Clear, popup);

    let lines = HELP
        .iter()
        .map(|(key, description)| {
            let prefix = if key.is_empty() {
                String::new()
            } else {
                format!(" {key:<12}")
            };
            Line::from(vec![
                Span::styled(prefix, Style::new().fg(Color::Cyan).bold()),
                Span::raw(*description),
            ])
        })
        .collect::<Vec<_>>();

    let block = Block::new()
        .borders(Borders::ALL)
        .title(" Tastenkürzel ")
        .title_alignment(Alignment::Center)
        .style(Style::new().bg(Color::Black).fg(Color::White));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::new().bg(Color::Black)), area);

    let [content, status] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let page_area = app
        .current
        .as_ref()
        .and_then(|page| page_area_for(content, page));
    app.page_area = page_area;
    render_page(frame, app, content, page_area);

    let bar = if app.mode == Mode::Input {
        app.input_line()
    } else {
        app.status_line()
    };
    frame.render_widget(
        Paragraph::new(bar).style(Style::new().bg(Color::Black)),
        status,
    );

    if app.mode == Mode::Help {
        render_help(frame, area);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<FetchResult>();
    let mut app = App::new(tx);
    app.load(PageNumber::new(100), false);

    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &mut rx).await;
    ratatui::restore();
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture)?;
    result
}

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<FetchResult>,
) -> Result<()> {
    let mut reader = EventStream::new();
    let mut ticker = tokio::time::interval(TICK);

    loop {
        terminal.draw(|frame| ui(frame, app))?;

        tokio::select! {
            maybe_event = reader.next() => match maybe_event {
                Some(Ok(Event::Key(key))) => app.on_key(key),
                Some(Ok(Event::Mouse(mouse))) => app.on_mouse(mouse),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.into()),
                None => break,
            },
            Some(fetched) = rx.recv() => app.on_fetch(fetched),
            _ = ticker.tick() => app.tick(),
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let (tx, _rx) = mpsc::unbounded_channel();
        App::new(tx)
    }

    #[test]
    fn always_offers_at_least_one_subpage() {
        assert_eq!(app().max_sub_index(), 1);
    }

    #[test]
    fn probes_one_past_the_highest_valid_subpage() {
        let mut app = app();
        app.sub_pages = 12;
        app.max_valid_sub = 3;
        assert_eq!(app.max_sub_index(), 4);
    }

    #[test]
    fn never_exceeds_the_declared_total() {
        let mut app = app();
        app.sub_pages = 2;
        app.max_valid_sub = 9;
        assert_eq!(app.max_sub_index(), 1);
    }

    #[test]
    fn maps_common_errors_to_short_messages() {
        assert_eq!(
            short_error("HTTP status client error (404 Not Found) for url (https://x)"),
            "Seite nicht gefunden"
        );
        assert_eq!(short_error("operation timed out"), "Zeitüberschreitung");
    }
}
