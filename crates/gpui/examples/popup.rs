//! Example and manual test for platform-native popups (`WindowKind::Popup`).
//!
//! A native popup is a real, parent-anchored window that can extend beyond its parent onto the
//! screen, unlike gpui's in-window popovers. Only some platforms implement it so far, so this also
//! works as a check for a new platform implementation. Run it, open the menu, and confirm the
//! points listed in the window. On a platform without an implementation the button reports that
//! popups are not supported instead of opening anything.
//!
//! Run with: cargo run -p gpui --example popup
//!
//! Two notes on grabbing popups (see `PopupOptions::grab`):
//! - They are opened from `on_mouse_down`, not `on_click`. The grab must be taken while the
//!   triggering button is still held, or the compositor declines it.
//! - Auto-dismissal only covers clicks in other applications, so this example dismisses the menu
//!   itself when the user clicks elsewhere in its own window.

#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, MouseButton, SharedString, Window, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, div, point, popup::*, prelude::*, px, rgb, size,
};
use gpui_platform::application;

/// The trigger button, at a fixed position so the popup can anchor to a known rectangle. Real code
/// would anchor to the measured bounds of whatever element opens the popup.
const BUTTON_BOUNDS: Bounds<gpui::Pixels> = Bounds {
    origin: point(px(24.), px(24.)),
    size: size(px(200.), px(32.)),
};

const POPUP_SIZE: gpui::Size<gpui::Pixels> = size(px(260.), px(320.));

struct Menu;

impl Render for Menu {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let item = |label: &str| {
            div()
                .id(label.to_string())
                .px_3()
                .py_1()
                .rounded_sm()
                .hover(|this| this.bg(rgb(0x3a3a3a)))
                .cursor_pointer()
                .child(label.to_string())
                .on_click(|_, window, _| window.remove_window())
        };

        div()
            .id("menu-root")
            .size_full()
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .bg(rgb(0x2a2a2a))
            .text_color(gpui::white())
            .rounded_md()
            .border_1()
            .border_color(rgb(0x454545))
            .child(item("Foo"))
            .child(item("Bar"))
            .child(item("Baz"))
            .child(item("Qux"))
            .child(item("Alice"))
            .child(item("Bob"))
    }
}

struct PopupExample {
    menu: Option<WindowHandle<Menu>>,
    status: SharedString,
}

impl Default for PopupExample {
    fn default() -> Self {
        Self {
            menu: None,
            status: "Click \"Open menu\" to open a native popup.".into(),
        }
    }
}

impl PopupExample {
    /// Closes the menu if it is open. Returns true if a menu was actually open.
    fn close_menu(&mut self, cx: &mut App) -> bool {
        match self.menu.take() {
            Some(menu) => menu
                .update(cx, |_, window, _| window.remove_window())
                .is_ok(),
            None => false,
        }
    }

    fn toggle_menu(&mut self, cx: &mut App) {
        if self.close_menu(cx) {
            return;
        }
        match open_menu(cx) {
            Ok(menu) => {
                self.menu = Some(menu);
                self.status = "Menu open. Dismiss it by selecting an item, clicking elsewhere in \
                    this window, or clicking another application."
                    .into();
            }
            // A real application would fall back to an in-window popover on `PopupNotSupportedError`.
            Err(error) => {
                self.status =
                    format!("Native popups are not supported on this platform yet: {error}").into();
                log::error!("failed to open popup: {error}");
            }
        }
    }
}

impl Render for PopupExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bullet = |text: &str| div().child(format!("• {text}"));

        div()
            .id("root")
            .size_full()
            .bg(rgb(0xf7f7f7))
            .text_color(rgb(0x222222))
            // Same-app clicks are our responsibility to dismiss on (see `PopupOptions::grab`).
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _window, cx| {
                    this.close_menu(cx);
                }),
            )
            .child(
                div()
                    .size_full()
                    .p_5()
                    .pt(px(76.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_xl().child("Native popup test"))
                    .child(div().text_sm().child(
                        "WindowKind::Popup opens a real, parent-anchored window that can extend \
                         past this window onto the screen. Only some platforms implement it so far.",
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_sm()
                            .text_color(rgb(0x555555))
                            .child(div().child("Verify:"))
                            .child(bullet("The menu opens anchored below the button."))
                            .child(bullet("The menu extends past the bottom edge of this window."))
                            .child(bullet(
                                "Near the bottom of the screen, the menu flips above the button.",
                            ))
                            .child(bullet("Clicking another application dismisses the menu."))
                            .child(bullet(
                                "Selecting an item or clicking in this window dismisses it.",
                            )),
                    )
                    .child(div().text_sm().text_color(rgb(0x333333)).child(self.status.clone())),
            )
            .child(
                div()
                    .absolute()
                    .left(BUTTON_BOUNDS.origin.x)
                    .top(BUTTON_BOUNDS.origin.y)
                    .w(BUTTON_BOUNDS.size.width)
                    .h(BUTTON_BOUNDS.size.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(0xd0d0d0))
                    .rounded_md()
                    .cursor_pointer()
                    .id("open-menu")
                    .active(|this| this.bg(rgb(0xeeeeee)))
                    .child("Open menu ▾")
                    // Open on mouse-down, not on click, so the grab is taken while the button is
                    // still held.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _window, cx| {
                            // Don't let the window handler above close the menu we are opening.
                            cx.stop_propagation();
                            this.toggle_menu(cx);
                        }),
                    ),
            )
    }
}

fn open_menu(cx: &mut App) -> anyhow::Result<WindowHandle<Menu>> {
    cx.open_window(
        WindowOptions {
            titlebar: None,
            // The initial bounds size the surface. The platform decides the final position from the
            // `PopupOptions` below.
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: POPUP_SIZE,
            })),
            kind: WindowKind::Popup(PopupOptions {
                anchor_rect: BUTTON_BOUNDS,
                size: POPUP_SIZE,
                // Anchor to the button's bottom-left and grow down-right so the menu drops beneath it.
                anchor: PopupAnchor::BottomLeft,
                gravity: PopupGravity::BottomRight,
                // Slide horizontally and flip vertically if the menu would leave the screen.
                constraint_adjustment: PopupConstraintAdjustment::SLIDE_X
                    | PopupConstraintAdjustment::FLIP_Y,
                offset: point(px(0.), px(4.)),
                // Grab input so the compositor dismisses the popup on clicks into other applications.
                grab: true,
            }),
            ..Default::default()
        },
        |_, cx| cx.new(|_| Menu),
    )
}

fn run_example() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(100.), px(100.)),
                    size: size(px(420.), px(300.)),
                })),
                ..Default::default()
            },
            |_, cx| cx.new(|_| PopupExample::default()),
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
