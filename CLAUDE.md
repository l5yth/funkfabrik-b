# CLAUDE.md — FUNKFABRIK\*B

Architecture and design reference for AI assistants working on this project.

---

## Project overview

FUNKFABRIK\*B is a German internet radio station website styled as a 1980s
European teletext service (think Ceefax, ARD-Text). The original site ran on
Drupal 7 (~2010). This repo is a ground-up rewrite in Rust, preserving the
visual identity exactly while modernising the entire stack.

The original static-HTML download lives in `cache/` for reference only.

---

## Tech stack

| Layer | Choice | Notes |
|-------|--------|-------|
| Language | Rust 2024 edition | No PHP, Node, or Ruby |
| HTTP server | Axum 0.7 | async, tower-compatible |
| Templates | Tera 1 | Jinja2 syntax, files in `templates/` |
| Static files | tower-http `ServeDir` | mounted at `/static` |
| CSS | Vanilla | no preprocessor, no framework |
| JS | Vanilla | no bundler, no dependencies |

Run with `cargo run` from the project root. Server binds `0.0.0.0:3000`.

---

## Directory layout

```
.
├── src/main.rs          Single-file Rust backend
├── templates/
│   ├── base.html        Master layout (header, nav, viewport, footer)
│   ├── 100.html         Startseite
│   ├── 101.html         Radio hören
│   ├── 170.html         Wettermagazin
│   ├── 300.html         Fanseite
│   ├── 666.html         Kontakt (form placeholder)
│   ├── 777.html         Spiele
│   ├── 999.html         Impressum
│   └── 404.html         Fallback for unknown page numbers
├── static/
│   ├── style.css        All visual styling
│   ├── teletext.js      Runtime interactions
│   └── fonts/
│       └── AEnigma.woff Teletext bitmap font (from original site)
├── cache/               Original Drupal HTML archive (do not modify)
│   ├── LhB7ljSZZu9h.de/ Downloaded site assets
│   └── 127.0.0.1_8081/  Local crawler artifacts
├── Cargo.toml
├── Cargo.lock
├── LICENSE              Apache 2.0
├── README.md
└── CLAUDE.md            This file
```

---

## Routing (`src/main.rs`)

- `GET /` → 308 redirect to `/100`
- `GET /:page` → render `templates/{page}.html` with Tera context
- Unknown page → render `templates/404.html`
- `GET /static/*` → serve files from `static/`

Tera is initialised once at startup and shared via `Arc<Tera>` in `AppState`.

### Template context

Every page receives:

| Variable | Type | Description |
|----------|------|-------------|
| `current_page` | `String` | e.g. `"100"` |
| `page_title` | `&str` | e.g. `"Startseite"` |
| `pages` | `Vec<{num, title}>` | all pages, for the nav bar |

---

## Visual design

### Color palette (exact hex values from original)

| Name | Hex | Usage |
|------|-----|-------|
| Black | `#000000` | Background, form inputs |
| White | `#FFFFFF` | Body text, links |
| Red | `#FC0204` | `h1`, submit buttons, nav item 1/5 |
| Green | `#04FE04` | Form borders, nav item 2/6 |
| Yellow | `#FCFE04` | `h2`, station name, nav item 3/7 |
| Cyan | `#04FEFC` | `h3`, page-info header, nav item 4 |
| Blue | `#0402FC` | Screen bezel (20 px border), header/footer bar |

CSS custom properties are defined as `--black`, `--white`, `--red`, `--green`,
`--yellow`, `--cyan`, `--blue` at the `:root`.

### Layout

- `body` — black, centred, flex column
- `#screen` — 960 px wide, 20 px solid blue border (the "TV bezel")
- `#header` — blue bar: station name (yellow) | page info (cyan) | clock (white)
- `#nav` — black bar with inline page-number links, cycling colors via
  `nth-child(4n+1..4)`: red → green → yellow → cyan
- Active nav item: white background, black text (`!important` overrides color cycle)
- `#viewport` — 440 px tall, `overflow: hidden`; content scrolls via JS transform
- `#content-scroll` — CSS `transition: transform 0.35s` for smooth scroll
- `#footer` — blue bar: hint text (white) | station name (yellow)

### Typography

Font: `AEnigma` (WOFF), declared as `font-family: 'Teletext'`. Falls back to
`'Courier New', monospace`. Base size: `20px`.

### Heading hierarchy

```
h1 — red  (#FC0204), 10 px red bottom border
h2 — yellow (#FCFE04), 6 px yellow bottom border
h3 — cyan (#04FEFC), no border
```

---

## JavaScript (`static/teletext.js`)

Three self-contained IIFEs, no global state leakage:

### 1. Live clock
Updates `#clock` every second. Colons blink by toggling `visibility: hidden` on
odd seconds — matching the original jQuery implementation but without jQuery.

### 2. Arrow-key scroll
Tracks a `offset` variable; on `ArrowUp`/`ArrowDown` applies
`translateY(-Npx)` to `#content-scroll`, clamped to the scrollable range.
Step size: 300 px.

### 3. Remote-control digit input
- Number keys accumulate up to 3 digits into a string.
- `#remote` div (bottom-right overlay, yellow border) shows typed digits +
  underscores for empty slots.
- After 3 digits: 250 ms flash then `window.location.href` navigation.
- Clears automatically after 3 s of inactivity or on `Escape`.
- Ignored when focus is in `<input>` or `<textarea>`.

---

## CSS utilities

Colour helper classes available in all templates:

```
.color-red  .color-green  .color-yellow  .color-cyan  .color-white
.bg-red     .bg-green     .bg-yellow     .bg-cyan     .bg-blue
```

Page index list (used on 100, 404):
```html
<ul class="page-index">
  <li><a href="/NNN">
    <span class="pg-num">NNN</span>
    <span class="pg-title">Title</span>
  </a></li>
```

Blink: add class `.blink` to any element — driven by CSS `@keyframes`, no JS.

---

## Template authoring

Child templates extend `base.html`:

```jinja
{% extends "base.html" %}

{% block page_title %}PAGE TITLE{% endblock %}

{% block content %}
  <!-- content here -->
{% endblock %}
```

> **Note:** Tera requires `{% extends %}` to be the very first token in a child
> template. No comments, whitespace, or other tags may precede it — the parser
> will reject the file. Copyright headers for child templates are therefore
> omitted; the copyright is covered by `base.html`, `LICENSE`, and `CLAUDE.md`.

Add a new page:
1. Create `templates/NNN.html`
2. Add `("NNN", "Title")` to the `PAGES` array in `src/main.rs`

---

## License & copyright

Copyright (c) 2006-2026 afri & veit.
Apache License 2.0 — see `LICENSE`.

Header convention:
- `.rs` files: full two-paragraph Apache boilerplate (`// Copyright …` × 13 lines)
- All other files: two-line short form (`Copyright` + `SPDX-License-Identifier`)
- `base.html`: uses `{# … #}` Tera comment syntax
- Child templates: **no header** (Tera parser restriction — `{% extends %}` must be first)
- CSS uses `/* … */`, JS and TOML use `//` / `#`
