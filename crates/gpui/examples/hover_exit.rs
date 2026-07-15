#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct HoverExit {
    hovered: bool,
}

impl Render for HoverExit {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Fills the whole window so its edge is the window edge: moving the mouse
        // out of the window is what exercises the MouseExited path.
        div()
            .id("hover-exit")
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .text_xl()
            .text_color(rgb(0xffffff))
            .bg(if self.hovered {
                rgb(0x585f58)
            } else {
                rgb(0x505050)
            })
            .child(if self.hovered {
                "HOVERED"
            } else {
                "not hovered"
            })
            .on_hover(cx.listener(|this, hovered, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(240.), px(160.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("gpui-hover-exit".to_string()),
                ..Default::default()
            },
            |_, cx| cx.new(|_| HoverExit { hovered: false }),
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
