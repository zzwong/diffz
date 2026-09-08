# Themes

Palettes in Omarchy's `colors.toml` format drive diffz. They colour the
semantic skin tokens of the diff view; they also feed the gpui-component
widget palette. The picker panel shows every available theme, and the theme
file in use reloads as you edit it.

## Selecting a palette

Choose **Omarchy current theme** in the theme picker or launch with
`diffz --theme current change.patch`. This reads
`~/.local/state/omarchy/current/theme/colors.toml`. The choice is saved;
Omarchy colors are opt-in, not automatically selected on first launch.
The app checks the active file's modification time every two seconds.
A named theme or `--theme /path/to/colors.toml` works on other desktops too.

## `colors.toml` format

A theme is either a directory containing `colors.toml` or the `colors.toml`
file itself, addressed by path. Every key holds one `#rrggbb` colour, and
`mode` says either `"dark"` or `"light"`, with anything else counted as dark.
Keys that are not recognised are ignored; comments (`#`) and blank lines cause
no trouble.

Canonical keys, listed in the order the spec defines:

```toml
accent
selection
muted

background
dark_background
darker_background
lighter_background

foreground
dark_foreground
light_foreground
bright_foreground

red
yellow
orange
green
cyan
blue
magenta
brown

bright_red
bright_yellow
bright_green
bright_cyan
bright_blue
bright_magenta
```

Every one of the 25 keys must be present. When a key is missing or a value is
malformed, the error message lists each one, like
`missing: red, green; bad value for accent: "#12"`.

### Legacy aliases

You can use each alias in place of its canonical key. If both spellings show
up, in any order, the canonical one wins:

| Alias       | Canonical           |
| ----------- | ------------------- |
| `bg`        | `background`        |
| `fg`        | `foreground`        |
| `dark_bg`   | `dark_background`   |
| `darker_bg` | `darker_background` |
| `lighter_bg`| `lighter_background`|
| `dark_fg`   | `dark_foreground`   |
| `light_fg`  | `light_foreground`  |
| `bright_fg` | `bright_foreground` |

## Skin mapping

A `Palette` becomes a `Skin` for the diff view plus the chrome around it:

| Skin token             | Source                    |
| ---------------------- | ------------------------- |
| `base`                 | `background`              |
| `surface`              | `lighter_background`      |
| `raised`               | `selection`               |
| `text`                 | `foreground`              |
| `muted`                | `dark_foreground`         |
| `border`               | `muted`                   |
| `accent`               | `accent`                  |
| `selection`            | `selection`               |
| `positive`             | `green`                   |
| `negative`             | `red`                     |
| `warning`              | `yellow`                  |
| `function`             | `magenta`                 |
| `symbol`               | `cyan`                    |
| `added`                | `background`.mix(`green`, 0.18)  |
| `removed`              | `background`.mix(`red`, 0.18)    |
| `added_word`           | `background`.mix(`green`, 0.40)  |
| `removed_word`         | `background`.mix(`red`, 0.40)    |

## Widget mapping

The same palette also fills the gpui-component widget theme
(`ThemeConfig.colors`):

| Widget token             | Source                                   |
| ------------------------ | ---------------------------------------- |
| `background`             | `background`                             |
| `foreground`             | `foreground`                             |
| `border`                 | `muted`                                  |
| `muted`                  | `lighter_background`                     |
| `muted_foreground`       | `dark_foreground`                        |
| `accent`                 | `selection`                              |
| `accent_foreground`      | `foreground`                             |
| `primary`                | `accent`                                 |
| `primary_foreground`     | `background`                             |
| `primary_hover`          | `accent`.mix(`foreground`, 0.15)         |
| `primary_active`         | `accent`.mix(`background`, 0.15)         |
| `secondary`              | `lighter_background`                     |
| `secondary_foreground`   | `foreground`                             |
| `secondary_hover`        | `selection`                              |
| `secondary_active`       | `muted`                                  |
| `input`                  | `muted`                                  |
| `ring`                   | `accent`                                 |
| `selection`              | `selection`                              |
| `popover`                | `dark_background`                        |
| `popover_foreground`     | `foreground`                             |
| `list`                   | `background`                             |
| `list_hover`             | `lighter_background`                     |
| `list_active`            | `selection`                              |
| `list_active_border`     | `accent`                                 |
| `sidebar`                | `dark_background`                        |
| `sidebar_foreground`     | `foreground`                             |
| `sidebar_border`         | `muted`                                  |
| `sidebar_accent`         | `selection`                              |
| `sidebar_primary`        | `accent`                                 |
| `button`                 | `lighter_background`                     |
| `button_hover`           | `selection`                              |
| `button_active`          | `muted`                                  |
| `button_foreground`      | `foreground`                             |
| `button_primary`         | `accent`                                 |
| `button_primary_foreground` | `background`                          |
| `scrollbar`              | `background`                             |
| `scrollbar_thumb`        | `muted`                                  |
| `scrollbar_thumb_hover`  | `dark_foreground`                        |
| `link`                   | `blue`                                   |
| `caret`                  | `bright_foreground`                      |
| `success`                | `green`                                  |
| `warning`                | `yellow`                                 |
| `danger`                 | `red`                                    |
| `info`                   | `blue`                                   |

Widget tokens not listed here stay unset and inherit the gpui-component
fallback instead.
