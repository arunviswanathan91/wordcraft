# Manuscript projects: design

> **Last updated:** 2026-11-07 · **Status:** proposal, nothing implemented. Describes a
> project-based manuscript and thesis workflow: each chapter or section is its own document,
> several are edited in one window as a continuous stack, and compile joins them into one DOCX,
> PDF or LaTeX file, or into chosen parts such as the front pages or each chapter. Page
> numbering, placement of the cover, contents and index, and thesis templates are set in the
> binder. Heading numbers (3, 3.2, 3.2.1) come from levels, are checked and repaired, and typed
> numbers in imported documents are detected and converted. Open questions at the end need a maintainer decision before work starts.

## Why

Long documents such as theses, books and reports are hard to manage as one Word file:

- **Slow editing.** A change on page 12 can re-paginate all 300 pages.
- **Pictures jump.** A floating picture that moves, or text that reflows around one, can push
  everything after it onto different pages.
- **Hard to restructure.** Writers reorder chapters, keep notes and research beside the text,
  track how much they have written, and want to see the structure without scrolling.

Applications built for this split the manuscript into a tree of small documents, show them
together while writing, and **compile** them into one formatted document at the end.

WordCraft already has most of what that final document needs:
- headings, outline view, captions with chapter numbering, cross-references;
- tables of contents and figures, footnotes;
- citations with a bibliography, and Zotero;
- DOCX, PDF and LaTeX export.

What it lacks is the project layer around all this. This document designs that layer.

## Goals

1. **A project is a folder of ordinary documents.** Each section is a `.docx` that opens in Word,
   LibreOffice or WordCraft on its own, so a supervisor can review one chapter with tracked
   changes and send it back.
2. **Many documents, one window.** The editor shows several sections stacked as one continuous
   scroll. The engine only lays out the sections on screen, so editing speed depends on the size
   of a chapter, not of the whole thesis (see [Stacked view](#stacked-view-many-documents-one-window)).
3. **Pictures can't disturb other chapters.** Each chapter is its own document that starts on a
   new page. A picture that moves or reflows can only affect pages of its own chapter, and
   figure defaults keep it from moving at all (see [Figures](#figures)).
4. **A binder.** A tree of folders and sections, reordered by drag or by command. Each item has a
   title, synopsis, label, status and word-count target.
5. **Compile.** One command combines the included sections in binder order into a single
   document, applies a format preset, rebuilds the generated content and exports DOCX, PDF, ODT
   or LaTeX. It works the same in the app, the CLI and over MCP.
6. **Numbering, cross-references and citations that span sections** come out right in the
   compiled document.
7. **Numbers that track themselves.** Heading numbers come from levels, problems are found and
   repaired, and typed numbers in imported documents are converted to real numbering.
8. **Snapshots.** Saved versions of a section that can be compared and restored.
9. **Never lose work.** Same rules as the rest of WordCraft: no panics, hostile input is bounded,
   and a damaged project file never stops the sections from opening.

## Non-goals (for the first version)

- **Real-time collaboration or cloud sync.** The project is a folder, so users can sync it with
  git, Syncthing or a cloud drive themselves.
- **A new file format for section text.** Sections are `.docx`.
- **A selection or a single edit spanning two sections** in the stacked view. Moving text between
  sections works by cut and paste, or with `binder.split` / `binder.merge`.
- **Exact final pagination while writing.** The stacked view paginates each section on its own;
  `compile.preview` shows the true pages.

## Concepts

| Term | Meaning |
|---|---|
| **Project** | A folder ending in `.wcproj` holding `project.toml` and the subfolders below |
| **Binder** | The tree of items in `project.toml` |
| **Item** | A folder (groups items, may carry its own text) or a section |
| **Section** | One `.docx` under `sections/`, named by the item's id, never by its title |
| **Stack** | The items shown together in the editor: by default, the children of the selected folder |
| **Research** | Files kept with the project but never compiled (PDFs, images, notes) |
| **Snapshot** | A saved copy of a section at a moment, with an optional label |
| **Compile preset** | Rules that turn the binder into one document: template, which items, separators, numbering, figure handling, output format |

## Stacked view: many documents, one window

This is the core of the design. Each section stays a separate document in the engine, and the
editor draws them one after another as if they were one.

### Why it is faster

- Paragraph shaping is already cached between layouts (`LayoutCache`,
  `crates/layout/src/cache.rs`). Pagination, however, runs over the whole document. I found no
  incremental pagination in `crates/layout` or `crates/engine/src/session/layout.rs`.
- Gap 20 in [gaps.md](gaps.md) records that large documents aren't measured and that opening a
  1,096-page one still lays out slowly.
- With one document per section, an edit re-lays out only the section being edited. Typing in a
  20-page chapter of a 300-page thesis paginates 20 pages.
- Compile and `compile.preview` are the only times the whole thesis is laid out, and the user asks
  for them explicitly.

### How it works

- **Documents.** The session already holds several open documents (`Session::add_document`,
  `Session::with_document`, `crates/engine/src/session/documents.rs`). Each section shown in the
  stack is one of them, so editing, undo, autosave and recovery work as they do today.
- **Layout.**
  - The stack lays out only the sections on screen, plus one before and one after.
  - Each section's height comes from its own layout, or, while it isn't laid out, from its page
    count cached in the project (see [Caches](#caches)).
  - The scroll bar covers the whole stack from those numbers.
- **Memory.**
  - At most a fixed number of sections are open at once (for example 12).
  - A section that scrolls away is saved if changed, then closed. Scrolling back reopens it.
  - Sections are small, so this is quick.
- **Pages.**
  - Each section starts on a new page in the stack, with a thin divider showing its title.
  - Page numbers in the stack add the cached page counts of earlier sections, so they read close
    to the final numbers.
  - Where the preset joins sections without a page break, the compiled pages differ from the
    stack. `compile.preview` shows the real result.
- **Caret and selection.**
  - Arrow keys, Page Up/Down and the mouse move the caret across section boundaries, which
    activates the next document.
  - A selection stays inside one section.
  - Select All selects the current section; `binder.selectAll` selects items for binder
    operations.
- **Undo.** Each section keeps its own undo history, the same as separate documents today. Binder
    operations (move, split, merge, delete) have their own undo in the project.
- **Project-wide commands.** Find and Replace, word count and spelling run section by section
  through `with_document`, opening and closing sections as needed. The results list names the
  section.
- **Styles.**
  - Every section starts from the preset's template.
  - `project.updateStyles` runs the existing `tools.updateStyles` on each section, so a style
    change reaches every chapter.
  - Compile applies the template's styles regardless.

### What isn't live in the stack

Some things only make sense across the whole document, so they are exact only in the compiled
result:
- **Figure and table numbers.** The stack can show numbers close to the final ones by starting
  each section's caption count after the counts cached for earlier sections. The caption update
  (`update_seq`, `crates/engine/src/cmd/references/fields.rs`) walks one document and would need
  a starting count per sequence. That is a later step; until then, each section counts from 1.
- **Cross-references to other sections** show their last result (see
  [Cross-references between sections](#cross-references-between-sections)).
- **Table of contents and bibliography** exist only in the compiled document, or in a front-matter
  section that compile fills.

## Project format

```
thesis.wcproj/
  project.toml          # binder, metadata, targets, presets (the only file WordCraft must parse to list the project)
  sections/
    s0001.docx
    s0002.docx
  research/             # anything; shown in the binder's Research folder
  snapshots/
    s0002/
      20261107T143000Z.docx
      20261107T143000Z.toml   # label, word count
  templates/
    university.dotx     # optional; a preset may also name a template outside the project
  .wcproj-cache.toml    # page and word counts, reference index; safe to delete
  .wcproj-lock          # who has the project open (host, pid, time); advisory
```

### `project.toml`

```toml
format = 1                    # project format version; newer files open read-only with a message
title = "Ink and Iron: Metallurgy in Early Print"
author = ""                   # left blank by default; never guessed from git or the OS
next_id = 7

[targets]
project_words = 80000
deadline = "2027-03-31"      # parsed with chrono
session_words = 1000

[[item]]
id = "s0001"
kind = "folder"
title = "Front matter"
include = true
numbering = { zone = "front" }

  [[item.children]]
  id = "s0002"
  kind = "section"
  title = "Abstract"
  role = "abstract"
  synopsis = "300 words on the question, method and finding."
  label = "front"
  status = "draft"            # free text; the UI offers todo / draft / revised / final
  target_words = 300
  include = true

[[preset]]
name = "University submission"
template = "templates/university.dotx"
output = "pdf"                 # pdf | docx | odt | latex | html | markdown
title_page = "s0003"           # an item to place first, or omitted
separator = "section-break"    # between top-level folders: section-break | page-break | none
section_separator = "none"     # between sections inside a folder
heading_from_title = false     # true: insert each item's title as a heading at its level
include_labels = []            # empty = every included item
toc = true
caption_numbering = "per-chapter"  # continuous (Figure 14) | per-chapter (Figure 3.2)
normalize_figures = "keep"     # keep | inline | top-bottom (see Figures)
final = false                  # true: compile.check treats comments and tracked changes as errors
```

Why TOML and ids:
- `project.toml` is readable and diffable, so a project kept in git shows clean changes when the
  binder is reordered.
- Section files are named by id, so renaming an item never moves a file, and a broken
  `project.toml` still leaves every section intact and openable.
- `toml` is already in `Cargo.lock`. Its licence must be confirmed against the allowlist before the
  new crate depends on it directly.

### Caches

- `.wcproj-cache.toml` holds, per section:
  - its file's modification time and size;
  - page count under the stack's layout, and word count;
  - caption counts per sequence;
  - the headings, captions and bookmarks it holds (the reference index).
- It speeds up the stack's scroll bar, the binder's totals and `insert.crossReference`. It is never
  needed for correctness: an entry whose file has changed is recomputed, and a missing or damaged
  cache is rebuilt.

### Reading `project.toml` safely

The file is hostile input like any other:
- The total file size is capped (for example 4 MiB), along with the item count (for example
  10,000) and nesting depth (for example 32). Items beyond a cap are reported, not loaded.
- Ids must match `^[a-z0-9]{1,32}$`. Paths in `template` and `title_page` must stay inside the
  project folder, or be absolute paths the user chose. `..` is refused.
- Unknown keys are kept and written back, so a newer version's settings survive an older WordCraft.
- If the file can't be parsed, the project opens in **recovery mode**: every `sections/*.docx` is
  listed in a flat binder ordered by id, and the broken file is kept as
  `project.toml.broken-<time>` before anything is written.
- Saves go through the existing `safe_save` path (write to a temporary file, then rename).

## Where the code goes

| Piece | Crate | Layer | Notes |
|---|---|---|---|
| Project model, `project.toml` and cache I/O, binder operations, snapshots, compile | new `crates/manuscript` | L5 (beside `mcp`, `zotero`) | Depends on `engine`, `doc`, `chrono`, `toml`. No UI dependencies. Builds for wasm (no direct file access there; see [Web](#web-wasm)) |
| `project.*`, `binder.*`, `snapshot.*`, `compile.*` commands | `crates/manuscript` | L5 | Registered into the session's command registry the same way other crates' commands are, so the CLI, control channel and MCP get them for free |
| Stack model: which sections are open, their heights, page offsets, caret hand-over | `crates/manuscript` | L5 | No drawing; the UI asks it what to draw |
| Stacked editor, binder sidebar, corkboard, compile dialog | `crates/ui-egui` | L6 | Thin: draws each open section's existing page layout at its offset and dispatches commands |
| CLI | `apps/wordcraft-cli` | apps | `wordcraft-cli project compile thesis.wcproj --preset "University submission" -o thesis.pdf` |

`xtask/src/layers.rs` needs one new row: `("manuscript", Class::Layer(5))`. The crate root
carries the standard `#![deny(clippy::unwrap_used, …)]` attribute, and `Cargo.toml` uses
`[lints] workspace = true`.

## Commands

Everything is a command, as elsewhere in WordCraft. Programmatic calls never open dialogs.

| Id | Params | Does |
|---|---|---|
| `project.new` | `{"path", "title"?, "template"?}` | Create the folder, `project.toml` and one empty section |
| `project.open` | `{"path"}` | Load the binder; show the last stack |
| `project.close` | `{}` | Save open sections and the binder |
| `project.info` | `{}` | Binder tree, word counts, targets, open sections (pure) |
| `project.updateStyles` | `{"template"?}` | Run `tools.updateStyles` on every section |
| `project.saveAsTemplate` | `{"path"}` | Write the project's structure, presets, template and starter sections as a project template |
| `project.setMetadata` | `{"key", "value"}` | Title, author, degree…; shown on the front pages through `DOCPROPERTY` fields |
| `project.refreshReferences` | `{"ids"?}` | Write the last compile's cross-section reference results back into section files |
| `numbering.check`, `numbering.repair`, `numbering.detect` | `{"fix"?, "ids"?}` | Engine commands for any document; see [Heading and list numbering](#heading-and-list-numbering) |
| `project.find`, `project.replace` | as `edit.find` / `edit.replace` | Across every included section; results name the section |
| `binder.add` | `{"parent"?, "after"?, "kind", "title"}` | New item; a section gets an empty `.docx` made from the default preset's template |
| `binder.import` | `{"path", "parent"?, "split"?: "none"\|"heading1"\|"heading2"}` | Bring in an existing document, optionally split at headings into one section each |
| `binder.move` | `{"id", "parent", "index"}` | Reorder; refuses moving a folder into itself |
| `binder.rename`, `binder.setMeta` | `{"id", …}` | Title, synopsis, label, status, target, include |
| `binder.delete` | `{"id"}` | Moves the item to a Trash folder in the binder; its file stays until `binder.emptyTrash` |
| `binder.split` | `{}` | Split the section at the caret into two sections |
| `binder.merge` | `{"ids"}` | Join adjacent sections into the first |
| `binder.show` | `{"ids"}` or `{"folder"}` | Set the stack: one section, several, or a folder's children |
| `binder.exportForReview` | `{"id", "path"}` | See [Supervisor round trip](#supervisor-round-trip) |
| `binder.importReviewed` | `{"id", "path"}` | See [Supervisor round trip](#supervisor-round-trip) |
| `snapshot.take` | `{"id"?, "label"?}` | Copy the section's current state |
| `snapshot.list`, `snapshot.restore`, `snapshot.compare` | `{"id", "at"?}` | `compare` uses the existing `review.compare` command |
| `compile.run` | `{"preset", "path"?}` | Compile and export (see below); returns warnings |
| `compile.preview` | `{"preset"}` | Compile into a new unsaved document in the editor, so the user can inspect it before export |
| `compile.check` | `{"preset"}` | Compile in memory and return only the checks (see [Checks before submission](#checks-before-submission)) |
| `compile.presets` | `{}` | List presets (pure) |

`crates/engine/src/tests.rs::hostile_params_never_panic` must cover the new commands. If the
registry it walks doesn't include commands from L5 crates, the crate gets its own equivalent test.

## Compile

### What already exists

- **`Document::adopt`** (`crates/doc/src/edit.rs:1041`) takes another document's body as a
  fragment of this one.
  - Media, lists, styles, notes, comments and tracked changes come along. Lists, notes, comments
    and permission ranges are renumbered so they can't collide.
  - A style we already have by id stays ours. That is what compile wants: the template's styles
    win.
- **Pictures are stored once.** `add_media` (`crates/doc/src/document.rs:806`) reuses an identical
  picture already in the document, so the same figure in several sections isn't stored twice.
- **The `insert.textFromFile` flow** (`crates/engine/src/cmd/insert.rs:134`) already does: open,
  adopt, unlink the inserted file's sections, insert the fragment.
  - Unlinking (`crates/engine/src/cmd/paste.rs:406`) clears the inserted sections' headers,
    footers and different-first-page setting, so they continue the template's.
  - Compile does the same for each section, without a selection.
- **Captions with chapter numbers.** `references.caption` already takes `chapter` and
  `separator`. It writes `STYLEREF n \s` and `SEQ … \s n`, and `update_seq` honours both
  (`crates/engine/src/cmd/references/fields.rs:29`).
- **Breaks and generated content.**
  - Section breaks: `layout.break`.
  - Generated tables: `references.toc` / `references.updateToc`, `references.updateFigures`,
    `references.updateIndex`.
  - Field update: `references.updateFields`.
- **Template styles:** `tools.updateStyles` and `styles.organizer`.
- **Export:** `crate::io::save_path_with` (`crates/engine/src/io.rs:844`) writes DOCX, ODT, RTF,
  HTML, Markdown, LaTeX (`.tex`, `crates/formats/src/latex/`) and PDF through one call.

Without a template, the base is the first included section's styles, numbering and page
setup (its text comes with the section), so a heading list defined in the sections' heading
styles numbers on through every chapter.

### What compile has to add

`adopt` was built for pasting one document into another. A compiled manuscript joins dozens, so
compile has to handle the following. Each was checked in the code.

1. **Bookmarks aren't renamed.** `adopt` renumbers notes and comments but copies `BookmarkStart`
   and `BookmarkEnd` names unchanged. If two sections both have `_Ref123` (Word makes these for
   cross-references), the compiled document has duplicates.
2. **Bibliography sources aren't merged.** `adopt` doesn't touch `Document::sources`
   (`crates/doc/src/lib.rs:115`), which `CITATION` fields and `references.bibliography` use.
   - Compile must combine every section's sources by `tag`.
   - When two sections have the same tag with different content, compile keeps the first and
     warns.
3. **Headings numbered directly restart in each chapter.**
   - Heading numbering that comes from the heading **style** is fine. The template's
     `Heading 1` stays ours, numbering included, so chapters count 1, 2, 3.
   - But `adopt` gives each document's lists new ids. Numbering applied directly to the heading
     paragraphs, as the numbering button does, becomes a separate list in every section, and each
     chapter starts again at 1.
   - Compile maps the direct numbering of heading-styled paragraphs to the list the template's
     heading style uses.
   - Ordinary numbered lists in the body keep their own list, so a list in chapter 3 doesn't
     continue one from chapter 2.
4. **Section breaks inside a section file keep their page setup.** Unlinking clears headers and
   footers but keeps margins, paper size and page numbering.
   - A section file with no section break of its own carries no page setup, and follows the
     template.
   - A section file with internal breaks (a landscape table page, say) keeps them by default,
     which is usually intended. Compile reports each one as a note, so a stray break with the
     wrong margins is visible.
   - `section_setup = "template"` in the preset resets their margins, paper size and header
     distances to the template's, keeping only orientation. Use it when sections come from
     documents with stray layouts.
5. **Captions numbered continuously.**
   - A section whose captions have no chapter part (`SEQ Figure \* ARABIC`) comes out as
     Figure 14, not Figure 3.2.
   - With `caption_numbering = "per-chapter"`, compile rewrites such captions in memory to the
     form `references.caption` writes with `chapter = 1`. This happens before the field update,
     and the source files are unchanged.

### The pipeline

```
load preset
  └─ template → new Document (styles, numbering, page setup, headers/footers)
for each included item in binder order (depth-first):
  ├─ open sections/<id>.docx            (error → warning, skip; compile continues)
  ├─ rename its bookmarks:  name → name, or <id>_name if the name is already taken
  │    and rewrite REF/PAGEREF/NOTEREF fields inside the same section to match
  ├─ collect its sources into the combined list (by tag)
  ├─ optional: per-chapter caption rewrite, figure normalisation (see Figures)
  ├─ adopt into the compiled document    (template styles win)
  ├─ map directly numbered headings to the template's heading list
  ├─ unlink its sections (as insert.textFromFile does)
  ├─ optional heading from the item's title
  └─ separator (section break / page break / none) per preset
resolve cross-section references (below)
insert TOC if the preset asks and the template has none
update fields: captions/SEQ and STYLEREF, TOC, table of figures, index, cross-references, bibliography
run the checks (warnings; errors only for a preset marked final)
export to the preset's format, or open in the editor (compile.preview)
```

Compile never changes a section file. It builds a new `Document` in memory, so it can't damage
the project. A failure partway leaves nothing written, because `save_path_with` already writes
safely.

### Cross-references between sections

In Word, a cross-reference is a `REF` field pointing at a bookmark, and both must be in one file.
If chapter 3 refers to a figure in chapter 2, no section file can hold both ends.

- **Project-wide reference names.**
  - With a project open, `insert.crossReference` also lists captions and headings in other
    sections, from the reference index in the cache.
  - Picking one inserts a normal `REF` field whose bookmark name carries a project-scoped prefix,
    `_wcp_<itemid>_<name>`. The target section gets a bookmark of the same name.
- **In the stack**, the field shows its last result, its cached text.
- **In the compiled document**, both ends exist and the field updates normally. Compile's
  bookmark renaming leaves `_wcp_` names alone, because they are already unique per project.
- **Missing targets:** compile reports every `REF` whose bookmark doesn't exist in the compiled
  document as a warning naming the section.
- **Opened alone in Word**, an update of fields (including the one Word runs before printing)
  would replace such a field's text with Word's "reference not found" error. So:
  - **Every save of a section locks its `_wcp_` fields** (`w:fldLock`, which WordCraft already
    reads and writes, `crates/docx/src/write/story/field.rs:94`). A section file opened directly
    in Word keeps its cached text ("Figure 2.4") through Update Fields and printing.
  - WordCraft itself never updates a `_wcp_` field whose target isn't in the same document.
  - **Compile** unlocks them in memory before updating fields.
  - **`project.refreshReferences`** puts the fresh results from the last compile back into the
    section files, so the cached text in each file is current. Sections open in the editor are
    updated there instead, as an undoable edit. Compile itself still never writes a section file.

### Numbering across sections

- **Captions (`SEQ`) and heading numbers** are recalculated over the whole compiled document,
  continuously or per chapter as the preset says (above).
- **Footnotes and endnotes** are already renumbered by `adopt`. Restarts per chapter come from the
  template's section properties.
- **Page numbering** (cover not counted, roman front matter, arabic from chapter 1, skipped
  pages) is set per binder item. See
  [Structure, page numbering and templates](#structure-page-numbering-and-templates).

### Citations

- **WordCraft citations** (`CITATION` fields plus `Document::sources`) work once the sources are
  combined. `references.bibliography` then renders one bibliography for the whole compiled
  document.
- **Zotero citations** survive `adopt` unchanged. Each field carries its CSL JSON, the citation
  style language's data format (`ADDIN ZOTERO_ITEM CSL_CITATION {…}`,
  `crates/zotero/src/fields.rs`).
  - Their numbering and the bibliography are produced by the Zotero desktop app
    (`ui.zotero.refresh`), which runs only in the app with Zotero open.
  - `compile.preview` in the app can run a Zotero refresh on the compiled document before export.
  - Headless compile (CLI, MCP) keeps each citation's last rendered text and warns that the
    bibliography wasn't refreshed. Numeric styles (IEEE, Vancouver) therefore need the in-app path.
- **Rendering Zotero citations without Zotero** is possible in principle, since every field carries
  its item data. It needs a CSL processor and the citation style files, and it is a separate
  project with licensing questions:
  - The Rust processor `citeproc-rs` is MPL-2.0, which isn't on the allowlist. `hayagriva`
    (MIT/Apache-2.0) supports CSL and would need evaluating.
  - The official style repository's licence must be checked against the allowlist. As far as I
    know it is CC-BY-SA-3.0, while the allowlist has only CC-BY-SA-4.0. If so, it needs a
    maintainer decision before any styles are bundled.
  - Assigning citation numbers in order of first appearance is the easy part. Formatting entries
    in a style is the hard part.

## Structure, page numbering and templates

A thesis has fixed parts in a fixed order, and university rules about how each part is numbered.
Writers shouldn't have to build that from section breaks by hand. The binder holds it as data,
and compile turns it into section properties.

### Roles: placing the front page, introduction, contents and index

Every item has a `role`. Roles tell compile what an item is, and templates use them for the
default order. Placement is simply the item's position in the binder: dragging the Index item
above the Appendices folder puts the index there.

| Role | Item holds | Compile does |
|---|---|---|
| `cover`, `title`, `declaration`, `abstract`, `acknowledgements`, `dedication` | A section the writer edits (the cover and title page usually come from the template) | Places it; applies its numbering rule |
| `contents`, `figures`, `tables`, `equations`, `bibliography`, `index` | No text of its own: a heading plus the generated content | Inserts the generated content at that position: `references.toc`, `references.tableOfFigures` (per caption label), `references.bibliography`, `references.index` |
| `introduction`, `chapter`, `conclusion` | A section | Starts the main numbering if it is the first of these |
| `appendix` | A section or folder | Applies the appendix numbering rule (below) |
| `other` (default) | A section | Places it |

- **Generated items can be repeated and customised.** For example:
  - a contents list showing only levels 1–2 in front matter, plus a mini contents list at the
    start of each part;
  - a list of equations, which is a table of figures for the Equation caption label.
- **Their options** are the existing commands' parameters, stored on the item (`[item.generate]`).

### Page numbering zones

Numbering is set per item, and inherited by its children unless they set their own.

```toml
[item.numbering]
zone = "front"        # continue (default) | front | main | appendix | none
counted = true        # false: the item's pages aren't counted (later pages number as if it weren't there)
show = "all"          # all | not-first (number hidden on the item's first page) | none
start_on = "next"     # next | odd (start on a right-hand page, a blank page added if needed) | even | continue
```

Each zone sets a format and a starting point. A template sets the formats; these are the
defaults:

| Zone | Format | Starts at |
|---|---|---|
| `front` | i, ii, iii | i at the first item of the zone |
| `main` | 1, 2, 3 | 1 at the first item of the zone |
| `appendix` | continue arabic, or A-1 (chapter-prefixed) as the template says | — |
| `none` | no number shown, pages not counted | — |

**A typical thesis then needs no settings at all, because the template supplies them:**

| Item | Rule | Pages show |
|---|---|---|
| Cover | `zone = "none"` | nothing; not counted |
| Title page | `zone = "front"`, `show = "none"` | nothing, but it is page i |
| Declaration, Abstract, Contents | `front` | ii, iii, iv … |
| Introduction | `main`, `start_on = "odd"` | 1, 2, 3 … |
| Chapters | continue, `start_on = "odd"` | … |
| A fold-out map inserted mid-chapter | `counted = false`, `show = "none"` | nothing; the next page continues the count |
| Appendices | `appendix` | A-1, A-2 … or continuing arabic |

**How compile does it.** Every rule maps to a section property the engine already has, in
`SectionProps` (`crates/doc/src/section.rs`):
- format: `page_num_format`, which includes the roman formats;
- restart: `page_num_start`;
- hidden number on the first page: `title_page`;
- start on a right-hand page: `start` = `SectionStart::OddPage`, which layout already handles
  (`crates/layout/src/tests.rs`);
- chapter-prefixed numbers: `chapter_style`, `chapter_sep`.

So the compiled document is an ordinary Word document with ordinary section breaks, and opens
the same way in Word.

**Uncounted pages need one extra step.** Word can only restart numbering, not skip pages. So:
1. Compile lays the document out once.
2. It reads how many pages each uncounted item takes.
3. It sets the following section's `page_num_start` so the count carries on as if those pages
   weren't there.
4. It lays out again.

A second check confirms the numbers settled. Layout is deterministic, so this ends after the
second pass. If page counts still change (a contents list growing by a page, say), compile
repeats, with a cap of a few passes, and warns if they never settle.

**In the stack and the binder** the same rules give each item's page range from the cached page
counts (`Title i`, `Abstract ii–iii`, `Chapter 1  1–23`). Writers see the numbering before
compiling, and a wrong rule shows up at once.

### Project templates

A project template is a folder (or a `.zip` of one) that `project.new` copies from:

```
thesis-generic.wcproj-template/
  template.toml        # name, description, licence of its text, metadata fields it asks for
  project.toml         # binder with roles, numbering zones, generated items, compile presets, checks
  templates/main.dotx  # styles, page setup, headers and footers
  sections/            # starter sections: cover, title page, declaration with placeholder text
```

- **Metadata fills the front pages.**
  - `[metadata]` in `project.toml` holds title, subtitle, author, degree, department,
    supervisor, date and any extra keys the template declares.
  - The cover and title page show them through `DOCPROPERTY` fields, which WordCraft already
    supports (`crates/engine/src/cmd/references/properties.rs`). Compile copies the metadata into
    the compiled document's properties.
  - Changing the title in one place updates every page that shows it.
- **Checks travel with the template.** A university's rules (word limit, minimum font size,
  margins, line spacing, which parts are required) are entries under `[[check]]`, and
  `compile.check` reports them.
- **Making your own.**
  - `project.saveAsTemplate {"path"}` writes the current project's structure, presets, template
    and the sections marked as starters, without the rest of the text.
  - Users adjust a generic template once for their university or company, and reuse it for
    every report.
- **Shipped templates** are WordCraft's own text and colours, as for document templates: Thesis,
  Report, Book, Journal article. No university or publisher names or logos.

### Saving and exporting parts separately: export sets

A preset can produce several files at once, and choose which items go in. One project can then
have:
- "Full thesis (PDF)";
- "Front pages only (PDF, for signing)";
- "Each chapter as DOCX (for the supervisor)";
- "Chapters 3–4 as a journal article (LaTeX)".

```toml
[[preset]]
name = "Chapters for review"
include_roles = ["introduction", "chapter", "conclusion"]   # or include_ids / include_labels
split = "per-item"           # none (one file) | per-item | per-top-folder
file_name = "{n:02}-{title}" # n = position in the output; title from the binder
numbers = "as-in-full"       # as-in-full | standalone
output = "docx"
```

- **`numbers = "as-in-full"`.** Each part keeps the page, figure and chapter numbers it has in
  the full document: chapter 3 exported alone starts at page 41, with Figure 3.1. That is what a
  supervisor or examiner expects.
  - For PDF, compile builds the full document and writes only that part's pages. The print
    command already takes page ranges; PDF export would get the same `pages` option.
  - For DOCX, compile sets the part's `page_num_start` and starts its caption counts at their
    values in the full document. The caption counter needs the per-sequence starting count also
    used by the stack.
- **`numbers = "standalone"`.** Each part numbers from 1 as a document of its own (the journal
  case).
- **Saving a part separately while writing** needs no export: every section is already its own
  file. `binder.exportForReview` gives it a readable name and makes it safe to open in Word.

## Figures

Floating pictures that move when text changes are the most common complaint about long Word
documents. The cause is that a floating picture is attached to a paragraph, its anchor:
- when that paragraph moves, the picture moves with it;
- dragging the picture attaches it to whichever paragraph is nearest the drop point;
- text wrapping around the picture can push its own anchor paragraph onto the next page, taking
  the picture along.

This design deals with it at four levels.

1. **Contain it.** Each chapter is its own document starting on a new page. However a picture
   behaves, only its own chapter's pages reflow. Nothing in later chapters moves, in the stack or
   in the compiled document.
2. **Default to figures that can't jump.**
   - `binder.add`'s section template and the Insert Figure command use an inline picture in its
     own centred paragraph, set to Keep with Next, followed by a caption.
   - An inline picture behaves like a large character in the text, so it can't drift.
3. **Placement presets for figures that must float** (Picture Format › Position):
   - **Top of page:** vertical position relative to the margin, aligned top; Top and Bottom
     wrapping; anchor locked.
   - **Bottom of page:** the same, aligned bottom.
   - Top and Bottom wrapping never squeezes text into a narrow column beside the figure. A
     margin-aligned position doesn't change when text above the anchor grows. The figure moves
     only when its anchor paragraph moves to another page, and it lands at the same place there.
   - These are standard DrawingML settings, so Word shows them the same way.
4. **Normalise at compile.** `normalize_figures` in the preset:
   - `keep`: leave figures as they are, and warn about each floating picture anchored to a
     paragraph.
   - `inline`: convert floating pictures with Square, Tight or Through wrapping to inline figures
     in their own centred paragraph, set to Keep with Next.
   - `top-bottom`: convert them to the Top of page preset.
   - Only the in-memory compiled document changes, never the section files.

### Engine fixes this depends on

These were checked in the code. They are useful on their own, outside projects, and should land
first (milestone 0).

- **Lock Anchor isn't modelled.**
  - `Float` (`crates/doc/src/para/float.rs:136`) has no field for it.
  - The DOCX reader ignores `wp:anchor/@locked`, and the writer always writes `locked="0"`
    (`crates/docx/src/write/story/drawing.rs:259`), so a Word file's locked anchors are lost when
    it is saved.
  - The fix: add the field, read and write it, add Lock Anchor to the Layout Options menu, and
    when dragging a locked picture change only its offsets, keeping its anchor paragraph. As in
    Word, an unlocked picture is still re-anchored on drag. That is needed when it is dragged to
    another page.
- **Dragging resets "Fix Position on Page".**
  - After placing a dragged picture, `move_object_once` always sets its position relative to the
    column and paragraph (`crates/engine/src/cmd/objects/arrange.rs:201-203`).
  - So a picture set to Fix Position on Page becomes Move with Text after any drag.
  - Word is reported to keep Fix Position on Page after a drag, recalculating the offset in the
    same frame (`page` or `margin`). That still has to be observed directly, following the clean
    room rules, before the fix lands.
  - The fix: a drag keeps the float's existing `h_rel` and `v_rel` and computes the new `x` and
    `y` in that frame. It changes the anchor paragraph only when the picture isn't locked, and
    draws the anchor marker while dragging.
- **Captions don't stay with their pictures.**
  - `references.caption` doesn't set Keep with Next on the picture's paragraph (or Keep with Next
    on the caption, for a caption above).
  - On a floating picture, it inserts an ordinary paragraph beside the anchor paragraph. That
    paragraph can end up on a different page from the picture.
  - Proposed: set Keep with Next on the paragraph that comes first. For a floating picture with
    Square, Tight or Through wrapping, offer to make it inline first. Grouping a floating picture
    and a caption text box is possible but leaves the caption outside the table of figures in some
    readers, so it isn't the default.

## Heading and list numbering

Numbering is the other long-standing complaint about long Word documents. Numbers like 3, 3.2
and 3.2.1 go wrong in several ways, and Word mostly doesn't say so:

- **Typed numbers aren't tracked.** A heading typed as "3.2.1 Methods" is plain text. Insert a
  section before it, or move it, and the number is simply wrong. Documents imported from PDF,
  Google Docs, older Word files or other people's templates often have numbers typed like this.
- **Real numbering silently splits into several lists.** Pasting a heading from another document,
  or applying numbering with the button instead of through the heading style, gives that
  paragraph a different list. It then restarts at 1, or continues the wrong sequence
  (2.1 after 3.4).
- **Restarts and "Set Numbering Value" stay hidden.** A restart set by mistake months ago is
  invisible until the numbers look wrong.
- **Skipped levels.** A Heading 3 directly under a Heading 1 shows as 1.0.1.
- **Double numbers.** A heading with both list numbering and a typed number shows "3.2 3.2
  Methods".
- **References go stale.** Typed "see Section 3.2" never changes. A cross-reference field changes
  only when fields are updated.
- **Separate files can't count on.** A chapter kept in its own file starts at 1 when opened alone.

### What WordCraft already has

Checked in the code:
- **Heading-linked multilevel lists.** `list.multilevelPreset` with `headings` (1 / 1.1 / 1.1.1),
  `article`, `outlineHeadings` and `chapter` links levels 1–9 to Heading 1–9. A new heading is
  numbered by its style alone (`docs/typing-parity.md`, tests in
  `crates/engine/src/tests/typing.rs`).
- **List controls:** `para.restartNumbering`, `para.setNumberingValue`, `para.listLevel`,
  `para.defineNumber`, and legal-style numbering (`legal` in `crates/doc/src/numbering.rs`).
- **Per-level start overrides on a list** (`Num::start_overrides`): one list can start its first
  level at 3 without a second list.
- **Cross-references** to headings, figures, tables and equations, showing text, page or number
  (`insert.crossReference`, `show: "number"`).
- **Outline commands** to promote, demote and move headings with their content (`outline.*`).
- **Not there yet:** nothing detects typed numbers, and nothing reports the problems above. The
  AutoFormat rule for `1. ` at the start of a paragraph works only while typing.

### The rule: one scheme, numbers come from levels

- **A project has one numbering scheme**, from its template. It is a heading-linked multilevel
  list: level formats (`1`, `1.1`, `1.1.1`, or `Chapter 1`, `A.1` for appendices), linked to
  Heading 1–9.
- **A heading's number comes only from its level and position.** Writers never type numbers; they
  choose a level (Heading 2, or promote/demote with the existing outline keys), and the number
  follows. Moving a section in the binder, or a heading in outline view, renumbers everything.
- **Appendices switch format, not lists.** The `appendix` numbering zone (see
  [Page numbering zones](#page-numbering-zones)) switches level 1 to letters in the compiled
  document, so Appendix A, A.1, A.2 follow without a second list.
- **Body lists stay separate.** Numbered lists in the text (1, 2, 3 / a, b, c) are their own lists
  and never continue across sections.

### Numbering check and repair

`numbering.check` lists every problem with its location, in a task pane like the spelling pane.
`numbering.repair` fixes one item or all of them, as one undo step each, and reports exactly what
changed. Both work on any document, not only in projects, so they belong in the engine
(`crates/engine/src/cmd/lists.rs` or a new `cmd/numbering.rs`).

| Problem found | Repair offered |
|---|---|
| A heading on a different list from the scheme (pasted or button-numbered) | Move it to the scheme's list at its level |
| A restart or numbering value on a heading | Remove it, or keep it (marked as intended, so it isn't reported again) |
| A skipped level (Heading 3 under Heading 1, shown 1.0.1) | Promote it, or insert the missing level, or keep it |
| A typed number in front of a numbered heading ("3.2 3.2 Methods") | Delete the typed number |
| A heading with a typed number and no list numbering | Convert (see [Detecting typed numbers](#detecting-typed-numbers)) |
| Typed references ("see Section 3.2", "Figure 4.1") matching a heading or caption | Turn into a cross-reference field to it |
| A cross-reference field whose shown number differs from its target's current number | Update it |
| A cross-reference to a heading that no longer exists | Report only |

"Marked as intended" is stored as a document variable, so it survives saving and opening in
Word.

### Detecting typed numbers

`numbering.detect` finds paragraphs that start with a typed number and proposes real numbering.
It is offered automatically when a document is opened or imported into a project
(`binder.import`), and never applied without the writer confirming.

1. **Patterns** at the start of a paragraph:
   - decimal `3`, `3.2`, `3.2.1` (with or without a final full stop or bracket);
   - `Chapter 3`, `Section 3.2`, `Part II`, `Appendix A`, `A.1`;
   - roman `I.`, `II.`, letters `A.`, `(a)`, `a)`.
   - Patterns are a fixed list, and each one is matched once per paragraph start, over a bounded
     number of characters.
2. **Level** comes from the number of parts: `3.2.1` is level 3.
3. **Confidence** comes from three things:
   - **Sequence:** the numbers found form a sequence (3.1 after 3, 3.2 after 3.1). A jump or
     repeat lowers confidence and is shown, since it is usually a numbering mistake in the
     source.
   - **Looks like a heading:** short, one line, bold or larger, followed by body text, or already
     in a heading style.
   - **Not a list or a table:** numbers in table cells, and runs of short numbered lines in body
     text, are treated as body lists, not headings.
4. **Proposal:** a table of every candidate (number, text, proposed level, confidence) where the
   writer can untick lines or change a level.
5. **Convert:**
   - remove the typed number and the space after it;
   - apply Heading *n* (or the scheme's style for that level);
   - put the paragraph on the scheme's list.
   - If the first number isn't 1, as in a chapter file starting at "3.1", set a start override so
     the numbers stay what they were (see the next section).
   - Typed references to the converted headings are then offered as cross-references (the
     check's last rows).
6. **Body lists** found the same way (`1)`, `a.`, `(i)` at the start of consecutive paragraphs)
   are offered as real numbered lists.

### Numbers across separate section files

Each section is its own document, so chapter 3's file has to know it is chapter 3:

- **In the stack and the binder**, heading numbers start after the counts cached for earlier
  sections (heading counts per level are in the cache, like caption counts).
- **In the section file**, `binder.move`, `binder.add` and `binder.delete` set the scheme list's
  level-1 start override (`Num::start_overrides`) in each affected section. That is one small
  change to each file whose position changed, done after taking care of open sections like any
  other edit.
  - So a supervisor opening `s0005.docx` in Word also sees 3, 3.1, 3.2.
  - This is the only time WordCraft changes a section file for the writer. It can be turned off
    per project (`sync_numbering = false`).
- **In the compiled document**, compile drops those start overrides and maps every heading to the
  template's list, so numbering simply runs through.
- **Cross-chapter references** to heading numbers work like the other cross-section references
  (project-scoped bookmarks, locked in section files; see
  [Cross-references between sections](#cross-references-between-sections)).

### Adding numbering fast

- **Typing a number creates a numbered heading** (projects only, off in Word-parity mode): `3.2 `
  followed by a space at the start of an empty paragraph applies the scheme's level 2 heading.
  The typed number is removed, and the paragraph shows the number its position gives. If that
  number isn't what was typed, the status bar shows the actual number. This extends the existing
  `1. ` AutoFormat rule and is pinned in `crates/engine/src/tests/typing.rs`.
- **Level keys:** the existing promote and demote keys (`outline.promote`, `outline.demote`) work
  on headings outside outline view too.
- **The binder shows numbers.** Items show their heading numbers from the cache (`3  Methods`,
  `3.2  Sampling`), so reordering in the binder shows the new numbers at once.

## Supervisor round trip

Section files open in Word, so a supervisor can review a chapter directly. Two commands make that
safe and simple:

- **`binder.exportForReview {"id", "path"}`**
  1. Takes a snapshot labelled "Sent for review".
  2. Writes a copy with a readable name (for example `04-Results.docx`, from the binder position
     and title).
  3. Runs `project.refreshReferences` for that section first, so its locked cross-section
     references show current numbers.
- **`binder.importReviewed {"id", "path"}`**
  1. Takes a snapshot of the current section.
  2. Opens the reviewed file. Its `_wcp_` fields are still locked, as in every section file.
  3. Compares it with the snapshot taken at export (`review.compare`). The supervisor's tracked
     changes and comments, and any untracked edits as tracked differences, show in the section
     for the writer to accept or reject.
  4. If the writer also edited the section since export, the writer's version is kept and the
     review is shown as a comparison against the export snapshot. Nothing is overwritten silently.

## Checks before submission

`compile.check`, and every compile, reports:
- cross-section references whose target is missing;
- captions that nothing refers to ("Figure 4.3 is never referenced in the text");
- raster pictures below a resolution threshold at their placed size (default 300 dpi, set in the
  preset);
- floating pictures, when `normalize_figures = "keep"`;
- leftover comments, tracked changes and placeholder markers (`TODO`, `[CITE]`; the list is set
  in the preset). They are errors when the preset is marked `final`;
- sources with the same tag and different content, and Zotero citations not refreshed;
- section breaks inside section files (a note, not a warning).

Each result names the section and, where it can, the position, so the binder can open it.

## Snapshots and targets

- **Snapshots.**
  - `snapshot.take` copies the section's saved bytes to `snapshots/<id>/<UTC time>.docx`, with a
    small `.toml` beside it for the label and word count. Times go through `chrono`.
  - Snapshots are taken automatically before `binder.split`, `binder.merge`, `snapshot.restore`
    and `binder.importReviewed`.
  - Restoring takes a snapshot of the current state first, so a restore can be undone.
- **Word counts.**
  - They reuse `review.wordCount`'s counting on each section.
  - The cache holds counts by file modification time, so a large project doesn't open every
    section to show totals.
- **Session target.** Words written since the project was opened, counted as the change in the
  total. Deleting text can make it negative; it is shown as is.

## User interface (L6)

- **Stacked editor** in the main area (described above). With a single section selected, it is
  the ordinary editor.
- **Binder sidebar** on the left.
  - It replaces the Navigation Pane while a project is open; the Navigation Pane stays available
    as a toggle.
  - It shows the tree, word counts against targets, and status colours.
  - Selecting a folder stacks its children. Dragging an item runs `binder.move`.
- **Corkboard.** A grid of cards for a folder's children, showing title, synopsis, label and
  status. Dragging a card reorders it. The cards are drawn in code with `theme::Tokens` colours,
  using no bitmap assets.
- **Inspector** on the right for the selected item: synopsis, notes, label, status, target and
  snapshots.
- **Compile dialog.** Pick a preset, edit its options, then Check, Preview or Export.
- **Project tab** in the ribbon for these commands.
  - Its catalog entries are WordCraft's own, outside Word parity, so they don't affect
    `cargo xtask parity`.
  - It positions projects as a replacement for Word's Master Documents (Outline › Show Document).

UI work follows the house rule of looking at the result: `ui_shot` screenshots of each view
before merging.

## Web (wasm)

- The web build has no direct file system.
- A project can be opened there as a `.zip` of the `.wcproj` folder (the zip crate is already in
  the lock file). It is edited in memory and downloaded again as a zip, with compile producing a
  download.
- This is milestone 4. Milestones 1–3 must still **build** for wasm (`cargo xtask wasm`), with the
  file-system commands returning a clear "not available in the browser" error.

## Testing

- **Unit:**
  - `project.toml` and cache round-trips, unknown keys kept, every cap enforced, recovery mode on
    a broken file.
  - Bookmark renaming, the sources merge with conflicting tags, heading-list mapping,
    per-chapter caption rewriting, figure normalisation.
- **Compile fixtures (synthetic, made in tests):**
  - Three sections with figures and a cross-reference between sections. Footnotes in two
    sections. One `CITATION` per section. Headings numbered by style in one section and directly
    in another. A TOC.
  - Assert:
    - figure numbers run 1–N (continuous) or 1.1, 2.1… (per chapter);
    - the cross-reference resolves to the right number;
    - footnotes run 1–N;
    - chapters number 1, 2, 3;
    - there is one bibliography with every source;
    - the TOC lists every heading;
    - the source files are byte-identical before and after.
- **Stack:**
  - Moving the caret across section boundaries.
  - Sections opened and closed while scrolling, with no lost edits (a changed section is saved
    before it is closed).
  - The cap on open sections holds.
  - Page offsets come out right after an earlier section grows by a page.
- **Page numbering:**
  - The typical thesis table above, compiled: assert the number shown on every page.
  - An uncounted fold-out page mid-chapter.
  - Odd-page chapter starts adding blank pages.
  - A contents list that grows by a page between passes.
  - Each numbering rule written to DOCX and read back.
- **Export sets:** `per-item` output names. A chapter exported `as-in-full` starts at its page
  and figure numbers in the full document.
- **Numbering:**
  - Each problem in the check's table: found, repaired, and left alone once marked as intended.
  - Detection: a document of typed headings (3, 3.1, 3.1.1, a jump to 3.3, a numbered table and a
    numbered body list) is converted with the right levels; the table and list aren't headings;
    a chapter file starting at 3.1 keeps its numbers.
  - Hostile: 100,000 paragraphs starting with digits; numbers with thousands of parts.
  - Start sync: moving a chapter in the binder renumbers the files after it, and the compiled
    document numbers continuously.
- **Figures:** Lock Anchor round-trips through DOCX. A locked picture keeps its anchor when
  dragged. A Fix Position on Page picture stays page-relative after a drag. Insert Caption sets
  Keep with Next.
- **Hostile input:**
  - Every new command with junk params.
  - A project folder whose `project.toml` points at `../../etc/passwd`, nests 10,000 levels deep
    or names 100,000 items.
  - A damaged cache.
- **Round trip:**
  - Compile to DOCX, reopen, and compare the document model.
  - Compile to PDF and check the page count and the text on each page.
  - Compile to LaTeX and check it parses back with the existing LaTeX reader.
- **Performance:**
  - A generated 300-page, 40-section project with 100 pictures.
  - Measured: typing latency in the stack versus the same text as one document, compile time,
    and peak memory.
  - This also measures what gap 20 in [gaps.md](gaps.md) says is unmeasured.

No file produced by Word is committed. Fixtures are built in test code.

## Milestones

Estimates are rough, for one person working on it most days, and assume the engine behaves as
described here.

| # | Scope | Estimate |
|---|---|---|
| 0 | Engine fixes from [Figures](#engine-fixes-this-depends-on): Lock Anchor, the Fix Position on Page drag reset, Keep with Next on captions (**done**; the drag behaviour is still to be confirmed against Word); top and bottom of page placement presets (**done**: Position › Figure at Top / Bottom of Page). Not yet: keeping a caption with a floating figure (see Figures) | 3–5 days |
| 0b | Numbering in the engine, for any document: `numbering.check`, `numbering.repair`, `numbering.detect` with their task pane (see [Heading and list numbering](#heading-and-list-numbering)). **Started:** `numbering.check` and `numbering.repair` for otherList, restart, skippedLevel and doubleNumber, body headings only; `numbering.detect` and `numbering.convert` for decimal and Chapter/Section numbers; to do: letters and roman numerals, typed and stale references, body lists, marking a problem as intended. The Heading Numbering pane is done | 1.5–2 weeks |
| 1 | `crates/manuscript`: project format, cache (with heading and caption counts), binder commands, numbering start sync, snapshots, CLI `project` subcommands, tests. No UI (**done**, except the cache's heading and caption counts, numbering start sync and the reference index, which come with milestones 2 and 3) | 1–1.5 weeks |
| 2 | Compile: pipeline, bookmark renaming, sources merge, heading-list mapping, per-chapter captions, figure normalisation, cross-section references, roles and generated items, page numbering zones (with the uncounted-page pass), checks, presets, fixtures (**started**: the pipeline, presets with template, separators, titles as headings and a table of contents, bookmark renaming, sources merge, heading-list repair, warnings, `compile.run`/`preview`/`presets` and the Project tab's Compile group; to do: per-chapter captions, figure normalisation, project-wide cross-references, roles and generated items, page numbering zones, `compile.check`) | 2.5–3 weeks |
| 3 | UI: stacked editor, binder sidebar, inspector, compile dialog, Project tab, targets (**started ahead of milestone 2**, at the owner's request: the Project tab and the Binder pane are done; the stacked editor, inspector, compile dialog and targets UI are still to do) | 2–3 weeks |
| 4 | Project templates and metadata, export sets (split output, `as-in-full` numbering, PDF page ranges), corkboard, supervisor round trip, live caption offsets in the stack, web zip projects, import and split of a long `.docx` at headings | 2–3 weeks |
| later | Abbreviation list (a marked-term list in front matter, with a check that each term is spelled out before its first use in binder order); Zotero rendering without Zotero, after the licence questions above | — |

The first useful release is milestones 0–3: write in sections, see them together, reorder them,
and compile a correctly numbered PDF, DOCX or LaTeX file.

## Risks

| Risk | Mitigation |
|---|---|
| Cross-section references confuse users when a section is opened alone | The field keeps its last result. The binder marks sections with unresolved references. Export for review locks the fields. Compile warnings name them |
| The stack's page numbers differ from the compiled ones | Dividers say each section starts on a new page in the stack; `compile.preview` is one click away |
| Opening and closing sections while scrolling loses an edit | A changed section is saved (through `safe_save`) before it is closed; covered by stack tests; recovery works per document as today |
| `adopt` misses another document part that collides when joining many files | The compile fixture covers every kind the doc model has: content controls, permissions, comments, revisions, text boxes, charts. Each collision found gets a test in `crates/doc` |
| Long compile times on large theses | Measure early (Testing › Performance). Compile is pure, so later sections can be read on worker threads (not on wasm) |
| Zotero numbering in headless compile | Documented limitation; the in-app preview refreshes |
| Two copies of WordCraft editing one project | Advisory `.wcproj-lock`; the second opens read-only with a message |
| Scope creep into a different product | Non-goals above; each milestone ships on its own |

## Open questions for maintainers

Each has a recommendation. The owner of this fork has chosen the first and third: an in-app
Project mode, with `.docx` section storage. For upstream WordCraft, the decision stays with its
maintainers.

1. **A mode in WordCraft, or a separate app?**
   - Recommended: a mode. A writer needs every WordCraft tool while editing a section.
   - Opening a `.wcproj` adds the Project tab, the binder and the stack, with one shell to
     maintain.
2. **In scope for upstream WordCraft, whose roadmap is Word parity?**
   - Recommended: yes, framed as a reliable replacement for Word's Master Documents. That is
     Word's own multi-file feature, and it has a long reputation for damaging documents.
   - It stays outside the parity catalog.
3. **Section storage: `.docx`, or the internal JSON model?**
   - Recommended: `.docx`, for interoperability with supervisors and co-authors.
   - For readable diffs in git, a project can set a `.gitattributes` text conversion that runs
     `wordcraft-cli` to print a section's text.
4. **Name and extension:** `.wcproj` is a placeholder.

## Revision history

| Date | Change |
|---|---|
| 2026-11-08 | Milestone 2 started: compile pipeline, presets, Compile on the Project tab |
| 2026-11-08 | Thesis mode's first visible part (Project tab, Binder pane) built before compile, at the owner's request |
| 2026-11-08 | Milestone 1: the `manuscript` crate, project format, binder commands, snapshots, CLI |
| 2026-11-07 | Milestone 0b: Heading Numbering pane |
| 2026-11-07 | Milestone 0b: typed heading numbers detected and converted |
| 2026-11-07 | Milestone 0b started: numbering check and repair |
| 2026-11-07 | Milestone 0 complete: figure placement presets added |
| 2026-11-07 | Milestone 0: Lock Anchor, the drag fix and caption Keep with Next implemented |
| 2026-11-07 | Heading and list numbering: one scheme per project, check and repair of split lists, restarts, skipped levels, double and typed numbers, and stale references; detection of typed numbers on import; numbers kept across separate section files. Cross-section reference fields locked in every section file, with `project.refreshReferences`. `section_setup` preset option. HTML and Markdown outputs. Fork owner's choices recorded for the open questions |
| 2026-11-07 | Structure, page numbering and templates: item roles (placing the cover, contents, index and others), page numbering zones (roman front matter, restarts, uncounted and hidden pages, odd-page starts) mapped to existing section properties, project templates with metadata, export sets (front pages or each chapter separately, numbers as in the full document) |
| 2026-11-07 | Stacked view of separate section documents as the core of the design (speed, containing reflow). Corrections from review: LaTeX output exists; per-chapter captions; directly numbered headings; section breaks inside section files; pictures already stored once. Added figure presets and normalisation, engine fixes found in the code (Lock Anchor, the Fix Position on Page drag reset, caption Keep with Next), supervisor round trip, checks before submission, Zotero licensing notes, recommendations for the open questions |
| 2026-11-07 | First draft |
