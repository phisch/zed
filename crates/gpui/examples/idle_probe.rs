#![cfg_attr(target_family = "wasm", no_main)]

use std::time::Duration;

use gpui::{
    App, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct IdleProbe {
    ticks: usize,
}

impl Render for IdleProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update(cx, |this, cx| {
                this.ticks += 1;
                println!("tick {} -> notify", this.ticks);
                cx.notify();
            })
            .ok();
        })
        .detach();

        let colors = [0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0xff00ff];
        let color = colors[self.ticks % colors.len()];
        println!("render tick={}", self.ticks);
        div()
            .bg(rgb(color))
            .size(px(500.0))
            .text_color(rgb(0xffffff))
            .text_xl()
            .child(format!("ticks: {}", self.ticks))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(500.), px(500.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| IdleProbe { ticks: 0 }),
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
