use gpui_kit::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    Recent,
    Find,
    Files,
    Inspector,
    Wrap,
    Split,
    Rich,
    RichInline,
    NextFile,
    PreviousFile,
    NextHunk,
    PreviousHunk,
    Copy,
    Comment,
    CommentFile,
    Preview,
    Refresh,
    Palette,
    ZoomIn,
    ZoomOut,
    Theme,
    Themes,
    Export,
    Keys,
    Cancel,
}
pub const COMMANDS: &[(Command, &str, &str)] = &[
    (Command::Recent, "Switch recent review", "primary-shift-o"),
    (Command::Open, "Open source", "primary-o"),
    (Command::Find, "Find in loaded diff", "primary-f"),
    (Command::Files, "Toggle files", "primary-b"),
    (Command::Inspector, "Toggle review overview", "primary-i"),
    (Command::Wrap, "Toggle soft wrap", "alt-z"),
    (Command::Split, "Toggle split view", "primary-alt-s"),
    (
        Command::Rich,
        "Toggle rich diff for prose",
        "primary-shift-r",
    ),
    (
        Command::RichInline,
        "Show or hide word-level marks within rich diff blocks",
        "primary-shift-i",
    ),
    (Command::NextFile, "Next file", "]"),
    (Command::PreviousFile, "Previous file", "["),
    (Command::NextHunk, "Next hunk", "n"),
    (Command::PreviousHunk, "Previous hunk", "p"),
    (Command::Copy, "Copy source selection", "primary-c"),
    (Command::Comment, "Draft comment on source", "c"),
    (Command::CommentFile, "Comment on file", "shift-c"),
    (
        Command::Preview,
        "Preview review — never submit",
        "primary-enter",
    ),
    (Command::Refresh, "Check for new revision", "primary-r"),
    (Command::Palette, "Command palette", "primary-k"),
    (Command::ZoomIn, "Larger text", "primary-="),
    (Command::ZoomOut, "Smaller text", "primary--"),
    (Command::Theme, "Toggle light/dark theme", ""),
    (Command::Themes, "Choose theme…", ""),
    (Command::Export, "Export selected context", ""),
    (Command::Keys, "Keyboard shortcuts", "f1"),
    (Command::Cancel, "Close panel / clear selection", "escape"),
];
actions!(
    workbench,
    [
        Open,
        Recent,
        Find,
        Files,
        Inspector,
        Wrap,
        Split,
        Rich,
        RichInline,
        NextFile,
        PreviousFile,
        NextHunk,
        PreviousHunk,
        CopySource,
        Comment,
        CommentFile,
        Preview,
        Refresh,
        Palette,
        ZoomIn,
        ZoomOut,
        Themes,
        Keys,
        Cancel,
        Quit
    ]
);
/// One row of the keyboard sheet.
pub struct SheetRow {
    pub keys: SheetKeys,
    pub label: &'static str,
}
pub enum SheetKeys {
    /// Key text comes from `COMMANDS`, so the sheet cannot drift from the bindings.
    Run(&'static [Command]),
    /// Keys no command owns, each written as a modifier prefix and the key after it.
    Fixed(&'static [(&'static str, &'static str)]),
}
pub struct SheetGroup {
    pub name: &'static str,
    pub rows: &'static [SheetRow],
}
const fn run(keys: &'static [Command], label: &'static str) -> SheetRow {
    SheetRow {
        keys: SheetKeys::Run(keys),
        label,
    }
}
const fn fixed(keys: &'static [(&'static str, &'static str)], label: &'static str) -> SheetRow {
    SheetRow {
        keys: SheetKeys::Fixed(keys),
        label,
    }
}
pub const SHEET: &[SheetGroup] = &[
    SheetGroup {
        name: "Navigate",
        rows: &[
            run(
                &[Command::NextFile, Command::PreviousFile],
                "next / previous file",
            ),
            run(
                &[Command::NextHunk, Command::PreviousHunk],
                "next / previous hunk",
            ),
            run(&[Command::Find], "find in the loaded diff"),
            fixed(&[("", "↑ ↓"), ("", "← →")], "move the selection"),
            fixed(&[("shift-", "↑ ↓ ← →")], "extend the selection"),
            fixed(&[("", "PgUp"), ("", "PgDn")], "scroll a page"),
            fixed(&[("", "Home"), ("", "End")], "start / end of the line"),
            fixed(
                &[("primary-", "Home"), ("primary-", "End")],
                "first / last line",
            ),
            fixed(
                &[("alt-", "↑"), ("alt-", "↓")],
                "more context above / below",
            ),
        ],
    },
    SheetGroup {
        name: "Review",
        rows: &[
            run(&[Command::Comment], "comment on the selected lines"),
            run(&[Command::CommentFile], "comment on the whole file"),
            run(&[Command::Copy], "copy the selected source"),
            run(&[Command::Inspector], "review overview"),
            run(&[Command::Preview], "preview the review"),
            run(&[Command::Refresh], "check for a new revision"),
        ],
    },
    SheetGroup {
        name: "View",
        rows: &[
            run(&[Command::Files], "file tree"),
            run(&[Command::Wrap], "soft wrap"),
            run(&[Command::Split], "split view"),
            run(&[Command::Rich], "rich diff for prose"),
            run(&[Command::RichInline], "word marks in rich diff"),
            run(
                &[Command::ZoomIn, Command::ZoomOut],
                "larger / smaller text",
            ),
        ],
    },
    SheetGroup {
        name: "File tree",
        rows: &[
            fixed(&[("", "↑ ↓")], "move through the files"),
            fixed(&[("", "↵")], "open the file"),
            fixed(&[("", "← →")], "fold / unfold"),
            fixed(&[("", "Space")], "mark reviewed"),
            fixed(&[("", "Home"), ("", "End")], "first / last row"),
        ],
    },
    SheetGroup {
        name: "App",
        rows: &[
            run(&[Command::Palette], "command palette"),
            run(&[Command::Keys], "this list of keys"),
            run(&[Command::Open], "open a source"),
            run(&[Command::Recent], "switch recent review"),
            fixed(&[("", "Tab")], "move focus between panes"),
            run(&[Command::Cancel], "close the panel, clear the selection"),
        ],
    },
];
/// Key text for a sheet row, one string per keycap.
pub fn sheet_keys(row: &SheetRow) -> Vec<String> {
    match row.keys {
        SheetKeys::Run(commands) => commands.iter().copied().filter_map(shortcut).collect(),
        SheetKeys::Fixed(keys) => keys.iter().map(|&(m, k)| fixed_key(m, k)).collect(),
    }
}
/// Key text for a key no command owns: the modifier follows the platform, the
/// rest reads as written.
pub fn fixed_key(modifier: &str, key: &str) -> String {
    format!("{}{key}", display_key(modifier))
}
/// Build the native menu. On macOS, a main menu makes the program visible to
/// the app switcher, Dock, and standard quit shortcut.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("diffz").items([MenuItem::action("Quit diffz", Quit)]),
        Menu::new("File").items([
            MenuItem::action("Open Source…", Open),
            MenuItem::action("Switch Recent Review…", Recent),
            MenuItem::separator(),
            MenuItem::action("Check for New Revision", Refresh),
            MenuItem::action("Preview Review", Preview),
        ]),
        Menu::new("View").items([
            MenuItem::action("Find in Diff", Find),
            MenuItem::action("Command Palette", Palette),
            MenuItem::action("Keyboard Shortcuts", Keys),
            MenuItem::separator(),
            MenuItem::action("Toggle Files", Files),
            MenuItem::action("Toggle Review Overview", Inspector),
            MenuItem::action("Toggle Soft Wrap", Wrap),
            MenuItem::action("Toggle Split View", Split),
            MenuItem::action("Toggle Rich Diff", Rich),
            MenuItem::action("Toggle Rich Diff Word Marks", RichInline),
            MenuItem::separator(),
            MenuItem::action("Larger Text", ZoomIn),
            MenuItem::action("Smaller Text", ZoomOut),
            MenuItem::separator(),
            MenuItem::action("Choose Theme…", Themes),
        ]),
        Menu::new("Go").items([
            MenuItem::action("Next File", NextFile),
            MenuItem::action("Previous File", PreviousFile),
            MenuItem::action("Next Hunk", NextHunk),
            MenuItem::action("Previous Hunk", PreviousHunk),
        ]),
    ]
}
pub fn bind(cx: &mut App) {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let key = |s: &str| s.replace("primary", primary);
    cx.bind_keys([
        KeyBinding::new(&key("primary-shift-o"), Recent, Some("Workbench")),
        KeyBinding::new(&key("primary-o"), Open, Some("Workbench")),
        KeyBinding::new(&key("primary-f"), Find, Some("Workbench")),
        KeyBinding::new(&key("primary-b"), Files, Some("Workbench")),
        KeyBinding::new(&key("primary-i"), Inspector, Some("Workbench")),
        KeyBinding::new(&key("primary-k"), Palette, Some("Workbench")),
        KeyBinding::new(&key("primary-r"), Refresh, Some("Workbench")),
        KeyBinding::new(&key("primary-enter"), Preview, Some("Workbench")),
        KeyBinding::new("f1", Keys, Some("Workbench")),
        // Shift and slash arrive as the single key "?" on every platform.
        KeyBinding::new("?", Keys, Some("WorkbenchDiff")),
        KeyBinding::new("alt-z", Wrap, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary-alt-s"), Split, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary-shift-r"), Rich, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary-shift-i"), RichInline, Some("WorkbenchDiff")),
        KeyBinding::new("]", NextFile, Some("WorkbenchDiff")),
        KeyBinding::new("[", PreviousFile, Some("WorkbenchDiff")),
        KeyBinding::new("n", NextHunk, Some("WorkbenchDiff")),
        KeyBinding::new("p", PreviousHunk, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary-c"), CopySource, Some("WorkbenchDiff")),
        KeyBinding::new("c", Comment, Some("WorkbenchDiff")),
        KeyBinding::new("shift-c", CommentFile, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary-="), ZoomIn, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary-+"), ZoomIn, Some("WorkbenchDiff")),
        KeyBinding::new(&key("primary--"), ZoomOut, Some("WorkbenchDiff")),
        KeyBinding::new("escape", Cancel, Some("Workbench")),
        KeyBinding::new(&key("primary-q"), Quit, None),
    ]);
}
/// Return the platform-specific shortcut text for `command`, when defined.
pub fn shortcut(command: Command) -> Option<String> {
    COMMANDS
        .iter()
        .find(|(c, _, key)| *c == command && !key.is_empty())
        .map(|(_, _, key)| display_key(key))
}
/// Combine a tooltip with its shortcut so controls expose their key binding.
pub fn tip(text: &str, command: Command) -> String {
    match shortcut(command) {
        Some(key) => format!("{text} · {key}"),
        None => text.to_string(),
    }
}
pub fn display_key(key: &str) -> String {
    display_key_for(key, cfg!(target_os = "macos"))
}
fn display_key_for(key: &str, mac: bool) -> String {
    key.replace("primary-", if mac { "⌘" } else { "Ctrl+" })
        .replace("shift-", if mac { "⇧" } else { "Shift+" })
        .replace("alt-", if mac { "⌥" } else { "Alt+" })
        .replace('=', "+")
        .replace("enter", "↵")
        .replace("escape", "Esc")
        .to_uppercase()
        .replace("CTRL", "Ctrl")
        .replace("SHIFT", "Shift")
        .replace("ALT", "Alt")
        .replace("ESC", "Esc")
}

#[cfg(test)]
mod tests {
    use super::{COMMANDS, Command, display_key_for};
    #[test]
    fn platform_shortcut_labels() {
        assert_eq!(display_key_for("primary-=", true), "⌘+");
        assert_eq!(display_key_for("primary-=", false), "Ctrl++");
        assert_eq!(display_key_for("primary-shift-o", true), "⌘⇧O");
        assert_eq!(display_key_for("primary-shift-o", false), "Ctrl+Shift+O");
        assert_eq!(display_key_for("primary-alt-s", false), "Ctrl+Alt+S");
    }
    #[test]
    fn reserved_chat_shortcut_is_not_bound() {
        assert!(COMMANDS.iter().all(|(_, _, key)| !key.ends_with("-j")));
    }
    #[test]
    fn tooltips_carry_the_bound_shortcut() {
        let files = super::tip("Toggle file tree", Command::Files);
        assert!(files.starts_with("Toggle file tree · "), "{files}");
        assert!(files.ends_with('B'), "{files}");
        assert_eq!(super::shortcut(Command::Theme), None);
        assert_eq!(
            super::tip("Toggle appearance", Command::Theme),
            "Toggle appearance"
        );
    }
    #[test]
    fn menu_bar_has_an_app_menu_first() {
        let menus = super::menus();
        assert_eq!(menus[0].name.as_ref(), "diffz");
        assert!(menus.iter().all(|m| !m.items.is_empty()));
    }
    #[test]
    fn every_bound_command_is_on_the_sheet_once() {
        for (command, label, key) in COMMANDS {
            if key.is_empty() {
                continue;
            }
            let count = sheet_commands().filter(|c| c == command).count();
            assert_eq!(count, 1, "{label} appears {count} times on the sheet");
        }
    }
    #[test]
    fn the_sheet_lists_no_unbound_command() {
        for command in sheet_commands() {
            assert!(super::shortcut(command).is_some(), "{command:?} has no key");
        }
    }
    #[test]
    fn sheet_key_text_comes_from_the_bindings() {
        for row in super::SHEET.iter().flat_map(|group| group.rows) {
            let super::SheetKeys::Run(commands) = row.keys else {
                continue;
            };
            let expected: Vec<String> = commands
                .iter()
                .map(|&c| super::shortcut(c).expect("bound"))
                .collect();
            assert_eq!(super::sheet_keys(row), expected, "{}", row.label);
        }
    }
    #[test]
    fn fixed_sheet_keys_carry_the_platform_modifier() {
        assert_eq!(super::fixed_key("", "Space"), "Space");
        assert_eq!(
            super::fixed_key("shift-", "↑"),
            if cfg!(target_os = "macos") {
                "⇧↑"
            } else {
                "Shift+↑"
            }
        );
    }
    fn sheet_commands() -> impl Iterator<Item = Command> {
        super::SHEET
            .iter()
            .flat_map(|group| group.rows)
            .filter_map(|row| match row.keys {
                super::SheetKeys::Run(commands) => Some(commands),
                super::SheetKeys::Fixed(_) => None,
            })
            .flatten()
            .copied()
    }
    #[test]
    fn action_names_are_unique() {
        let mut names = std::collections::HashSet::new();
        for (_, name, _) in COMMANDS {
            assert!(names.insert(name));
        }
    }
}
