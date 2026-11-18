# Typography and reading hierarchy

Sidenote separates text by size, weight and colour, the way GitHub's Primer does: names
are heavier than body, body is regular, and facts about an item are smaller and greyer.
Space groups things first; lines come second. Page titles use bundled Fraunces (400) and
all other text bundled Figtree (400 and 600), on every platform, with the warm palette in
`src/ui/theme.rs`. Hindi and Telugu fall back to Nirmala UI on Windows and to bundled Noto
Sans Devanagari and Telugu on Linux. The font files and their OFL licences are in
`assets/fonts`. The roles live in `src/ui/roles.rs`. Pages use them, never raw sizes or colours.

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
| Page title | Fraunces, 26/33 | 400 | `ink` | One per page |
| `SECTION` | 14/20 | 600 | `ink` | Needs attention, Decisions, To do, a settings section, Memory and Tasks panels |
| `TITLE` | 14.5/22 | 600 | `ink` | A meeting in the list, a topic, a model card, the Ask question |
| `READ` | 14.5/23 (1.6) | 400 | `ink` | Summary points, to-dos, Ask answers, live notes |
| `SMALL` | 12.5/19 | 400 | `ink` or `soft` | Dense rows, descriptions, transcript lines |
| `NAME` | 12.5/19 | 600 | `ink` | The name in a dense row: a setting, a finding, the active sidebar place |
| `CONTROL` | 13/19 | 500 | `soft`, `ink` when active | Tabs, buttons, links, sidebar places |
| `META` | 12/18 | 400 | `faint` | Times, people, dates, sizes, licences, sources, help text |
| `GROUP` | 13/20 | 600 | `ink` or `soft` | Group labels inside a section (Yesterday, This week, Medium) and field labels |
| `CAPTION` | 12/16 | 600, capitals | `faint` | Panel captions: FINDING, ASKED ABOUT THIS MEETING |
| Count | 12/18 in a pill | 500 | `soft` on `chip` | The number after a heading or tab |

Variants: `.bold()` (600), `.medium()` (500) and `.regular()` (400) keep the size.
Warnings people must act on use `SMALL.medium()` in `warn`, not 12 px.

Glyphs (ticks, chevrons, the close cross) use `widgets::label` with their own size. Words
never do.

## The Ask thread

The answer reads at `READ` (14.5/23), the same as the summary under it, so it reads as
body text and not as a size used nowhere else. An answer is prose
people read in full, so it gets the reading size; the question above it is `TITLE`.

## Colour tokens and contrast (WCAG, checked 2026-11-10)

| Token | Light: page / card / sidebar / chip / sel / hit | Dark: page / card / sidebar / chip / sel / hit |
|---|---|---|
| `ink` | 16.1 / 16.7 / 14.9 / 14.3 / 13.9 / 15.0 | 14.1 / 12.6 / 14.9 / 11.6 / 11.7 / 10.7 |
| `soft` | 6.6 / 6.8 / 6.1 / 5.8 / 5.7 / 6.1 | 8.9 / 7.9 / 9.4 / 7.3 / 7.4 / 6.7 |
| `faint` | 5.4 / 5.6 / 5.0 / 4.8 / 4.6 / 5.0 | 7.8 / 6.9 / 8.2 / 6.4 / 6.5 / 5.9 |
| `warn` | 5.4 / 5.6 / 5.0 / 4.8 / 4.7 / 5.0 | 8.4 / 7.5 / 8.9 / 7.0 / 7.0 / 6.4 |

Every text token keeps 4.5:1 on every fill text sits on: the page, cards, the sidebar,
hover (`chip`), the selected row (`sel`) and the search highlight (`hit`).

Dev.4 raises dark captions and placeholders to #b0ada4. Kit captions use the shared
12 px `META` role. Disabled instruction saves keep `soft` text at full opacity.

Round 2 changes: dark `faint` #8f8c83 to #9a978e (was 4.3:1 on `chip`, 3.75:1 on `hit`),
dark `soft` #b0ada4 to #bcb9b0 to keep a clear step above `faint`, dark `hit` #3a3317 to
#363019, light `faint` #6e6b64 to #6b6861 and light `warn` #a35d00 to #9a5800 (was
4.35:1 on `chip`). Selected rows used the hairline colour, which left `faint` at 4.3:1;
they now use `sel`, the accent at 10% over the page (#e8ebed light, #262a2c dark).

Items 7, 8, 13 of the v3 page: the size scale lives only in `roles.rs` and `theme.rs`
(`BODY`, `BODY_LH`, `PAGE_TITLE`), so a page that uses the roles follows it. A page title
never passes `PAGE_TITLE` (26). `#77746d` is no longer used for text; `faint` is the dimmest
text grey, never below #a9a69f in dark mode. Greys carry structure; the coral accent marks the one main action; green, amber
and red mean status only. `ui::theme::type_and_contrast_tests` fails any text token under
4.5:1 on any fill it sits on, and any clickable mark under 3:1.

## Spacing (4 px grid)

- Page title to the first content: 16. Page sides: 40 wide, 16 to 20 at 560 px.
- `SECTION_TOP` 28 above a section heading, `SECTION_GAP` 12 below it.
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
- The summary: two to four sentences in `READ`. It is the `day_summary` the daily check
  writes about the day before it, in the same run; a day without one joins the first
  sentence of each meeting's own summary.
- Meetings: one 32-point link row each, the time in `META` (`faint`) in a 44-point column,
  the title in `CONTROL` `ink`, the length in `META` at the end.
- Folded sections, all closed at first, like the Tasks page groups: the Tasks chevron, a
  `GROUP` name and a count pill. Tasks created, Memories created (Memory proposals when none
  were accepted), Done, and Needs your attention for the newest check, with low findings in
  their own fold inside it. Rows inside are `SMALL` with a `meta_line`, or the compact
  finding row. An older day's check is one `SMALL` `soft` line with an Open link.

## When to use what

- **Bold (600)** marks what you scan for: headings, names, decisions, the speaker. Never
  make facts or whole paragraphs bold.
- **Smaller** text is for facts about an item (when, who, how big). Facts drop to 12 px
  so a name and its facts read as two levels at a glance.
- **Grey** text is for what you can skip on a first read: `soft` if people need to read it
  (descriptions, empty states), `faint` if they only glance at it (facts, counts,
  captions). Do not combine grey and 12 px for anything someone must act on.
- Keep accent colour for things you can press, and the selected-row tint.
