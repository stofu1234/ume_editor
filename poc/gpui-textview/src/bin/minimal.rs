//! Baseline for memory comparison: a plain GPUI window with one line of text.
//! POC_INIT_COMPONENT / POC_ROOT add gpui-component's init and Root wrapper.

use gpui::{
    App, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

#[path = "../memstat.rs"]
mod memstat;

#[global_allocator]
static ALLOC: memstat::CountingAlloc = memstat::CountingAlloc;

struct Hello;

impl Render for Hello {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .font_family("BIZ UDGothic")
            .child("こんにちは GPUI")
            .children(std::env::var_os("POC_DEFAULT_FONT").map(|_| {
                div()
                    .font_family(".SystemUIFont")
                    .child("既定のフォントで描いた行 (無題)")
            }))
    }
}

fn main() {
    memstat::start_from_env();
    application().run(|cx: &mut App| {
        if std::env::var("POC_TEXT_MODE").as_deref() == Ok("grayscale") {
            cx.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        }
        if std::env::var_os("POC_INIT_COMPONENT").is_some() {
            gpui_component::init(cx);
        }
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        if std::env::var_os("POC_ROOT").is_some() {
            cx.open_window(options, |window, cx| {
                let hello = cx.new(|_| Hello);
                cx.new(|cx| gpui_component::Root::new(hello, window, cx))
            })
            .expect("failed to open window");
        } else {
            cx.open_window(options, |_, cx| cx.new(|_| Hello))
                .expect("failed to open window");
        }
    });
}
