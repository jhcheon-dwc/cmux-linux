---
version: alpha
name: cmux Signal
description: >-
  Interface design system for cmux on Linux/WSL2, a terminal for running several
  AI coding agents side by side. The chrome is a quiet slate frame around the
  terminals; color is reserved for agent signals.
colors:
  primary: "#E5A54B"
  secondary: "#56B6C2"
  tertiary: "#E06C75"
  neutral: "#282C34"
  success: "#98C379"
  surface-sunken: "#1B1F27"
  surface-chrome: "#21252E"
  surface-raised: "#2C313B"
  surface-hover: "#313641"
  line: "#2C313A"
  line-strong: "#3B4150"
  text-primary: "#E3E6EC"
  text-secondary: "#A2A9B6"
  focus: "#8A93A6"
typography:
  title-sm:
    fontFamily: Noto Sans CJK KR
    fontSize: 13px
    fontWeight: 700
    lineHeight: 1.3
  label-md:
    fontFamily: Noto Sans CJK KR
    fontSize: 13px
    fontWeight: 700
    lineHeight: 1.3
  body-md:
    fontFamily: Noto Sans CJK KR
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.4
  body-sm:
    fontFamily: Noto Sans CJK KR
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.3
  caption:
    fontFamily: Noto Sans CJK KR
    fontSize: 11px
    fontWeight: 400
    lineHeight: 1.3
  status:
    fontFamily: Noto Sans CJK KR
    fontSize: 11px
    fontWeight: 700
    lineHeight: 1.3
  terminal:
    fontFamily: D2Coding ligature
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.2
rounded:
  none: 0px
  sm: 4px
  md: 6px
  lg: 8px
  full: 999px
spacing:
  xxs: 2px
  xs: 4px
  sm: 6px
  md: 8px
  lg: 12px
  xl: 16px
components:
  headerbar:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    typography: "{typography.title-sm}"
    height: 38px
  icon-button:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    rounded: "{rounded.md}"
    size: 28px
  icon-button-hover:
    backgroundColor: "{colors.surface-hover}"
    textColor: "{colors.text-primary}"
    rounded: "{rounded.md}"
    size: 28px
  sidebar:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    width: 200px
  workspace-row:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    typography: "{typography.label-md}"
    rounded: "{rounded.md}"
    padding: 8px
  workspace-row-hover:
    backgroundColor: "{colors.surface-hover}"
    textColor: "{colors.text-primary}"
    typography: "{typography.label-md}"
    rounded: "{rounded.md}"
    padding: 8px
  workspace-row-selected:
    backgroundColor: "{colors.neutral}"
    textColor: "{colors.text-primary}"
    typography: "{typography.label-md}"
    rounded: "{rounded.md}"
    padding: 8px
  status-needs-input:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.primary}"
    typography: "{typography.status}"
  status-running:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.secondary}"
    typography: "{typography.status}"
  status-error:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.tertiary}"
    typography: "{typography.status}"
  status-idle:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    typography: "{typography.status}"
  status-lamp:
    backgroundColor: "{colors.primary}"
    rounded: "{rounded.full}"
    size: 7px
  attention-dot:
    backgroundColor: "{colors.primary}"
    rounded: "{rounded.full}"
    size: 8px
  tab-bar:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    height: 30px
  tab:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.text-secondary}"
    typography: "{typography.body-sm}"
    padding: 12px
  tab-active:
    backgroundColor: "{colors.neutral}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body-sm}"
    padding: 12px
  pane-focus:
    backgroundColor: "{colors.focus}"
    size: 1px
  hairline:
    backgroundColor: "{colors.line}"
    size: 1px
  input-outline:
    backgroundColor: "{colors.line-strong}"
    size: 1px
  status-done:
    backgroundColor: "{colors.surface-chrome}"
    textColor: "{colors.success}"
    typography: "{typography.status}"
  text-input:
    backgroundColor: "{colors.neutral}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body-md}"
    rounded: "{rounded.md}"
    height: 30px
  pane-gutter:
    backgroundColor: "{colors.surface-sunken}"
    size: 3px
  pane:
    backgroundColor: "{colors.neutral}"
    textColor: "{colors.text-primary}"
    typography: "{typography.terminal}"
  find-bar:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.lg}"
    padding: 4px
  menu:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body-md}"
    rounded: "{rounded.lg}"
  menu-item-hover:
    backgroundColor: "{colors.surface-hover}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
---

# cmux Signal

## Overview

cmux is where a developer runs several Claude Code (or other) agents at once, one
workspace each, and switches to whichever one needs them. The terminals are the
product; the interface around them exists to answer one question at a glance:
**which agent is waiting for me?**

So the chrome behaves like the frame of an instrument panel. It takes its tone
from the terminal surface (`#282C34`, Ghostty's default background) and steps
it down into darker slates, so terminals and chrome read as one object instead
of a gray app with blue-gray windows pasted in. Nothing in the frame is colored
for decoration. Color appears only when an agent sends a signal, and the
strongest signal, "needs input", is the one amber element on screen.

Principles:

- **Color is signal.** Hue means agent state. Selection, focus and hover use
  lightness steps of the slate scale, never a brand blue.
- **One loud thing.** Amber (`primary`) marks an agent that is blocked on the
  user. Everything else stays quiet so amber is never missed.
- **Selection joins the surface.** The selected workspace row and the active tab
  take the terminal's own surface color, the way a folder tab joins its page.
- **The terminal wins.** Chrome never competes with terminal text: smaller type,
  lower contrast, no gradients, no shadows except on floating layers.

## Colors

Slate scale, darkest to lightest. Each step has one job:

| Token | Hex | Use |
|---|---|---|
| `surface-sunken` | `#1B1F27` | Gutters between panes, window background behind content |
| `surface-chrome` | `#21252E` | Header bar, sidebar, tab bar |
| `neutral` | `#282C34` | Terminal surface; also the selected row and active tab |
| `surface-raised` | `#2C313B` | Floating layers: find bar, menus, popovers |
| `surface-hover` | `#313641` | Hover and pressed states on chrome |
| `line` / `line-strong` | `#2C313A` / `#3B4150` | Hairlines between regions / input outlines |

Text: `text-primary` `#E3E6EC` for names and active content, `text-secondary`
`#A2A9B6` for all other chrome text, including counters and meta. There is no
dimmer text step: anything dimmer fails AA on these surfaces, so de-emphasize
with size instead. `focus` `#8A93A6` outlines the focused pane.

Signals (agent state), taken from the same One Dark family as the terminal so
they sit naturally next to terminal output:

| State | Token | Hex |
|---|---|---|
| Needs input / unread attention | `primary` | `#E5A54B` |
| Running | `secondary` | `#56B6C2` |
| Error | `tertiary` | `#E06C75` |
| Done / success | `success` | `#98C379` |
| Idle | `text-secondary` | `#A2A9B6` |

All signal colors pass WCAG AA (4.5:1) as text on `surface-chrome`; the error
red is the tightest (about 4.6:1), so don't put it on lighter surfaces.

## Typography

Two families with separate jobs:

- **Noto Sans CJK KR** (Source Han Sans, from Ubuntu's `fonts-noto-cjk`) for
  all chrome. It was drawn for UI sizes: at 11–13px its Hangul and Latin share
  one stroke weight and x-height, so mixed names like "cmux 개발" read as one
  word, and its Bold is a true bold, which the hierarchy depends on (names and
  states are told apart by weight, not color). NanumSquare Neo was the first
  choice, but GTK 4 does not select its Bold face on this system, so names
  rendered at Regular weight.
- **D2Coding ligature** for terminals (set in the Ghostty config, not by cmux).

Scale: 13px for names and titles (`label-md`, `title-sm`, Bold), 13px Regular
for menus (`body-md`), 12px for tabs (`body-sm`), 11px for meta and status
(`caption`, `status`). Sentence case everywhere; no all-caps labels, no letter
spacing tricks.

## Layout

```
┌──────────────────────────────────────────────────────────────┐
│ [+][↻]              title                 [⊞][↕][▤][≡] _ □ × │ headerbar (chrome)
├────────────┬─────────────────────────────┬───────────────────┤
│▌cmux 개발  │ ~/a  │ ~/b │ ~/c ×          │                   │ tab bar (chrome),
│ ● Running  │━━━━━━┷━━━━━┷━━━━━━━━━━━━━━━━│  terminal         │ active tab = neutral
│  발표 준비  │                             │                   │
│ ● Needs in │  terminal (neutral)         ├───────────────────┤ gutter (sunken)
│  로그 분석 ●│                             │  terminal         │
│            │                             │                   │
│     +      │                             │                   │
└────────────┴─────────────────────────────┴───────────────────┘
```

- Everything is left aligned; names ellipsize at the end.
- Spacing uses the 2/4/6/8/12/16 px scale only. Rows: 8px vertical, 12px left.
- Panes are separated by a 3px `surface-sunken` gutter, not by lines.
- The sidebar is a list, not a stack of cards: rows are flush with 1px gaps.

## Elevation & Depth

Depth comes from the slate steps: sunken gutters, chrome frame, terminal
surface, raised floating layers. Only floating layers (find bar, menus,
popovers) cast a shadow, `0 6px 16px rgba(0,0,0,0.45)`, because they genuinely
sit above the terminal. Nothing else has a shadow, glow or gradient.

## Shapes

- `rounded.md` (6px): workspace rows, icon buttons.
- `rounded.lg` (8px): floating layers.
- `rounded.sm` (4px): small close buttons, menu items.
- `rounded.full`: status lamps and attention dots, which are true circles.
- `rounded.none`: tabs, panes, gutters. Tabs are flat strips attached to the
  pane below, not pills.

## Components

- **Header bar**: `surface-chrome`, 38px, title in `text-secondary`. Icon
  buttons are flat until hovered.
- **Workspace row**: name (`label-md`) on the first line; the second line is the
  agent status as a 7px lamp plus the state in its signal color (`status`),
  followed by the agent's message in `caption`. Selected rows take `neutral`;
  no edge bar or color, lightness alone marks the selection. An unread workspace
  shows an amber `attention-dot` at the right edge.
- **Status lamp**: `needs input` pulses slowly (2s) to draw the eye; all other
  states are steady. The pulse is disabled when GTK animations are off.
- **Tab bar / tab**: tabs size to their title (min 96px, max about 24 chars).
  The active tab takes `neutral`, so it joins the terminal below; inactive
  tabs stay on `surface-chrome`. Close buttons appear on hover and on the
  active tab.
- **Pane**: the focused pane gets a 1px `focus` outline; unfocused panes have
  none. Gutters are `pane-gutter`.
- **Find bar**: `surface-raised` floating at the top-right of the pane, with a
  match counter in `text-secondary` `caption`.
- **Menus**: `surface-raised`, items highlight with `surface-hover`. The tab
  context menu (right-click a tab) offers Rename Tab… and Close Tab.
- **Inline rename** (workspace and tab names): the name turns into a
  `text-input` in place, inside the same window. Enter or clicking away saves,
  Escape cancels, and focus returns to the terminal. No separate dialog
  window: under WSLg a new window doesn't get keyboard focus.
- **Tab names**: a tab shows its custom name if the user gave one (double-click
  the tab, right-click → Rename Tab…, or Ctrl+Alt+R), else the terminal's OSC
  title, else "Terminal N". A renamed tab keeps the tab bar visible even when
  it is the pane's only tab, so the name is never hidden.

## Do's and Don'ts

- Do use a signal color only for an agent state that the user should act on or
  know about; remove it as soon as the state clears.
- Do keep amber unique: if a second element wants amber, it is probably not a
  "needs input" signal.
- Do show selection with lightness (`neutral`, `surface-hover`), not hue.
- Don't add pill badges, colored row backgrounds, or a brand-blue highlight.
- Don't add shadows to anything that isn't floating above the terminal.
- Don't use all-caps or tracked-out labels, or icons that duplicate a text label.
- Don't hardcode colors in widget code or CSS; reference the tokens
  (`src/style/tokens.css` mirrors this file).
