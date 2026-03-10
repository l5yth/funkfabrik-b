# FUNKFABRIK\*B

A teletext-aesthetic radio station website, rebuilt from a 15-year-old Drupal 7
site as a modern Rust application.

## Design

Faithful recreation of 1980s European teletext: black background, primary-color
palette (red / green / yellow / cyan / blue), chunky AEnigma bitmap font, fixed
960 px "screen" with a blue 20 px bezel, blinking elements, and live clock.

Navigate like a remote control — type any three-digit page number on the keyboard.

## Pages

| No. | Title |
|-----|-------|
| 100 | Startseite |
| 101 | Radio hören |
| 170 | Wettermagazin |
| 300 | Fanseite |
| 666 | Kontakt |
| 777 | Spiele |
| 999 | Impressum |

## Stack

- **Rust 2024 edition** — no PHP, Node, or Ruby
- **Axum 0.7** — async HTTP server
- **Tera 1** — Jinja2-style HTML templates
- **tower-http** — static file serving
- Vanilla CSS + vanilla JS (zero front-end dependencies)

## Run

```sh
cargo run
```

Serves on `http://0.0.0.0:3000`. Templates and static files are loaded from the
working directory at runtime, so run from the project root.

## Project layout

```
.
├── src/           Rust source
├── templates/     Tera HTML templates (base + one per page)
├── static/
│   ├── style.css  Teletext stylesheet
│   ├── teletext.js  Clock, scroll, remote-control nav
│   └── fonts/     AEnigma.woff
├── cache/         Original Drupal static-HTML archive (reference only)
├── Cargo.toml
├── CLAUDE.md      Architecture & design notes for AI assistants
└── LICENSE        Apache 2.0
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `↑` / `↓` | Scroll content viewport |
| `0`–`9` | Type page number (3 digits → navigate) |
| `Esc` | Cancel digit input |

## License

Copyright (c) 2006-2026 afri & veit.
Licensed under the [Apache License, Version 2.0](LICENSE).
