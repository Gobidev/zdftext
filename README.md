# zdftext

A terminal client for the German [ZDFtext](https://teletext.zdf.de/) teletext service.

It fetches a page, parses the HTML into a 40×25 character grid and renders it in a
full-screen TUI with true teletext colors, block graphics and keyboard navigation.

```
                               100   ZDFtext   Mi 16.09.26   12:00:14
                                 ⠠⠾⠿⠿⠦
                                 ⠿⠿⠿⠖⠊⠏⠑⠮⠍       Mit dem Zweiten
                                 ⠻⠿⠿⠬⠭⠓⠊⠃        sieht man besser
                                  ⠙⠛⠛⠁
                               ⠿ EU will soziale Medien erst ab 13 120
                               ⠿ Rüstungsvereinbarung mit den USA  123
                               ⠿ Huthi-Drohne bei Mekka abgewehrt  125

 ZDF   Seite 100 ? Hilfe · q Beenden
```

## Usage

```sh
cargo run --release
```

## Key bindings

| Key        | Action                                    |
| ---------- | ----------------------------------------- |
| `0-9`      | Enter a page number (3 digits go at once) |
| `Enter`    | Open the typed page                       |
| `Backspace`| Delete a digit                            |
| `←`/`h`/`-`| Previous page                             |
| `→`/`l`/`+`| Next page                                 |
| `↑`/`k`    | Next sub-page                             |
| `↓`/`j`    | Previous sub-page                         |
| `g`        | Clear the page input                      |
| `r`        | Reload the current page                   |
| `c` / `C`  | Next / previous channel                   |
| `Home`     | Jump to the index (page 100)              |
| `?`        | Toggle the help overlay                   |
| `q` / `Esc`| Quit                                      |

Page numbers shown in the text are links and can be clicked with the mouse.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The parser and fetch layer live in `src/lib.rs`; the TUI lives in `src/main.rs`.
`tests/fixtures/page100.html` is a saved page used by the parser tests.
