//! `--self-check <dir>`: types into the first window as a user would, saves what the windows
//! show as PNG files in `<dir>`, prints what happened, and quits. It checks the prototype without
//! macOS screen-recording or accessibility permissions. Built with `--features self-check`.
//!
//! Keys go through the keymap and the input handler like real key presses
//! (`Window::dispatch_keystroke`). On macOS, the font size and case conversion keys also go in
//! as macOS key events, to check GPUI's reading of the keyboard layout. The IME is played by
//! calling the input handler the way the macOS IME does; whether a real IME behaves is for a
//! person to check (ui-framework.md #1).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use gpui::{App, AsyncApp, Bounds, EntityInputHandler, Keystroke, WindowHandle};

use crate::actions::OpenSampleText;
use crate::editor::Editor;

pub fn take_arg(args: &mut Vec<PathBuf>) -> Option<PathBuf> {
    let i = args
        .iter()
        .position(|arg| arg.as_os_str() == "--self-check")?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

pub fn run(dir: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        match drive(&dir, cx).await {
            Ok(()) => eprintln!("self-check: ok"),
            Err(err) => eprintln!("self-check: FAILED: {err:#}"),
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

async fn drive(dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let window = cx
        .update(|cx| crate::editor_windows(cx).first().copied())
        .context("no editor window")?;
    settle(cx).await;
    // What the files on the command line opened as.
    for window in cx.update(|cx| crate::editor_windows(cx)) {
        window.update(cx, |editor, _, _| {
            eprintln!(
                "self-check: window: {}, {} lines, status: {}",
                editor
                    .path()
                    .map_or_else(|| "(無題)".to_owned(), |path| path.display().to_string()),
                editor.doc.line_count(),
                editor.message().unwrap_or("-"),
            );
        })?;
    }
    save(window, &dir.join("opened.png"), cx)?;

    // Typing at the end of the file: Ctrl/Cmd+End, Enter, letters, Enter.
    press(
        window,
        "secondary-end enter h e l l o space g p u i enter",
        cx,
    )?;

    // The IME composes "にほんご", converts it to "日本語", and commits it.
    let candidate_bounds = window.update(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "にほんご", Some(4..4), window, cx);
        editor.replace_and_mark_text_in_range(None, "日本語", Some(0..3), window, cx);
        let marked = editor.marked_text_range(window, cx)?;
        editor.bounds_for_range(marked, Bounds::default(), window, cx)
    })?;
    eprintln!("self-check: the IME candidate window goes near {candidate_bounds:?}");
    ensure!(
        candidate_bounds.is_some(),
        "no bounds for the composed text"
    );
    settle(cx).await;
    save(window, &dir.join("composing.png"), cx)?;

    window.update(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "日本語", window, cx);
    })?;
    ensure!(
        last_line(window, cx)? == "日本語",
        "the composed text was not committed"
    );

    // The whole composition is one undo step.
    press(window, "secondary-z", cx)?;
    ensure!(
        last_line(window, cx)?.is_empty(),
        "undo left {:?}",
        last_line(window, cx)?
    );
    press(window, "secondary-y", cx)?;
    ensure!(
        last_line(window, cx)? == "日本語",
        "redo did not restore the text"
    );

    press(window, "shift-left shift-left", cx)?;
    settle(cx).await;
    save(window, &dir.join("typed.png"), cx)?;
    let (lines, selection) = window.update(cx, |editor, _, _| {
        (editor.doc.line_count(), editor.doc.selection())
    })?;
    eprintln!("self-check: {lines} lines, selection {selection:?}");

    #[cfg(target_os = "macos")]
    native_keys::check(window, cx).await?;

    check_new_file(dir, cx).await?;

    // The million-line sample, scrolled to its middle.
    cx.update(|cx| cx.dispatch_action(&OpenSampleText));
    settle(cx).await;
    let sample = cx
        .update(|cx| {
            crate::editor_windows(cx).into_iter().find(|window| {
                window
                    .read(cx)
                    .is_ok_and(|e| e.doc.line_count() > 1_000_000)
            })
        })
        .context("the sample text did not open")?;
    sample.update(cx, |editor, _, cx| editor.go_to_line(500_000, cx))?;
    settle(cx).await;
    save(sample, &dir.join("sample.png"), cx)?;
    Ok(())
}

/// Opens a file that does not exist, types into it, and saves it. Sakura opens such a file as
/// an empty document, and saving creates it.
async fn check_new_file(dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    let path = dir.canonicalize()?.join("新規.txt");
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    cx.update(|cx| crate::open_paths(vec![path.clone()], cx));
    let window = cx
        .update(|cx| {
            crate::editor_windows(cx).into_iter().find(|window| {
                window
                    .read(cx)
                    .is_ok_and(|editor| editor.path() == Some(&path))
            })
        })
        .context("the new file did not open")?;
    let message = window.update(cx, |editor, _, _| editor.message().map(str::to_owned))?;
    eprintln!(
        "self-check: opened a new file: {}",
        message.as_deref().unwrap_or("-")
    );
    press(window, "n e w secondary-s", cx)?;
    settle(cx).await;
    let saved = std::fs::read_to_string(&path).context("saving did not create the file")?;
    ensure!(saved == "new", "saved {saved:?}");
    Ok(())
}

fn press(window: WindowHandle<Editor>, keys: &str, cx: &mut AsyncApp) -> Result<()> {
    for key in keys.split_whitespace() {
        let keystroke = Keystroke::parse(key)?;
        // Through the window, not the editor: the editor must not be borrowed while the
        // keystroke runs its action.
        let window: gpui::AnyWindowHandle = window.into();
        window.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx))?;
    }
    Ok(())
}

fn last_line(window: WindowHandle<Editor>, cx: &mut AsyncApp) -> Result<String> {
    window.update(cx, |editor, _, _| {
        let doc = &editor.doc;
        doc.line_text(doc.line_count() - 1, usize::MAX)
    })
}

/// Waits for the window to draw the latest state.
async fn settle(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(500))
        .await;
}

fn save(window: WindowHandle<Editor>, path: &Path, cx: &mut AsyncApp) -> Result<()> {
    let image = window.update(cx, |_, window, _| window.render_to_image())??;
    image.save(path)?;
    eprintln!("self-check: saved {}", path.display());
    Ok(())
}

/// Key presses as macOS key events in the app's own event queue, which needs no permission.
/// They are made as the keyboard makes them (key code, modifier flags and keyboard type), so
/// macOS works out what they type from the keyboard layout, and GPUI reads them as it reads real
/// key presses. Keys that macOS takes before any app sees them are not covered: on a Mac
/// keyboard, F1 to F12 without fn set the brightness, the volume and so on.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, deprecated, unexpected_cfgs)]
mod native_keys {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::rc::Rc;

    use anyhow::{Result, anyhow, bail, ensure};
    use cocoa::appkit::{NSApp, NSApplication};
    use cocoa::base::{BOOL, NO, YES, id};
    use gpui::{AsyncApp, Window, WindowHandle, px};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::settle;
    use crate::document::Selection;
    use crate::editor::Editor;
    use crate::settings::Settings;

    // Virtual key codes (kVK_* in HIToolbox/Events.h) name a place on the keyboard, whatever
    // it types.
    const KEY_L: u16 = 0x25;
    /// = on a US keyboard, ^ on a JIS one.
    const KEY_EQUAL: u16 = 0x18;
    /// - on both; Shift+- is = on a JIS keyboard.
    const KEY_MINUS: u16 = 0x1b;
    /// Shift+; is + on a JIS keyboard.
    const KEY_SEMICOLON: u16 = 0x29;
    const KEY_F6: u16 = 0x61;
    const KEY_F7: u16 = 0x62;

    // Modifier flags (CGEventFlags; NSEvent's are the same bits).
    const NONE: u64 = 0;
    const SHIFT: u64 = 0x2_0000;
    const CTRL: u64 = 0x4_0000;
    const OPT: u64 = 0x8_0000;
    const CMD: u64 = 0x10_0000;
    /// Set on the F keys' events. On a Mac keyboard, fn is held to press them.
    const FN: u64 = 0x80_0000;
    /// kCGKeyboardEventKeyboardType: the kind of keyboard (US, JIS, ...) the key is on.
    const KEYBOARD_TYPE: u32 = 10;
    /// kCGEventSourceStateHIDSystemState: what the keyboards themselves report.
    const HID_SYSTEM_STATE: i32 = 1;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn LMGetKbdType() -> u8;
        fn KBGetLayoutType(keyboard_type: i16) -> u32;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceCreate(state: i32) -> *mut c_void;
        fn CGEventSourceGetKeyboardType(source: *const c_void) -> u32;
        fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }

    struct Key {
        label: &'static str,
        code: u16,
        flags: u64,
    }

    pub async fn check(window: WindowHandle<Editor>, cx: &mut AsyncApp) -> Result<()> {
        // The keys come from the keyboard that was typed on last. Other than JIS, the US layout
        // is assumed.
        let keyboard = unsafe {
            let source = CGEventSourceCreate(HID_SYSTEM_STATE);
            let keyboard = CGEventSourceGetKeyboardType(source);
            CFRelease(source);
            keyboard
        };
        let jis = unsafe { KBGetLayoutType(keyboard as i16) } == u32::from_be_bytes(*b"JIS ");
        eprintln!(
            "self-check: macOS key events from a {} keyboard (type {keyboard}; the app had type {})",
            if jis { "JIS" } else { "US" },
            unsafe { LMGetKbdType() },
        );
        // Where = and + are.
        let (equal, plus) = if jis {
            ((KEY_MINUS, SHIFT), (KEY_SEMICOLON, SHIFT))
        } else {
            ((KEY_EQUAL, NONE), (KEY_EQUAL, SHIFT))
        };
        let new_key = |label, (code, shift): (u16, u64), flags: u64| Key {
            label,
            code,
            flags: flags | shift,
        };

        window.update(cx, |_, window, _| window.activate_window())?;
        settle(cx).await;
        if !window.update(cx, |_, window, _| is_key_window(window))?? {
            bail!("the window did not become the key window, so it gets no key events");
        }
        let keystrokes = Rc::new(RefCell::new(Vec::new()));
        let _subscription = cx.update(|cx| {
            let keystrokes = keystrokes.clone();
            cx.observe_keystrokes(move |event, _, _| {
                let action = event.action.as_ref().map_or("none", |action| action.name());
                keystrokes
                    .borrow_mut()
                    .push(format!("{} → {action}", event.keystroke.unparse()));
            })
        });

        let font_size = |cx: &mut AsyncApp| cx.update(|cx| cx.global::<Settings>().font_size);
        let start = font_size(cx);
        for (key, step) in [
            (new_key("⌘-", (KEY_MINUS, NONE), CMD), -1.),
            (new_key("⌘=", equal, CMD), 1.),
            (new_key("⌘+", plus, CMD), 1.),
            (new_key("⌃-", (KEY_MINUS, NONE), CTRL), -1.),
            (new_key("⌃=", equal, CTRL), 1.),
            (new_key("⌃+", plus, CTRL), 1.),
        ] {
            let before = font_size(cx);
            press(&key, keyboard, cx).await;
            let after = font_size(cx);
            report(&key, &keystrokes, format!("{before:?} → {after:?}"));
            ensure!(
                after == before + px(step),
                "{} did not change the font size",
                key.label
            );
        }
        eprintln!("self-check:   the app now has keyboard type {}", unsafe {
            LMGetKbdType()
        });
        cx.update(|cx| {
            cx.global_mut::<Settings>().font_size = start;
            crate::menus::set_menus(cx);
            cx.refresh_windows();
        });

        // Case conversion of "gpui" in the line "hello gpui".
        window.update(cx, |editor, _, cx| {
            let line = editor.doc.line_count() - 2;
            editor.doc.set_selection(Selection {
                anchor: editor.doc.offset(line, 6),
                head: editor.doc.offset(line, 10),
            });
            cx.notify();
        })?;
        for (key, expected) in [
            (new_key("⌘F7", (KEY_F7, NONE), CMD | FN), "hello GPUI"),
            (new_key("⌘F6", (KEY_F6, NONE), CMD | FN), "hello gpui"),
            (
                new_key("⇧⌥⌘L", (KEY_L, NONE), CMD | OPT | SHIFT),
                "hello GPUI",
            ),
            (new_key("⌥⌘L", (KEY_L, NONE), CMD | OPT), "hello gpui"),
        ] {
            press(&key, keyboard, cx).await;
            let line = window.update(cx, |editor, _, _| {
                editor
                    .doc
                    .line_text(editor.doc.line_count() - 2, usize::MAX)
            })?;
            report(&key, &keystrokes, format!("{line:?}"));
            ensure!(line == expected, "{} did not convert the case", key.label);
        }
        Ok(())
    }

    fn report(key: &Key, keystrokes: &RefCell<Vec<String>>, result: String) {
        let keystrokes = keystrokes.borrow_mut().drain(..).collect::<Vec<_>>();
        eprintln!(
            "self-check:   {} = {}: {result}",
            key.label,
            if keystrokes.is_empty() {
                "(GPUI saw no keystroke)".to_owned()
            } else {
                keystrokes.join(", ")
            }
        );
    }

    fn is_key_window(window: &Window) -> Result<bool> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|err| anyhow!("no handle for the window: {err}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            bail!("not an AppKit window");
        };
        let view = handle.ns_view.as_ptr() as id;
        let key: BOOL = unsafe {
            let ns_window: id = msg_send![view, window];
            msg_send![ns_window, isKeyWindow]
        };
        Ok(key == YES)
    }

    /// Puts the key's down and up events in the app's queue, for the key window, and waits for
    /// them to be handled.
    async fn press(key: &Key, keyboard: u32, cx: &mut AsyncApp) {
        for down in [true, false] {
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), key.code, down);
                CGEventSetFlags(event, key.flags);
                CGEventSetIntegerValueField(event, KEYBOARD_TYPE, keyboard.into());
                let ns_event: id = msg_send![class!(NSEvent), eventWithCGEvent: event];
                NSApp().postEvent_atStart_(ns_event, NO);
                CFRelease(event);
            }
        }
        settle(cx).await;
    }
}
