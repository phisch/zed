//! Manual test for `Window::set_exclusive_zone` and `Window::set_exclusive_edge`
//! on a wlr-layer-shell surface. Not committed.
//!
//! Run with: cargo run -p gpui --example layer_shell_exclusive
//!
//! The window is a top bar anchored TOP | LEFT | RIGHT. Use the buttons to change
//! the exclusive zone at runtime (maximize another window to watch the reserved
//! space change) and to set the exclusive edge. The BOTTOM edge is not one the
//! surface is anchored to, so the guard logs and ignores it instead of raising a
//! fatal protocol error.

#![cfg_attr(target_family = "wasm", no_main)]

fn run_example() {
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    example::main();

    #[cfg(not(all(target_os = "linux", feature = "wayland")))]
    panic!("This example requires the `wayland` feature and a linux system.");
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

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod example {
    use gpui::{
        App, Bounds, Context, FontWeight, Pixels, SharedString, Size, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, layer_shell::*,
        point, prelude::*, px, rgb, rgba, white,
    };
    use gpui_platform::application;

    struct ExclusiveExample {
        zone: Pixels,
        status: SharedString,
    }

    impl ExclusiveExample {
        fn button(
            id: &'static str,
            label: impl Into<SharedString>,
            cx: &mut Context<Self>,
            handler: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        ) -> impl IntoElement {
            div()
                .id(id)
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(0x3b3b5c))
                .text_color(white())
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x52527a)))
                .child(label.into())
                .on_click(cx.listener(move |this, _, window, cx| handler(this, window, cx)))
        }

        fn set_zone(&mut self, zone: Pixels, window: &mut Window, cx: &mut Context<Self>) {
            self.zone = zone;
            window.set_exclusive_zone(zone);
            self.status = format!("set exclusive zone to {}", f32::from(zone)).into();
            cx.notify();
        }
    }

    impl Render for ExclusiveExample {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .bg(rgba(0x1e1e2eee))
                .text_color(white())
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::BOLD)
                        .child("Layer shell exclusive zone / edge"),
                )
                .child(div().child(format!("exclusive zone = {}px", f32::from(self.zone))))
                .child(div().text_color(rgb(0xaaaacc)).child(self.status.clone()))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Self::button(
                            "zone-inc",
                            "Zone +10",
                            cx,
                            |this, window, cx| {
                                this.set_zone(this.zone + px(10.), window, cx);
                            },
                        ))
                        .child(Self::button(
                            "zone-dec",
                            "Zone -10",
                            cx,
                            |this, window, cx| {
                                this.set_zone(this.zone - px(10.), window, cx);
                            },
                        ))
                        .child(Self::button(
                            "zone-extend",
                            "Zone -1 (extend under)",
                            cx,
                            |this, window, cx| this.set_zone(px(-1.), window, cx),
                        ))
                        .child(Self::button(
                            "zone-zero",
                            "Zone 0",
                            cx,
                            |this, window, cx| {
                                this.set_zone(px(0.), window, cx);
                            },
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Self::button(
                            "edge-top",
                            "Edge TOP",
                            cx,
                            |this, window, cx| {
                                window.set_exclusive_edge(Anchor::TOP);
                                this.status = "set exclusive edge TOP".into();
                                cx.notify();
                            },
                        ))
                        .child(Self::button(
                            "edge-left",
                            "Edge LEFT",
                            cx,
                            |this, window, cx| {
                                window.set_exclusive_edge(Anchor::LEFT);
                                this.status = "set exclusive edge LEFT".into();
                                cx.notify();
                            },
                        ))
                        .child(Self::button(
                            "edge-bottom",
                            "Edge BOTTOM (invalid)",
                            cx,
                            |this, window, cx| {
                                window.set_exclusive_edge(Anchor::BOTTOM);
                                this.status =
                                    "tried exclusive edge BOTTOM, ignored (see log)".into();
                                cx.notify();
                            },
                        )),
                )
        }
    }

    pub fn main() {
        env_logger::init();
        application().run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    titlebar: None,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: Size::new(px(600.), px(260.)),
                    })),
                    app_id: Some("gpui-layer-shell-exclusive".to_string()),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: "gpui".to_string(),
                        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                        exclusive_zone: Some(px(50.)),
                        // Valid: TOP is one of the anchors. Set this to Anchor::BOTTOM
                        // to watch the creation-path guard log and ignore it.
                        exclusive_edge: Some(Anchor::TOP),
                        keyboard_interactivity: KeyboardInteractivity::None,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| ExclusiveExample {
                        zone: px(50.),
                        status: "ready".into(),
                    })
                },
            )
            .unwrap();
        });
    }
}
