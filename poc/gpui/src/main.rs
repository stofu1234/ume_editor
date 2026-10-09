//! UI prototype of ume_editor on GPUI, to settle ADR-0004. See docs/poc/ui-framework.md.
//!
//! Throwaway code: the real editor is written again in Phase 1.

mod actions;
mod document;
mod editor;
mod element;
mod menus;
#[cfg(feature = "self-check")]
mod self_check;
mod settings;

use std::path::{Path, PathBuf};
use std::time::Instant;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, PathPromptOptions, PromptLevel, WindowHandle, px};

use actions::*;
use document::Document;
use editor::Editor;
use settings::{FONT_PRESETS, FONT_SIZES, Settings};

fn main() {
    element::STARTED.get_or_init(Instant::now);
    env_logger::init();

    #[cfg_attr(not(feature = "self-check"), allow(unused_mut))]
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    #[cfg(feature = "self-check")]
    let self_check = self_check::take_arg(&mut args);
    // Finder sends the files to open ("Open With", drag onto the Dock icon) as file URLs.
    let (url_sender, mut url_receiver) = mpsc::unbounded::<Vec<String>>();

    let app = gpui_platform::application();
    app.on_open_urls(move |urls| {
        url_sender.unbounded_send(urls).ok();
    });
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            editor::open_window(cx);
        }
    });
    app.run(move |cx| {
        cx.set_global(Settings::new());
        actions::bind_keys(cx);
        register_app_actions(cx);
        menus::set_menus(cx);
        settings::find_installed_fonts(cx);

        let mut paths = args;
        while let Ok(urls) = url_receiver.try_recv() {
            paths.extend(file_paths(urls));
        }
        if paths.is_empty() {
            editor::open_window(cx);
        } else {
            open_paths(paths, cx);
        }
        cx.spawn(async move |cx| {
            while let Some(urls) = url_receiver.next().await {
                cx.update(|cx| open_paths(file_paths(urls), cx));
            }
        })
        .detach();
        cx.activate(true);
        #[cfg(feature = "self-check")]
        if let Some(dir) = self_check {
            self_check::run(dir, cx);
        }
    });
}

fn register_app_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| quit(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &About, cx| about(cx));
    cx.on_action(|_: &FileNew, cx| {
        editor::open_window(cx);
    });
    cx.on_action(|_: &FileOpen, cx| prompt_and_open(cx));
    cx.on_action(|_: &OpenSampleText, cx| open_sample_text(cx));
    cx.on_action(|_: &ToggleGridLayout, cx| {
        update_settings(cx, |settings| settings.grid_layout = !settings.grid_layout)
    });
    cx.on_action(|_: &ToggleEolMarks, cx| {
        update_settings(cx, |settings| settings.show_eol = !settings.show_eol)
    });
    cx.on_action(|action: &SetFontPreset, cx| {
        update_settings(cx, |settings| settings.set_preset(&FONT_PRESETS[action.0]))
    });
    cx.on_action(|action: &SetFontSize, cx| {
        update_settings(cx, |settings| settings.font_size = px(FONT_SIZES[action.0]))
    });
    cx.on_action(|_: &FontSizeUp, cx| {
        update_settings(cx, |settings| {
            settings.font_size = (settings.font_size + px(1.)).min(px(72.))
        })
    });
    cx.on_action(|_: &FontSizeDown, cx| {
        update_settings(cx, |settings| {
            settings.font_size = (settings.font_size - px(1.)).max(px(6.))
        })
    });
}

fn update_settings(cx: &mut App, update: impl FnOnce(&mut Settings)) {
    update(cx.global_mut::<Settings>());
    menus::set_menus(cx);
    cx.refresh_windows();
}

fn file_paths(urls: Vec<String>) -> Vec<PathBuf> {
    urls.into_iter()
        .filter_map(|url| {
            url::Url::parse(&url)
                .ok()
                .and_then(|url| url.to_file_path().ok())
        })
        .collect()
}

pub(crate) fn editor_windows(cx: &App) -> Vec<WindowHandle<Editor>> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<Editor>())
        .collect()
}

/// A window with an untitled, untouched document, which opening a file reuses: the active one
/// if it qualifies, else any.
fn reusable_window(cx: &App) -> Option<WindowHandle<Editor>> {
    let is_pristine =
        |window: &WindowHandle<Editor>| window.read(cx).is_ok_and(|editor| editor.is_pristine());
    cx.active_window()
        .and_then(|window| window.downcast::<Editor>())
        .filter(is_pristine)
        .or_else(|| editor_windows(cx).into_iter().find(is_pristine))
}

/// The real, absolute path of a file to open. A file that does not exist yet gets its folder's
/// real path, so that opening it again finds the window that has it.
fn absolute_path(path: PathBuf) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    match (folder.canonicalize(), path.file_name()) {
        (Ok(folder), Some(name)) => folder.join(name),
        _ => std::path::absolute(&path).unwrap_or(path),
    }
}

/// Opens each file in its own window, as Sakura does. A file that is already open brings its
/// window to the front instead.
pub fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    for path in paths {
        let path = absolute_path(path);
        let open = editor_windows(cx).into_iter().find(|window| {
            window
                .read(cx)
                .is_ok_and(|editor| editor.path() == Some(&path))
        });
        if let Some(window) = open {
            window
                .update(cx, |_, window, _| window.activate_window())
                .ok();
            continue;
        }
        let Some(window) = reusable_window(cx).or_else(|| editor::open_window(cx)) else {
            continue;
        };
        window
            .update(cx, |editor, window, cx| {
                editor.open_file(&path, cx);
                window.activate_window();
            })
            .ok();
    }
}

fn prompt_and_open(cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("開く".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = receiver.await {
            cx.update(|cx| open_paths(paths, cx));
        }
    })
    .detach();
}

/// Opens a generated text of a million lines for the scrolling check
/// (docs/poc/ui-framework.md #4).
fn open_sample_text(cx: &mut App) {
    let Some(window) = reusable_window(cx).or_else(|| editor::open_window(cx)) else {
        return;
    };
    window
        .update(cx, |editor, window, cx| {
            let started = Instant::now();
            editor.set_document(Document::sample(1_000_000), cx);
            editor.set_message(
                format!(
                    "100 万行を生成しました（{:.0} ms）",
                    started.elapsed().as_secs_f64() * 1000.
                ),
                cx,
            );
            window.activate_window();
        })
        .ok();
}

fn quit(cx: &mut App) {
    let modified: Vec<_> = editor_windows(cx)
        .into_iter()
        .filter(|window| window.read(cx).is_ok_and(|editor| editor.doc.is_modified()))
        .collect();
    let Some(&first) = modified.first() else {
        cx.quit();
        return;
    };
    let count = modified.len();
    first
        .update(cx, |_, window, cx| {
            window.activate_window();
            let answer = window.prompt(
                PromptLevel::Warning,
                &format!("保存していない文書が {count} 個あります。"),
                Some("保存せずに終了しますか？"),
                &["終了", "キャンセル"],
                cx,
            );
            cx.spawn(async move |_, cx| {
                if answer.await == Ok(0) {
                    cx.update(|cx| cx.quit());
                }
            })
            .detach();
        })
        .ok();
}

fn about(cx: &mut App) {
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    let first_frame = element::FIRST_FRAME.get().map_or_else(
        || "未計測".to_owned(),
        |elapsed| format!("{:.0} ms", elapsed.as_secs_f64() * 1000.),
    );
    let memory = resident_memory_kb().map_or_else(
        || "不明".to_owned(),
        |kb| format!("{:.1} MB", kb as f64 / 1024.),
    );
    let shaders = if cfg!(feature = "runtime-shaders") {
        "起動時にコンパイル"
    } else {
        "ビルド時にコンパイル済み"
    };
    let detail = format!(
        "GPUI: gpui-pre 0.3.8（Zed のスナップショット）\n\
         Metal シェーダー: {shaders}\n\
         起動から最初の描画まで: {first_frame}\n\
         メモリ（RSS）: {memory}"
    );
    window
        .update(cx, |_, window, cx| {
            // Nobody waits for the answer; dropping the receiver leaves the dialog open.
            drop(window.prompt(
                PromptLevel::Info,
                "ume_editor UI 試作（GPUI）",
                Some(&detail),
                &["OK"],
                cx,
            ));
        })
        .ok();
}

/// The resident memory of this process, from `ps`.
fn resident_memory_kb() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_gets_its_folders_real_path() {
        // The temporary folder on a Mac is behind a symbolic link (/var -> /private/var).
        let dir = std::env::temp_dir().join(format!("ume-poc-gpui-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.canonicalize().unwrap();
        assert_eq!(absolute_path(dir.join("新規.txt")), real.join("新規.txt"));
        std::fs::write(dir.join("a.txt"), "").unwrap();
        assert_eq!(absolute_path(dir.join("a.txt")), real.join("a.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
