---
name: tensaku
description: Work through Tensaku notes, the comments a person left on pages of this Rails app in development, each pointing at the exact file and line that rendered the element. Use when the user mentions Tensaku, notes on the page, "my notes", or asks you to go through what they marked up in the browser.
---

# Tensaku notes

In this repository the Rails app is the documentation site in `docs/`: note paths are relative to `docs/`, and note files are in `docs/tmp/tensaku/notes/`. There is no MCP server entry, because each checkout's site runs at its own Parcela URL and port.

Tensaku is a development-only overlay for this Rails app. The person clicks an element (or drags over an
area) on a running page and writes what should change. Each note records the page URL, the comment, and
one or more targets with the source position Herb stamped on the element.

## Finding notes

Use whichever is available, in this order:

1. **MCP**: the `tensaku` server (`/__tensaku/mcp` on the dev server). Tools: `list_notes`, `get_note`,
   `start_note`, `resolve_note`, `reply_to_note`, `ask_question`, `watch_notes`, `current_selection`. The dev server must be running.
2. **Files**: `tmp/tensaku/notes/*.json`, one note per file. Read them directly when the server isn't
   running. Edit `status` and `reply` in the file to resolve one.

A note looks like this:

```json
{
  "id": "a7f3",
  "url": "/about",
  "comment": "tighten this, and the heading is wrong",
  "targets": [
    { "kind": "element", "file": "app/views/pages/_hero.html.erb", "range": "4:3-4:40", "tag": "h1", "text": "About me",
      "context": { "partials": ["app/views/pages/_hero.html.erb", "app/views/pages/about.html.erb"], "controller": "hero" } },
    { "kind": "translation", "key": "pages.about.title", "locale": "en", "file": "config/locales/en.yml", "range": "12" }
  ],
  "rect": { "x": 120, "y": 340, "width": 600, "height": 180 },
  "status": "open",
  "reply": null
}
```

- `range` is `line:column-line:column` in `file` (1-based). For translations it's the line of the key.
- `kind` is `element`, `text` (the person selected that text), `translation` (the text came from that
  I18n key) or `markdown` (a Markdown source block).
- `rect` is set for area notes: the person dragged over a region, and `targets` lists the elements in it.
- An element rendered in a loop or a repeated partial has one source position for every copy.
- Text from Ruby (`<%= @post.title %>`) points at the ERB expression; the data itself lives elsewhere.

## Working through notes

1. List the open notes (`list_notes`, or read the files with `"status": "open"`).
2. Call `start_note` on the note you take, so the person sees it's being worked on. Then open the target file at the range, read the comment, and make the change. Look at
   `context.partials` when the fix belongs in a parent template, and at the translation target when the
   text comes from a locale file (change every locale that needs it).
3. Read the note's `thread`: the person's answers to earlier questions are there. Use `reply_to_note`
   to say what you found when that helps the person. If the comment is ambiguous, don't guess: call `ask_question` with a short, specific question. The
   question shows on the page and the note waits for an answer.
4. When done, call `resolve_note` with a one-line reply that says what changed and where, for example
   `Shortened the intro in app/views/pages/_hero.html.erb:6`. The pin on the page turns green so the
   person can check it.

To keep going while the person adds notes, call `watch_notes` in a loop, passing the previous
`checked_at` as `since`.

You can take a screenshot of `url` with a browser tool if the layout matters; notes don't include one.
