//! Commands and their default keys.
//!
//! Command names follow Sakura's function codes (`sakura_core/Funccode_x.hsrc`): `GoLineTop` is
//! F_GOLINETOP, `PageDownSel` is F_1PageDown_Sel. Commands without a handler are placeholders
//! that show Sakura's menu layout; GPUI greys them out.

use gpui::{Action, App, KeyBinding, actions};

actions!(
    ume,
    [
        // Application
        About,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        // ファイル
        FileNew,
        FileOpen,
        FileSave,
        FileSaveAs,
        WinClose,
        // 編集
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        Delete,
        DeleteBack,
        SelectAll,
        Newline,
        IndentTab,
        ShowCharacterPalette,
        // 変換
        ToLower,
        ToUpper,
        // 検索 (placeholders)
        SearchDialog,
        SearchNext,
        SearchPrev,
        ReplaceDialog,
        JumpDialog,
        // ツール
        RecKeyMacro,
        OpenSampleText,
        // 設定
        ToggleGridLayout,
        ToggleEolMarks,
        FontSizeUp,
        FontSizeDown,
        // ウィンドウ
        Minimize,
        Zoom,
        // Caret movement
        Left,
        Right,
        Up,
        Down,
        GoLineTop,
        GoLineEnd,
        GoFileTop,
        GoFileEnd,
        PageUp,
        PageDown,
        LeftSel,
        RightSel,
        UpSel,
        DownSel,
        GoLineTopSel,
        GoLineEndSel,
        GoFileTopSel,
        GoFileEndSel,
        PageUpSel,
        PageDownSel,
    ]
);

/// Selects the font preset at this index of [`crate::settings::FONT_PRESETS`].
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ume, no_json)]
pub struct SetFontPreset(pub usize);

/// Selects the font size at this index of [`crate::settings::FONT_SIZES`].
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = ume, no_json)]
pub struct SetFontSize(pub usize);

pub const EDITOR_CONTEXT: &str = "Editor";

/// Sakura's default keys (`sakura_core/func/CKeyBind.cpp`), with Ctrl read as Cmd on macOS
/// (`secondary-`, ADR-0008), plus the macOS keys that every Mac app has.
pub fn bind_keys(cx: &mut App) {
    let editor = Some(EDITOR_CONTEXT);
    cx.bind_keys([
        // Ctrl+N/O/S, Shift+Ctrl+S
        KeyBinding::new("secondary-n", FileNew, None),
        KeyBinding::new("secondary-o", FileOpen, None),
        KeyBinding::new("secondary-s", FileSave, editor),
        KeyBinding::new("secondary-shift-s", FileSaveAs, editor),
        // Ctrl+Z/Y/X/C/V/A
        KeyBinding::new("secondary-z", Undo, editor),
        KeyBinding::new("secondary-y", Redo, editor),
        KeyBinding::new("secondary-x", Cut, editor),
        KeyBinding::new("secondary-c", Copy, editor),
        KeyBinding::new("secondary-v", Paste, editor),
        KeyBinding::new("secondary-a", SelectAll, editor),
        // BkSp, Del, Tab, Enter
        KeyBinding::new("backspace", DeleteBack, editor),
        KeyBinding::new("shift-backspace", DeleteBack, editor),
        KeyBinding::new("delete", Delete, editor),
        KeyBinding::new("tab", IndentTab, editor),
        KeyBinding::new("enter", Newline, editor),
        KeyBinding::new("shift-enter", Newline, editor),
        // Ctrl+F6, Ctrl+F7, and Ctrl+Alt+L, Shift+Ctrl+Alt+L, which a Mac keyboard can press
        // without holding fn
        KeyBinding::new("secondary-f6", ToLower, editor),
        KeyBinding::new("secondary-f7", ToUpper, editor),
        KeyBinding::new("secondary-alt-l", ToLower, editor),
        KeyBinding::new("secondary-alt-shift-l", ToUpper, editor),
        // Ctrl+F, F3, Shift+F3, Ctrl+R, Ctrl+J
        KeyBinding::new("secondary-f", SearchDialog, editor),
        KeyBinding::new("f3", SearchNext, editor),
        KeyBinding::new("shift-f3", SearchPrev, editor),
        KeyBinding::new("secondary-r", ReplaceDialog, editor),
        KeyBinding::new("secondary-j", JumpDialog, editor),
        // Shift+Ctrl+M
        KeyBinding::new("secondary-shift-m", RecKeyMacro, editor),
        // Arrows, Home/End (Ctrl: file top/end), PgUp/PgDn; Shift selects
        KeyBinding::new("left", Left, editor),
        KeyBinding::new("right", Right, editor),
        KeyBinding::new("up", Up, editor),
        KeyBinding::new("down", Down, editor),
        KeyBinding::new("home", GoLineTop, editor),
        KeyBinding::new("end", GoLineEnd, editor),
        KeyBinding::new("secondary-home", GoFileTop, editor),
        KeyBinding::new("secondary-end", GoFileEnd, editor),
        KeyBinding::new("pageup", PageUp, editor),
        KeyBinding::new("pagedown", PageDown, editor),
        KeyBinding::new("shift-left", LeftSel, editor),
        KeyBinding::new("shift-right", RightSel, editor),
        KeyBinding::new("shift-up", UpSel, editor),
        KeyBinding::new("shift-down", DownSel, editor),
        KeyBinding::new("shift-home", GoLineTopSel, editor),
        KeyBinding::new("shift-end", GoLineEndSel, editor),
        KeyBinding::new("secondary-shift-home", GoFileTopSel, editor),
        KeyBinding::new("secondary-shift-end", GoFileEndSel, editor),
        KeyBinding::new("shift-pageup", PageUpSel, editor),
        KeyBinding::new("shift-pagedown", PageDownSel, editor),
        // Font size. Sakura has no keys for it (only Ctrl+wheel); these are other apps' keys.
        // `+` is Shift+= on a US keyboard and Shift+; on a JIS keyboard, where = is Shift+-.
        KeyBinding::new("secondary-=", FontSizeUp, None),
        KeyBinding::new("secondary-+", FontSizeUp, None),
        KeyBinding::new("secondary--", FontSizeDown, None),
    ]);

    // Sakura's Ctrl+W selects a word and Ctrl+←/→ move by words, but on a Mac these keys
    // close the window and move to the line start/end. The Mac meaning wins here.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", WinClose, editor),
        KeyBinding::new("cmd-m", Minimize, editor),
        KeyBinding::new("cmd-shift-z", Redo, editor),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, editor),
        KeyBinding::new("cmd-left", GoLineTop, editor),
        KeyBinding::new("cmd-right", GoLineEnd, editor),
        KeyBinding::new("cmd-up", GoFileTop, editor),
        KeyBinding::new("cmd-down", GoFileEnd, editor),
        KeyBinding::new("cmd-shift-left", GoLineTopSel, editor),
        KeyBinding::new("cmd-shift-right", GoLineEndSel, editor),
        KeyBinding::new("cmd-shift-up", GoFileTopSel, editor),
        KeyBinding::new("cmd-shift-down", GoFileEndSel, editor),
        // The font size keys with Control too, as on Windows. ADR-0008 also proposes a keymap
        // that keeps Ctrl on a Mac.
        KeyBinding::new("ctrl-=", FontSizeUp, None),
        KeyBinding::new("ctrl-+", FontSizeUp, None),
        KeyBinding::new("ctrl--", FontSizeDown, None),
    ]);

    // Alt+F4 closes the window in Sakura (`CKeyBind::GetDefFuncCode`).
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([KeyBinding::new("alt-f4", WinClose, editor)]);
}
