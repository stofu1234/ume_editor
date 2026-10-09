//! The menu bar, in Sakura's order and with Sakura's labels
//! (`src/main/resources/MainMenu.ini` and the STRINGTABLE in `sakura_core/sakura_rc.rc`).
//! Each menu has only a few of Sakura's items.

use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType};

use crate::actions::*;
use crate::settings::{FONT_PRESETS, FONT_SIZES, Settings};

pub fn set_menus(cx: &mut App) {
    let settings = cx.global::<Settings>();
    let font_menu =
        Menu::new("フォント").items(FONT_PRESETS.iter().enumerate().map(|(i, preset)| {
            MenuItem::action(preset.label, SetFontPreset(i))
                .checked(settings.is_preset(preset))
                .disabled(!settings.is_installed(preset))
        }));
    // Sakura's names for F_SETFONTSIZEUP and F_SETFONTSIZEDOWN, which its menus do not have.
    let size_menu = Menu::new("文字の大きさ").items(
        [
            MenuItem::action("フォントサイズ拡大", FontSizeUp),
            MenuItem::action("フォントサイズ縮小", FontSizeDown),
            MenuItem::separator(),
        ]
        .into_iter()
        .chain(FONT_SIZES.iter().enumerate().map(|(i, size)| {
            MenuItem::action(format!("{size} pt"), SetFontSize(i))
                .checked(settings.font_size.as_f32() == *size)
        })),
    );
    let grid_layout = settings.grid_layout;
    let show_eol = settings.show_eol;

    cx.set_menus([
        Menu::new("ume_editor").items([
            MenuItem::action("ume_editor UI 試作について", About),
            MenuItem::separator(),
            MenuItem::os_submenu("サービス", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("ume_editor を隠す", Hide),
            MenuItem::action("ほかを隠す", HideOthers),
            MenuItem::action("すべてを表示", ShowAll),
            MenuItem::separator(),
            MenuItem::action("ume_editor を終了", Quit),
        ]),
        Menu::new("ファイル").items([
            MenuItem::action("新規作成", FileNew),
            MenuItem::action("開く...", FileOpen),
            MenuItem::action("上書き保存", FileSave),
            MenuItem::action("名前を付けて保存...", FileSaveAs),
            MenuItem::separator(),
            MenuItem::action("閉じる", WinClose),
        ]),
        Menu::new("編集").items([
            MenuItem::os_action("元に戻す", Undo, OsAction::Undo),
            MenuItem::os_action("やり直し", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("切り取り", Cut, OsAction::Cut),
            MenuItem::os_action("コピー", Copy, OsAction::Copy),
            MenuItem::os_action("貼り付け", Paste, OsAction::Paste),
            MenuItem::action("削除", Delete),
            MenuItem::os_action("すべて選択", SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("絵文字と記号", ShowCharacterPalette),
        ]),
        Menu::new("変換").items([
            MenuItem::action("小文字", ToLower),
            MenuItem::action("大文字", ToUpper),
        ]),
        Menu::new("検索").items([
            MenuItem::action("検索...", SearchDialog),
            MenuItem::action("次を検索", SearchNext),
            MenuItem::action("前を検索", SearchPrev),
            MenuItem::action("置換...", ReplaceDialog),
            MenuItem::separator(),
            MenuItem::action("指定行へジャンプ...", JumpDialog),
        ]),
        Menu::new("ツール").items([
            MenuItem::action("キーマクロの記録開始／終了", RecKeyMacro),
            MenuItem::separator(),
            MenuItem::action("100 万行のテキストを開く（試作用）", OpenSampleText),
        ]),
        Menu::new("設定").items([
            MenuItem::submenu(font_menu),
            MenuItem::submenu(size_menu),
            MenuItem::separator(),
            MenuItem::action("全角文字を半角 2 文字分の幅に揃える", ToggleGridLayout)
                .checked(grid_layout),
            MenuItem::action("改行マークを表示", ToggleEolMarks).checked(show_eol),
        ]),
        Menu::new("ウィンドウ").items([
            MenuItem::action("しまう", Minimize),
            MenuItem::action("拡大／縮小", Zoom),
            MenuItem::separator(),
            MenuItem::action("閉じる", WinClose),
        ]),
        Menu::new("ヘルプ").items([MenuItem::action("バージョン情報", About)]),
    ]);
}
