#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, ColorSpace, Context, Half, Render, Window, WindowOptions, canvas, div,
    linear_color_stop, linear_gradient, point, prelude::*, px, size,
};
use gpui_platform::application;

struct GradientViewer {
    color_space: ColorSpace,
}

impl GradientViewer {
    fn new() -> Self {
        Self {
            color_space: ColorSpace::default(),
        }
    }
}

impl Render for GradientViewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color_space = self.color_space;
        let gradient = |background: gpui::Background| {
            div()
                .flex_1()
                .rounded_xl()
                .bg(background.color_space(color_space))
        };

        div()
            .bg(gpui::white())
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_between()
                    .items_center()
                    .child("Gradient Examples")
                    .child(
                        div().flex().gap_2().items_center().child(
                            div()
                                .id("method")
                                .flex()
                                .px_3()
                                .py_1()
                                .text_sm()
                                .bg(gpui::black())
                                .text_color(gpui::white())
                                .child(format!("{}", color_space))
                                .active(|this| this.opacity(0.8))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.color_space = match this.color_space {
                                        ColorSpace::Oklab => ColorSpace::Srgb,
                                        ColorSpace::Srgb => ColorSpace::Oklab,
                                    };
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .child(
                        div()
                            .size_full()
                            .rounded_xl()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(gpui::red())
                            .text_color(gpui::white())
                            .child("Solid Color"),
                    )
                    .child(
                        div()
                            .size_full()
                            .rounded_xl()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(gpui::blue())
                            .text_color(gpui::white())
                            .child("Solid Color"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .h_24()
                    .text_color(gpui::white())
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            45.,
                            linear_color_stop(gpui::red(), 0.),
                            linear_color_stop(gpui::blue(), 1.),
                        )
                        .color_space(color_space)),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            135.,
                            linear_color_stop(gpui::red(), 0.),
                            linear_color_stop(gpui::green(), 1.),
                        )
                        .color_space(color_space)),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            225.,
                            linear_color_stop(gpui::green(), 0.),
                            linear_color_stop(gpui::blue(), 1.),
                        )
                        .color_space(color_space)),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            315.,
                            linear_color_stop(gpui::green(), 0.),
                            linear_color_stop(gpui::yellow(), 1.),
                        )
                        .color_space(color_space)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .h_24()
                    .text_color(gpui::white())
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            0.,
                            linear_color_stop(gpui::red(), 0.),
                            linear_color_stop(gpui::white(), 1.),
                        )
                        .color_space(color_space)),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            90.,
                            linear_color_stop(gpui::blue(), 0.),
                            linear_color_stop(gpui::white(), 1.),
                        )
                        .color_space(color_space)),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            180.,
                            linear_color_stop(gpui::green(), 0.),
                            linear_color_stop(gpui::white(), 1.),
                        )
                        .color_space(color_space)),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            360.,
                            linear_color_stop(gpui::yellow(), 0.),
                            linear_color_stop(gpui::white(), 1.),
                        )
                        .color_space(color_space)),
                    ),
            )
            .child(
                div().flex_1().rounded_xl().bg(linear_gradient(
                    0.,
                    linear_color_stop(gpui::green(), 0.05),
                    linear_color_stop(gpui::yellow(), 0.95),
                )
                .color_space(color_space)),
            )
            .child(
                div().flex_1().rounded_xl().bg(linear_gradient(
                    90.,
                    linear_color_stop(gpui::blue(), 0.05),
                    linear_color_stop(gpui::red(), 0.95),
                )
                .color_space(color_space)),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .child(
                        div().flex().flex_1().gap_3().child(
                            div().flex_1().rounded_xl().bg(linear_gradient(
                                90.,
                                linear_color_stop(gpui::blue(), 0.5),
                                linear_color_stop(gpui::red(), 0.5),
                            )
                            .color_space(color_space)),
                        ),
                    )
                    .child(
                        div().flex_1().rounded_xl().bg(linear_gradient(
                            180.,
                            linear_color_stop(gpui::green(), 0.),
                            linear_color_stop(gpui::blue(), 0.5),
                        )
                        .color_space(color_space)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_3()
                    .child(gradient(gpui::linear_gradient_stops(
                        90.,
                        [
                            linear_color_stop(gpui::red(), 0.),
                            linear_color_stop(gpui::yellow(), 0.25),
                            linear_color_stop(gpui::green(), 0.5),
                            linear_color_stop(gpui::blue(), 0.75),
                            linear_color_stop(gpui::red(), 1.),
                        ],
                    )))
                    .child(gradient(gpui::radial_gradient(
                        point(0.5, 0.5),
                        size(0.5, 0.5),
                        [
                            linear_color_stop(gpui::yellow(), 0.),
                            linear_color_stop(gpui::red(), 0.6),
                            linear_color_stop(gpui::blue(), 1.),
                        ],
                    )))
                    .child(gradient(gpui::conic_gradient(
                        point(0.5, 0.5),
                        0.,
                        [
                            linear_color_stop(gpui::red(), 0.),
                            linear_color_stop(gpui::yellow(), 1. / 6.),
                            linear_color_stop(gpui::green(), 2. / 6.),
                            linear_color_stop(gpui::rgb(0x00ffff), 3. / 6.),
                            linear_color_stop(gpui::blue(), 4. / 6.),
                            linear_color_stop(gpui::rgb(0xff00ff), 5. / 6.),
                            linear_color_stop(gpui::red(), 1.),
                        ],
                    ))),
            )
            .child(
                // Two full hue cycles from 13 stops.
                gradient(gpui::linear_gradient_stops(
                    90.,
                    (0..=12).map(|index| {
                        linear_color_stop(
                            match index % 6 {
                                0 => gpui::red(),
                                1 => gpui::yellow(),
                                2 => gpui::green(),
                                3 => gpui::rgb(0x00ffff).into(),
                                4 => gpui::blue(),
                                _ => gpui::rgb(0xff00ff).into(),
                            },
                            index as f32 / 12.,
                        )
                    }),
                )),
            )
            .child(div().h_24().child(canvas(
                move |_, _, _| {},
                move |bounds, _, window, _| {
                    let size = size(bounds.size.width * 0.8, px(80.));
                    let square_bounds = Bounds {
                        origin: point(
                            bounds.size.width.half() - size.width.half(),
                            bounds.origin.y,
                        ),
                        size,
                    };
                    let height = square_bounds.size.height;
                    let horizontal_offset = height;
                    let vertical_offset = px(30.);
                    let mut builder = gpui::PathBuilder::fill();
                    builder.move_to(square_bounds.bottom_left());
                    builder
                        .line_to(square_bounds.origin + point(horizontal_offset, vertical_offset));
                    builder.line_to(
                        square_bounds.top_right() + point(-horizontal_offset, vertical_offset),
                    );

                    builder.line_to(square_bounds.bottom_right());
                    builder.line_to(square_bounds.bottom_left());
                    let path = builder.build().unwrap();
                    window.paint_path(
                        path,
                        linear_gradient(
                            180.,
                            linear_color_stop(gpui::red(), 0.),
                            linear_color_stop(gpui::blue(), 1.),
                        )
                        .color_space(color_space),
                    );
                },
            )))
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                focus: true,
                ..Default::default()
            },
            |_, cx| cx.new(|_| GradientViewer::new()),
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
