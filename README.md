<!--
SPDX-FileCopyrightText: 2006-2026 afri & veit
SPDX-License-Identifier: Apache-2.0
-->

# FUNKFABRIK\*B

German internet radio station website styled as a 1980s European teletext service.

Live at **[funkfabrik-b.de](https://funkfabrik-b.de)**

## Design

- Black background, primary-color palette (red / green / yellow / cyan / blue)
- AEnigma bitmap font
- Fixed 960 px screen, blue 20 px bezel
- Blinking elements, live clock

## Pages

| No. | Title | Description |
|-----|-------|-------------|
| 100 | Startseite | Index |
| 101 | Radio hören | Stream & Archiv |
| 170 | Wettermagazin | Berlin weather (wttr.in) |
| 300 | 20 Jahre Brutto | Event, 14.-17. Mai 2026 |
| 404 | Fanseite | Fan archive |
| 666 | Kontakt | Guestbook & contact |
| 777 | Spiele | Tetris, Space Invaders, Snake |
| 999 | Impressum | Legal |

## Stack

| Layer | Choice |
|-------|--------|
| Language | Rust 2024 edition |
| HTTP server | Axum 0.8 |
| Templates | Tera 2 (Jinja2 syntax) |
| Static files | tower-http `ServeDir` |
| CSS / JS | Vanilla, zero front-end dependencies |

## Prerequisites

- [Rust](https://rustup.rs) stable toolchain

## Quick start

```sh
git clone <repo>
cd funkfabrik-b
cargo run
```

Run from the project root. Serves on `http://0.0.0.0:3000`.

## Configuration

| Variable | Default | Effect |
|----------|---------|--------|
| `FUNKFABRIK_ADDR` | `0.0.0.0:3000` | Listen address and port |

```sh
FUNKFABRIK_ADDR=127.0.0.1:8080 cargo run
```

## Development

```sh
cargo test                       # run all tests
cargo clippy -- -D warnings      # lint
cargo doc --no-deps --open       # browse docs
```

## Project layout

```
.
├── src/
│   ├── main.rs          Server, routing, weather & RSS handlers
│   └── guestbook.rs     Guestbook persistence (JSON)
├── templates/
│   ├── base.html        Master layout (header, nav, viewport, footer)
│   ├── 1xx / 7xx        One template per page
│   └── not_found.html   Fallback for unknown page numbers
├── static/
│   ├── style.css        Teletext stylesheet
│   ├── teletext.js      Clock, scroll (keys + swipe + wheel), remote-control nav
│   ├── img/             Images (event flyers, etc.)
│   └── fonts/           AEnigma.woff
├── data/                Runtime data, guestbook.json (git-ignored)
├── cache/               Original Drupal static-HTML archive (reference only)
├── Cargo.toml
└── LICENSE              Apache 2.0
```

## Navigation

| Input | Action |
|-------|--------|
| `↑` / `↓` | Scroll content |
| Swipe up / down | Scroll content (mobile) |
| Mouse wheel | Scroll content |
| `0`-`9` | Type page number, 3 digits navigates |
| `Esc` | Cancel digit input |

## License

Copyright (c) 2006-2026 afri & veit.
Licensed under the [Apache License, Version 2.0](LICENSE).
