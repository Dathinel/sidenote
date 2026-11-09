# Typography and reading hierarchy

Sidenote separates text by size, weight and colour, the way GitHub's Primer does: one
semibold weight for titles, a small set of body sizes, and two greys for secondary text.
We keep Sidenote's own look: Georgia page titles, Inter for everything else, and the warm
palette in `src/ui/theme.rs`. The helpers live in `src/ui/roles.rs`.

## What we took from Primer

- Few sizes. Primer reads at 14 px with 12 px for small text and 16 to 20 px for titles.
  Sidenote reads at 16 px (it is a reading app) and uses 14, 13 and 12 below it.
- One weight per job: 400 for reading, 500 for controls, 600 for titles and headings.
- Colour roles, not colours: `fg.default` for content, `fg.muted` for supporting text. Meta
  such as times and counts is smaller and greyer, never bolder.
- Counts sit in a small pill after a heading, not as loose numbers.
- A 4 px spacing grid; space separates groups before lines do.

## Text roles

| Role | Size / line height | Weight | Colour | Use |
|---|---|---|---|---|
| Page title | Georgia 34/42 (28/35 at 560 px) | 400 | `ink` | One per page |
| Section heading | 18/26 | 600 | `ink` | Decisions, To do, settings groups |
| Title | 16/24 | 600 | `ink` | List row titles, topic titles, card titles |
| Body | 16/27 | 400 | `ink` | Summary points, to-dos, answers |
| Small | 14/21 | 400 | `ink` or `soft` | Transcript lines, form text, descriptions |
| Control | 14/21 | 500 | `soft`, `ink` when active | Tabs, buttons, menu items |
| Meta | 13/20 | 400 | `faint` | Times, people line, dates, sources, help text |
| Group label | 13/20 | 600 | `soft` | Day groups (Today, Yesterday), field labels |
| Caption | 12/18 | 600, uppercase | `faint` | Panel captions, "Discussion" |
| Count | 12/18 in a pill | 500 | `soft` on `chip` | The number after a heading or tab |

Colour tokens: `ink` is Primer's `fg.default`. `soft` is the stronger muted grey for text
people read (descriptions, empty states). `faint` is the meta grey for text people scan
(times, counts, captions). Links and actions use `accent`; status uses `ok`, `warn`, `open`.

## Contrast (WCAG, checked 2026-11-09)

| Token | Light: page / card / sidebar | Dark: page / card / sidebar |
|---|---|---|
| `ink` | 16.1 / 16.7 / 14.9 | 14.1 / 12.6 / 14.9 |
| `soft` | 6.6 / 6.8 / 6.1 | 7.8 / 6.9 / 8.2 |
| `faint` | 5.1 / 5.3 / 4.7 | 5.2 / 4.6 / 5.5 |

`faint` keeps 4.5:1 on the page, on cards and in the sidebar in both themes. It drops to
4.3:1 on the dark `chip` fill (hover and selected rows), so text that sits on a `chip` fill
uses `soft`, and the count pill does too.

## Spacing (4 px grid)

- Page title to the first content: 16. Page sides: 40 wide, 16 to 20 at 560 px.
- Between sections: 32 above a section heading, 8 below it.
- Between list rows: no gap; each row has 8 to 10 px of vertical padding and a hover fill.
- Title to its meta line: 2. Meta to a snippet: 4.
- Inside cards: 14 to 16 px padding; hairlines between rows, not around them.

## When to use what

- **Bold (600)** marks what you scan for: titles, headings, decisions, the speaker. Never
  make meta or whole paragraphs bold.
- **Smaller** text is for facts about an item (when, who, how many). Drop one step (16 to 14,
  14 to 13), not two, so the pairing still reads as one item.
- **Grey** text is for what you can skip on a first read. Use `soft` if people need to read
  it, `faint` if they only glance at it. Do not combine grey and very small for anything
  someone must act on.
- Keep accent colour for things you can press.
