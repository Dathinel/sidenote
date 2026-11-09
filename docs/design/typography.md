# Typography and reading hierarchy

Sidenote separates text by size, weight and colour, the way GitHub's Primer does: names
are heavier than body, body is regular, and facts about an item are smaller and greyer.
Space groups things first; lines come second. We keep Sidenote's own look: Georgia page
titles, Inter for everything else, and the warm palette in `src/ui/theme.rs`. The roles
live in `src/ui/roles.rs`. Pages use them, never raw sizes or colours.

## What we took from Primer

- Three clear steps, not five near ones. A name (600), its body (400), its facts (12 px,
  grey). If two lines next to each other share size, weight and colour, merge them.
- One weight per job: 400 for reading, 500 for controls, 600 for names and headings.
- Colour roles, not colours: `ink` is `fg.default`, `soft` is `fg.muted` for text people
  read, `faint` is the scanning grey for facts. Facts are smaller and greyer, never bolder.
- Facts about one item sit on one line, joined by " · " (`roles::meta_line`).
- Space between sections is about three times the space between items in a section.

## Text roles

| Role | Size / line height | Weight | Colour | Use |
|---|---|---|---|---|
| Page title | Georgia 34/42 (28/35 at 560 px) | 400 | `ink` | One per page |
| `SECTION` | 20/28 | 600 | `ink` | Needs attention, Decisions, To do, a settings section, Memory and Tasks panels |
| `TITLE` | 16/24 | 600 | `ink` | A meeting in the list, a topic, a model card, the Ask question |
| `READ` | 16/27 | 400 | `ink` | Summary points, to-dos, Ask answers, live notes |
| `SMALL` | 14/21 | 400 | `ink` or `soft` | Dense rows, descriptions, transcript lines |
| `NAME` | 14/21 | 600 | `ink` | The name in a dense row: a setting, a finding, the active sidebar place |
| `CONTROL` | 14/20 | 500 | `soft`, `ink` when active | Tabs, buttons, links, sidebar places |
| `META` | 12/18 | 400 | `faint` | Times, people, dates, sizes, licences, sources, help text |
| `GROUP` | 13/20 | 600 | `ink` or `soft` | Group labels inside a section (Yesterday, This week, Medium) and field labels |
| `CAPTION` | 12/16 | 600, capitals | `faint` | Panel captions: FINDING, ASKED ABOUT THIS MEETING |
| Count | 12/18 in a pill | 500 | `soft` on `chip` | The number after a heading or tab |

Variants: `.bold()` (600), `.medium()` (500) and `.regular()` (400) keep the size.
Warnings people must act on use `SMALL.medium()` in `warn`, not 12 px.

Glyphs (ticks, chevrons, the close cross) use `widgets::label` with their own size. Words
never do.

## The Ask thread

The answer reads at `READ` (16/27), the same as the summary under it. It used to be 15/25,
a size used nowhere else, so it read as neither body nor small text. An answer is prose
people read in full, so it gets the reading size; the question above it is `TITLE`.

## Colour tokens and contrast (WCAG, checked 2026-11-09)

| Token | Light: page / card / sidebar / chip / sel / hit | Dark: page / card / sidebar / chip / sel / hit |
|---|---|---|
| `ink` | 16.1 / 16.7 / 14.9 / 14.3 / 13.9 / 15.0 | 14.1 / 12.6 / 14.9 / 11.6 / 11.7 / 10.7 |
| `soft` | 6.6 / 6.8 / 6.1 / 5.8 / 5.7 / 6.1 | 8.9 / 7.9 / 9.4 / 7.3 / 7.4 / 6.7 |
| `faint` | 5.4 / 5.6 / 5.0 / 4.8 / 4.6 / 5.0 | 6.0 / 5.3 / 6.3 / 4.9 / 5.0 / 4.5 |
| `warn` | 5.4 / 5.6 / 5.0 / 4.8 / 4.7 / 5.0 | 8.4 / 7.5 / 8.9 / 7.0 / 7.0 / 6.4 |

Every text token keeps 4.5:1 on every fill text sits on: the page, cards, the sidebar,
hover (`chip`), the selected row (`sel`) and the search highlight (`hit`).

Round 2 changes: dark `faint` #8f8c83 to #9a978e (was 4.3:1 on `chip`, 3.75:1 on `hit`),
dark `soft` #b0ada4 to #bcb9b0 to keep a clear step above `faint`, dark `hit` #3a3317 to
#363019, light `faint` #6e6b64 to #6b6861 and light `warn` #a35d00 to #9a5800 (was
4.35:1 on `chip`). Selected rows used the hairline colour, which left `faint` at 4.3:1;
they now use `sel`, the accent at 10% over the page (#e8ebed light, #262a2c dark).

## Spacing (4 px grid)

- Page title to the first content: 16. Page sides: 40 wide, 16 to 20 at 560 px.
- `SECTION_TOP` 40 above a section heading, `SECTION_GAP` 12 below it.
- `GROUP_TOP` 20 above a group label inside a section. Group labels are plain text, not
  shaded bars.
- `META_GAP` 2 between a name and its facts line.
- Rows in a list: no gap; each row has its own height and a hover fill.
- Hairlines only where rows hold controls that must line up (settings rows). Cards in a
  list (model cards) are separated by space alone.

## The sidebar

Three tiers, told apart by height, weight, icons and indent (`src/ui/nav.rs`):

- **Places** (Meetings, Tasks, Memory, Diary): 40-point rows with an icon, `CONTROL` in `soft`;
  the active one is `NAME` in `ink` on `sel`.
- **Projects**: a `GROUP` label in `soft`, then 32-point rows in `SMALL` (400, `soft`),
  indented 12 under the label, with no icon and no count. The six most recently met come
  first; "Show all" lists the rest for the session. A chosen project is `SMALL.medium()` in
  `ink` on `sel`, and stays listed even when it is not among the six.
- **Footer**: other devices (only when there are any), then Settings on a 36-point row,
  then Hex's status. The Diary used to sit here as "Check"; it moved up with the places
  because it is read every day, like them, not used now and then like a tool.

Routine counts live on their pages. A 7-point dot marks the two things that wait on the
user: memory proposals to review (`accent`) and open high findings in the daily check
(`warn`). The rail shows the same dots on the icon's corner.

## The Diary

One entry per day, newest first, in the reading column (760). Days with nothing are left out.

- The day: `CAPTION` in `accent` for Today or Yesterday, then the date in `SECTION`.
- The summary: one or two sentences in `READ`, counted from what the entry lists below it.
- A meeting: its time in `META` (`faint`) in a 44-point column, its title in `TITLE` (a
  link to the meeting), its facts on a `meta_line`, its context line in `SMALL` `soft`, then
  "Decided" and "For you" as 12 px 500 captions over `SMALL` rows. At 560 px the time joins
  the facts line and the column goes.
- Done, Memory and Needs your attention: `GROUP` labels with `SMALL` rows. The newest check's
  open high and medium findings use the compact finding row; lows are counted. An older
  day's check is one `SMALL` `soft` line with an Open link to its full report.

## When to use what

- **Bold (600)** marks what you scan for: headings, names, decisions, the speaker. Never
  make facts or whole paragraphs bold.
- **Smaller** text is for facts about an item (when, who, how big). Facts drop to 12 px
  so a name and its facts read as two levels at a glance.
- **Grey** text is for what you can skip on a first read: `soft` if people need to read it
  (descriptions, empty states), `faint` if they only glance at it (facts, counts,
  captions). Do not combine grey and 12 px for anything someone must act on.
- Keep accent colour for things you can press, and the selected-row tint.
