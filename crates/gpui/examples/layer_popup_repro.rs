//! Scratch repro: AnchoredPopup whose parent is a layer-shell bar, mimicking phrame's
//! tray menu. Auto-opens the popup 800ms after launch, closes itself after 3s.
//!
//! Run with: WAYLAND_DEBUG=1 cargo run -p gpui --example layer_popup_repro

#![cfg_attr(target_family = "wasm", no_main)]

use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Bounds, Context, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, div, layer_shell::*, point, popup::*, prelude::*, px, rgb, size,
};
use gpui_platform::application;

const BAR_SIZE: f32 = 32.0;

struct Bar;

impl Render for Bar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x224477))
    }
}

struct Menu;

impl Render for Menu {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x2a2a2a))
            .text_color(gpui::white())
            .child("menu")
    }
}

fn open_menu(parent: AnyWindowHandle, cx: &mut App) {
    // Mirrors phrame's bar_popup(Top, 32.0, 500.0, parent, true) + open_bare(..., 210x86, ...).
    let result = cx.open_window(
        WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(210.0), px(86.0)),
            })),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::AnchoredPopup(PopupOptions {
                parent,
                anchor_rect: Bounds {
                    origin: point(px(500.0), px(BAR_SIZE)),
                    size: size(px(1.0), px(1.0)),
                },
                anchor: PopupAnchor::BottomLeft,
                gravity: PopupGravity::BottomRight,
                constraint_adjustment: PopupConstraintAdjustment::SLIDE_X
                    | PopupConstraintAdjustment::FLIP_X,
                offset: point(px(0.0), px(0.0)),
                grab: true,
            }),
            ..Default::default()
        },
        |_, cx| cx.new(|_| Menu),
    );
    match result {
        Ok(_) => log::info!("popup opened"),
        Err(err) => log::error!("popup failed: {err}"),
    }
}

fn main() {
    env_logger::init();
    application().run(|cx: &mut App| {
        let bar = cx
            .open_window(
                WindowOptions {
                    titlebar: None,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size: size(px(1.0), px(BAR_SIZE)),
                    })),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: "phrame-repro".into(),
                        layer: Layer::Top,
                        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                        exclusive_zone: Some(px(BAR_SIZE)),
                        exclusive_edge: Some(Anchor::TOP),
                        keyboard_interactivity: KeyboardInteractivity::None,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Bar),
            )
            .expect("failed to open bar");

        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = cx.update(|cx| open_menu(bar.into(), cx));
            cx.background_executor()
                .timer(Duration::from_millis(2200))
                .await;
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
