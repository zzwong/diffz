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
        Cancel,
        Quit
    ]
);
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
    fn action_names_are_unique() {
        let mut names = std::collections::HashSet::new();
        for (_, name, _) in COMMANDS {
            assert!(names.insert(name));
        }
    }
}
