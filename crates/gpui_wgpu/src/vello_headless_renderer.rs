use crate::{
    WgpuContext,
    vello_scene::{VelloResourceCache, rebuild_vello_scene},
};
use anyhow::{Context as _, Result};
use gpui::{DevicePixels, PlatformHeadlessRenderer, Scene, Size};
use image::RgbaImage;
use parking_lot::Mutex;
use std::sync::{Arc, mpsc};

struct HeadlessTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

pub struct VelloHeadlessRenderer {
    context: WgpuContext,
    renderer: vello::Renderer,
    scene: vello::Scene,
    resource_cache: VelloResourceCache,
    target: Option<HeadlessTarget>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl VelloHeadlessRenderer {
    pub fn new() -> Result<Self> {
        let context = WgpuContext::new_headless()?;
        let renderer = vello::Renderer::new(
            &context.device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(|error| anyhow::anyhow!("failed to create headless Vello renderer: {error}"))?;
        let last_error = Arc::new(Mutex::new(None));
        context.device.on_uncaptured_error({
            let last_error = Arc::clone(&last_error);
            Arc::new(move |error| {
                *last_error.lock() = Some(error.to_string());
            })
        });

        Ok(Self {
            context,
            renderer,
            scene: vello::Scene::new(),
            resource_cache: VelloResourceCache::default(),
            target: None,
            last_error,
        })
    }

    fn dimensions(&self, size: Size<DevicePixels>) -> Result<(u32, u32)> {
        anyhow::ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "invalid headless render size: {size:?}"
        );
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let max_texture_size = self.context.device.limits().max_texture_dimension_2d;
        anyhow::ensure!(
            width <= max_texture_size && height <= max_texture_size,
            "headless render size {width}x{height} exceeds maximum texture dimension {max_texture_size}"
        );
        Ok((width, height))
    }

    fn ensure_target(&mut self, width: u32, height: u32) {
        let target_matches = self
            .target
            .as_ref()
            .map(|target| target.width == width && target.height == height)
            .unwrap_or(false);
        if target_matches {
            return;
        }

        let texture = self
            .context
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("vello_headless_target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some(HeadlessTarget {
            texture,
            view,
            width,
            height,
        });
    }

    fn take_gpu_error(&self) -> Result<()> {
        if let Some(error) = self.last_error.lock().take() {
            anyhow::bail!("GPU error during headless Vello rendering: {error}");
        }
        Ok(())
    }

    fn render_to_target(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<(u32, u32)> {
        self.take_gpu_error()?;
        anyhow::ensure!(
            !self.context.device_lost(),
            "headless Vello GPU device was lost"
        );
        let (width, height) = self.dimensions(size)?;
        self.ensure_target(width, height);

        let scene_build = rebuild_vello_scene(&mut self.scene, &mut self.resource_cache, scene);
        if !scene_build.unsupported.is_empty() {
            log::debug!(
                "Headless Vello frame omitted unsupported GPUI primitives: {:?}",
                scene_build.unsupported
            );
        }

        let target = self
            .target
            .as_ref()
            .context("headless Vello target was not initialized")?;
        self.renderer
            .render_to_texture(
                &self.context.device,
                &self.context.queue,
                &self.scene,
                &target.view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::TRANSPARENT,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .map_err(|error| {
                anyhow::anyhow!("Vello failed to render headless GPUI scene: {error}")
            })?;

        Ok((width, height))
    }

    fn read_target(&self, width: u32, height: u32) -> Result<RgbaImage> {
        let unpadded_bytes_per_row = width
            .checked_mul(4)
            .context("headless image row byte count overflowed")?;
        let padded_bytes_per_row = unpadded_bytes_per_row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .checked_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .context("aligned headless image row byte count overflowed")?;
        let buffer_size = u64::from(padded_bytes_per_row)
            .checked_mul(u64::from(height))
            .context("headless image readback buffer size overflowed")?;
        let readback = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vello_headless_readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let target = self
            .target
            .as_ref()
            .context("headless Vello target was not initialized")?;
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("vello_headless_readback_encoder"),
                });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.context.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            if sender.send(result).is_err() {
                log::error!(
                    "headless Vello readback receiver was dropped before mapping completed"
                );
            }
        });
        self.context
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .context("failed while waiting for headless Vello readback")?;
        receiver
            .recv()
            .context("headless Vello readback callback was not delivered")?
            .context("failed to map headless Vello readback buffer")?;
        self.take_gpu_error()?;

        let mapped = slice.get_mapped_range();
        let output_size = usize::try_from(unpadded_bytes_per_row)
            .ok()
            .and_then(|row| row.checked_mul(height as usize))
            .context("headless image output size overflowed")?;
        let mut pixels = Vec::with_capacity(output_size);
        for row in mapped
            .chunks_exact(padded_bytes_per_row as usize)
            .take(height as usize)
        {
            pixels.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
        }
        drop(mapped);
        readback.unmap();

        RgbaImage::from_raw(width, height, pixels)
            .context("failed to construct RGBA image from headless Vello pixels")
    }
}

impl PlatformHeadlessRenderer for VelloHeadlessRenderer {
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<RgbaImage> {
        let (width, height) = self.render_to_target(scene, size)?;
        self.read_target(width, height)
    }

    fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<()> {
        self.render_to_target(scene, size)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Background, BorderStyle, Bounds, ContentMask, Corners, Edges, FontRun, Hsla, Path,
        PlatformTextSystem as _, Quad, RenderImage, ScaledPixels, Shadow, TransformationMatrix,
        VectorGlyph, VectorGlyphRun, VectorImage, VectorSvg, checkerboard, font, pattern_slash,
        point, px, rgb, size, white,
    };
    use image::{Frame, RgbaImage};
    use smallvec::smallvec;
    use std::{borrow::Cow, sync::Arc};

    fn scaled_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: point(ScaledPixels(x), ScaledPixels(y)),
            size: size(ScaledPixels(width), ScaledPixels(height)),
        }
    }

    fn content_mask(bounds: Bounds<ScaledPixels>) -> ContentMask<ScaledPixels> {
        ContentMask { bounds }
    }

    fn quad(order: u32, bounds: Bounds<ScaledPixels>, color: impl Into<Background>) -> Quad {
        Quad {
            order,
            bounds,
            content_mask: content_mask(bounds),
            background: color.into(),
            ..Default::default()
        }
    }

    fn image_from_bgra(pixel: [u8; 4]) -> Arc<RenderImage> {
        let pixels = RgbaImage::from_raw(1, 1, pixel.to_vec())
            .expect("one BGRA pixel should make a one-pixel image");
        Arc::new(RenderImage::new(smallvec![Frame::new(pixels)]))
    }

    fn assert_pixel_near(image: &RgbaImage, x: u32, y: u32, expected: [u8; 4], tolerance: u8) {
        let actual = image
            .get_pixel_checked(x, y)
            .expect("test pixel should be inside the rendered image")
            .0;
        for (actual_channel, expected_channel) in actual.into_iter().zip(expected) {
            assert!(
                actual_channel.abs_diff(expected_channel) <= tolerance,
                "pixel ({x}, {y}) was {:?}, expected {:?} within {tolerance}",
                image.get_pixel(x, y).0,
                expected
            );
        }
    }

    #[test]
    fn dashed_borders_honor_enabled_sides() -> Result<()> {
        let mut renderer = VelloHeadlessRenderer::new()?;
        let bounds = scaled_bounds(0.0, 0.0, 24.0, 16.0);
        let mut scene = Scene::default();
        scene.quads.push(Quad {
            order: 1,
            bounds,
            content_mask: content_mask(bounds),
            border_color: Hsla::from(rgb(0xff0000)),
            border_widths: Edges {
                top: ScaledPixels(2.0),
                right: ScaledPixels(0.0),
                bottom: ScaledPixels(0.0),
                left: ScaledPixels(0.0),
            },
            border_style: BorderStyle::Dashed,
            ..Default::default()
        });
        scene.finish();

        let image =
            renderer.render_scene_to_image(&scene, size(DevicePixels(24), DevicePixels(16)))?;
        let top_coverage = image
            .enumerate_pixels()
            .filter(|(_, y, pixel)| *y < 3 && pixel.0[3] > 0x80)
            .count();
        let disabled_side_coverage = image
            .enumerate_pixels()
            .filter(|(_, y, pixel)| *y >= 4 && pixel.0[3] > 1)
            .count();

        assert!(top_coverage > 0, "top dashed border was not rendered");
        assert_eq!(
            disabled_side_coverage, 0,
            "disabled dashed border sides rendered pixels"
        );
        Ok(())
    }

    #[test]
    fn dense_alternating_text_renders_at_large_target_size() -> Result<()> {
        const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");

        let text_system = crate::ParleyTextSystem::new_without_system_fonts("Lilex");
        text_system.add_fonts(vec![Cow::Borrowed(LILEX)])?;
        let font_id = text_system.font_id(&font("Lilex"))?;
        let layout = text_system.layout_line("A", px(14.0), &[FontRun { len: 1, font_id }]);
        let shaped_glyph = layout
            .runs
            .first()
            .and_then(|run| run.glyphs.first())
            .context("test font should shape one glyph")?;
        let render_data = text_system
            .font_render_data(font_id)
            .context("test font should provide Vello render data")?;
        let target_bounds = scaled_bounds(0.0, 0.0, 2560.0, 1440.0);
        let target_mask = content_mask(target_bounds);
        let mut scene = Scene::default();

        for row in 0_usize..70 {
            for column in 0_usize..170 {
                let x = 8.0 + column as f32 * 14.0;
                let y = 18.0 + row as f32 * 18.0;
                scene.glyph_runs.push(VectorGlyphRun {
                    order: 1,
                    bounds: scaled_bounds(x, y - 14.0, 14.0, 18.0),
                    content_mask: target_mask,
                    color: if column.is_multiple_of(2) {
                        Hsla::from(rgb(0xffffff))
                    } else {
                        Hsla::from(rgb(0x60a0ff))
                    },
                    font: render_data.font.clone(),
                    font_size: ScaledPixels(14.0),
                    normalized_coords: render_data.normalized_coords.clone(),
                    glyphs: Arc::from([VectorGlyph {
                        id: shaped_glyph.id,
                        position: point(ScaledPixels(x), ScaledPixels(y)),
                    }]),
                });
            }
        }
        scene.finish();

        let mut renderer = VelloHeadlessRenderer::new()?;
        let image =
            renderer.render_scene_to_image(&scene, size(DevicePixels(2560), DevicePixels(1440)))?;
        let visible_pixels = image.pixels().filter(|pixel| pixel.0[3] > 0x80).count();

        assert!(
            visible_pixels > 10_000,
            "dense Vello text scene rendered too few visible pixels: {visible_pixels}"
        );
        Ok(())
    }

    #[test]
    fn renders_scenes_to_rgba_images() -> Result<()> {
        let mut renderer = VelloHeadlessRenderer::new()?;
        let full = scaled_bounds(0.0, 0.0, 8.0, 4.0);
        let mut scene = Scene::default();
        scene.quads.push(quad(1, full, rgb(0xff0000)));
        scene.vector_images.push(VectorImage {
            order: 2,
            bounds: full,
            content_mask: content_mask(scaled_bounds(2.0, 0.0, 4.0, 4.0)),
            corner_radii: Corners::default(),
            image: image_from_bgra([0xff, 0x00, 0x00, 0xff]),
            frame_index: 0,
            grayscale: false,
            opacity: 1.0,
        });
        scene
            .quads
            .push(quad(3, scaled_bounds(4.0, 0.0, 4.0, 4.0), rgb(0x00ff00)));
        scene.finish();

        let image =
            renderer.render_scene_to_image(&scene, size(DevicePixels(8), DevicePixels(4)))?;
        assert_eq!(image.dimensions(), (8, 4));
        assert_pixel_near(&image, 1, 2, [0xff, 0x00, 0x00, 0xff], 1);
        assert_pixel_near(&image, 3, 2, [0x00, 0x00, 0xff, 0xff], 1);
        assert_pixel_near(&image, 6, 2, [0x00, 0xff, 0x00, 0xff], 1);

        let mut image_scene = Scene::default();
        image_scene.vector_images.push(VectorImage {
            order: 1,
            bounds: scaled_bounds(0.0, 0.0, 8.0, 8.0),
            content_mask: content_mask(scaled_bounds(0.0, 0.0, 8.0, 8.0)),
            corner_radii: Corners::all(ScaledPixels(4.0)),
            image: image_from_bgra([0x00, 0x00, 0xff, 0xff]),
            frame_index: 0,
            grayscale: true,
            opacity: 0.5,
        });
        image_scene.finish();
        let image =
            renderer.render_scene_to_image(&image_scene, size(DevicePixels(8), DevicePixels(8)))?;
        let center = image.get_pixel(4, 4).0;
        assert_eq!(center[0], center[1]);
        assert_eq!(center[1], center[2]);
        assert!(
            center[0] >= 20,
            "grayscale image center was too dark: {center:?}"
        );
        assert!(
            (120..=136).contains(&center[3]),
            "image opacity was not applied: {center:?}"
        );
        assert!(
            image.get_pixel(0, 0).0[3] < center[3] / 2,
            "rounded image corner was not clipped"
        );

        let tree = usvg::Tree::from_str(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="2" height="4" fill="#000"/></svg>"##,
            &usvg::Options::default(),
        )?;
        let mut svg_scene = Scene::default();
        svg_scene.vector_svgs.push(VectorSvg {
            order: 1,
            bounds: full,
            content_mask: content_mask(full),
            tree: Arc::new(tree),
            color: Hsla::from(rgb(0x00ff00)),
            transformation: TransformationMatrix::unit(),
        });
        svg_scene.finish();
        let image =
            renderer.render_scene_to_image(&svg_scene, size(DevicePixels(8), DevicePixels(4)))?;
        assert_pixel_near(&image, 1, 2, [0x00, 0xff, 0x00, 0xff], 1);
        assert!(image.get_pixel(6, 2).0[3] <= 1);

        let mut quadratic = Path::new(point(px(0.0), px(8.0)));
        quadratic.curve_to(point(px(8.0), px(8.0)), point(px(4.0), px(0.0)));
        quadratic.content_mask = ContentMask {
            bounds: Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(8.0), px(8.0)),
            },
        };
        quadratic.color = rgb(0xff0000).into();
        let mut path_scene = Scene::default();
        path_scene.paths.push(quadratic.scale(1.0));
        path_scene.finish();
        let image =
            renderer.render_scene_to_image(&path_scene, size(DevicePixels(8), DevicePixels(8)))?;
        assert!(
            image.get_pixel(4, 1).0[3] <= 1,
            "quadratic control triangle leaked above the curve"
        );
        assert_pixel_near(&image, 4, 6, [0xff, 0x00, 0x00, 0xff], 1);

        let mut pattern_scene = Scene::default();
        pattern_scene.quads.push(Quad {
            order: 1,
            bounds: scaled_bounds(0.0, 0.0, 16.0, 8.0),
            content_mask: content_mask(scaled_bounds(0.0, 0.0, 16.0, 8.0)),
            background: checkerboard(rgb(0x0000ff), 4.0),
            border_color: white(),
            border_widths: Edges::all(ScaledPixels(1.0)),
            ..Default::default()
        });
        pattern_scene.quads.push(quad(
            2,
            scaled_bounds(0.0, 8.0, 16.0, 8.0),
            pattern_slash(rgb(0xff0000), 2.0, 6.0),
        ));
        pattern_scene.finish();
        let image = renderer
            .render_scene_to_image(&pattern_scene, size(DevicePixels(16), DevicePixels(16)))?;
        assert!(image.get_pixel(2, 2).0[3] <= 1);
        assert_pixel_near(&image, 6, 2, [0x00, 0x00, 0xff, 0xff], 1);
        assert!(
            image
                .get_pixel(0, 4)
                .0
                .iter()
                .all(|channel| *channel >= 0xfe)
        );
        let slash_coverage = image
            .enumerate_pixels()
            .filter(|(_, y, pixel)| *y >= 8 && pixel.0[3] > 0x80)
            .count();
        assert!(
            (8..120).contains(&slash_coverage),
            "slash pattern coverage was implausible: {slash_coverage}"
        );

        let full_shadow_mask = content_mask(scaled_bounds(0.0, 0.0, 16.0, 8.0));
        let mut shadow_scene = Scene::default();
        shadow_scene.shadows.push(Shadow {
            order: 1,
            blur_radius: ScaledPixels(0.0),
            bounds: scaled_bounds(10.0, 0.0, 4.0, 4.0),
            corner_radii: Corners::default(),
            content_mask: full_shadow_mask,
            color: Hsla::from(rgb(0x0000ff)),
            element_bounds: scaled_bounds(0.0, 0.0, 4.0, 4.0),
            element_corner_radii: Corners::default(),
            inset: false,
        });
        shadow_scene.shadows.push(Shadow {
            order: 2,
            blur_radius: ScaledPixels(0.0),
            bounds: scaled_bounds(2.0, 2.0, 4.0, 4.0),
            corner_radii: Corners::default(),
            content_mask: full_shadow_mask,
            color: Hsla::from(rgb(0xff0000)),
            element_bounds: scaled_bounds(0.0, 0.0, 8.0, 8.0),
            element_corner_radii: Corners::default(),
            inset: true,
        });
        shadow_scene.finish();
        let image = renderer
            .render_scene_to_image(&shadow_scene, size(DevicePixels(16), DevicePixels(8)))?;
        assert_pixel_near(&image, 12, 2, [0x00, 0x00, 0xff, 0xff], 1);
        assert_pixel_near(&image, 1, 4, [0xff, 0x00, 0x00, 0xff], 1);
        assert!(
            image.get_pixel(4, 4).0[3] <= 1,
            "hard inset shadow did not remove its inner hole"
        );

        renderer.render_scene(&svg_scene, size(DevicePixels(17), DevicePixels(5)))?;
        assert!(
            renderer
                .render_scene(&svg_scene, size(DevicePixels(0), DevicePixels(5)))
                .is_err()
        );
        Ok(())
    }

    mod gpui_integration {
        use super::*;
        use gpui::{
            AnyWindowHandle, HeadlessAppContext, InputEvent, MouseButton, MouseDownEvent,
            MouseUpEvent, NoopTextSystem, Render, div, point, prelude::*, px,
        };

        fn headless_context() -> HeadlessAppContext {
            HeadlessAppContext::with_platform(Arc::new(NoopTextSystem), Arc::new(()), || {
                VelloHeadlessRenderer::new()
                    .map(|renderer| Box::new(renderer) as Box<dyn PlatformHeadlessRenderer>)
                    .map_err(|error| {
                        log::error!("failed to create test Vello headless renderer: {error}");
                        error
                    })
                    .ok()
            })
        }

        fn simulate_event<E: InputEvent>(
            cx: &mut HeadlessAppContext,
            window: AnyWindowHandle,
            event: E,
        ) -> Result<()> {
            cx.update_window(window, |_, window, cx| {
                window.dispatch_event(event.to_platform_input(), cx);
            })?;
            cx.run_until_parked();
            Ok(())
        }

        fn simulate_click(
            cx: &mut HeadlessAppContext,
            window: AnyWindowHandle,
            position: gpui::Point<gpui::Pixels>,
        ) -> Result<()> {
            simulate_event(
                cx,
                window,
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                },
            )?;
            simulate_event(
                cx,
                window,
                MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                },
            )
        }

        fn redraw(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> Result<()> {
            cx.update_window(window, |_, window, cx| {
                window.draw(cx).clear();
            })?;
            Ok(())
        }

        struct ClickableColorView {
            clicked: bool,
        }

        impl Render for ClickableColorView {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                cx: &mut gpui::Context<Self>,
            ) -> impl IntoElement {
                let background = if self.clicked {
                    rgb(0x20c040)
                } else {
                    rgb(0xd02020)
                };
                div()
                    .id("click-target")
                    .size_full()
                    .bg(background)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clicked = true;
                        cx.notify();
                    }))
            }
        }

        #[test]
        fn clicking_invalidates_and_redraws_vello_pixels() -> Result<()> {
            let mut cx = headless_context();
            let window = cx.open_window(size(px(16.0), px(12.0)), |_window, cx| {
                cx.new(|_| ClickableColorView { clicked: false })
            })?;
            let window: AnyWindowHandle = window.into();

            redraw(&mut cx, window)?;
            let before = cx.capture_screenshot(window)?;
            assert_eq!(before.dimensions(), (32, 24));
            assert_pixel_near(&before, 16, 12, [0xd0, 0x20, 0x20, 0xff], 2);

            simulate_click(&mut cx, window, point(px(8.0), px(6.0)))?;
            redraw(&mut cx, window)?;
            let after = cx.capture_screenshot(window)?;
            assert_pixel_near(&after, 16, 12, [0x20, 0xc0, 0x40, 0xff], 2);
            assert_ne!(before.get_pixel(16, 12), after.get_pixel(16, 12));
            Ok(())
        }

        struct StaticColorView;

        impl Render for StaticColorView {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl IntoElement {
                div().size_full().bg(rgb(0x2040d0))
            }
        }

        struct PartialRepaintView {
            clickable: gpui::Entity<ClickableColorView>,
            static_color: gpui::Entity<StaticColorView>,
        }

        impl Render for PartialRepaintView {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl IntoElement {
                div()
                    .flex()
                    .size_full()
                    .child(div().w(px(8.0)).h_full().child(self.clickable.clone()))
                    .child(div().flex_1().h_full().child(self.static_color.clone()))
            }
        }

        #[test]
        fn partial_repaint_replays_cached_sibling_into_vello_scene() -> Result<()> {
            let mut cx = headless_context();
            let window = cx.open_window(size(px(16.0), px(12.0)), |_window, cx| {
                let clickable = cx.new(|_| ClickableColorView { clicked: false });
                let static_color = cx.new(|_| StaticColorView);
                cx.new(|_| PartialRepaintView {
                    clickable,
                    static_color,
                })
            })?;
            let window: AnyWindowHandle = window.into();

            redraw(&mut cx, window)?;
            let before = cx.capture_screenshot(window)?;
            assert_pixel_near(&before, 8, 12, [0xd0, 0x20, 0x20, 0xff], 2);
            assert_pixel_near(&before, 24, 12, [0x20, 0x40, 0xd0, 0xff], 2);

            simulate_click(&mut cx, window, point(px(4.0), px(6.0)))?;
            redraw(&mut cx, window)?;
            let after = cx.capture_screenshot(window)?;
            assert_pixel_near(&after, 8, 12, [0x20, 0xc0, 0x40, 0xff], 2);
            assert_pixel_near(&after, 24, 12, [0x20, 0x40, 0xd0, 0xff], 2);
            Ok(())
        }

        struct FullWindowColorView;

        impl Render for FullWindowColorView {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl IntoElement {
                div().size_full().bg(rgb(0x20c040))
            }
        }

        #[test]
        fn resizing_redraws_the_full_vello_target() -> Result<()> {
            let mut cx = headless_context();
            let window = cx.open_window(size(px(8.0), px(6.0)), |_window, cx| {
                cx.new(|_| FullWindowColorView)
            })?;
            let window: AnyWindowHandle = window.into();

            redraw(&mut cx, window)?;
            let before = cx.capture_screenshot(window)?;
            assert_eq!(before.dimensions(), (16, 12));

            cx.update_window(window, |_, window, cx| {
                window.resize(size(px(13.0), px(9.0)));
                window.bounds_changed(cx);
            })?;
            redraw(&mut cx, window)?;
            let after = cx.capture_screenshot(window)?;
            assert_eq!(after.dimensions(), (26, 18));
            assert_pixel_near(&after, 24, 16, [0x20, 0xc0, 0x40, 0xff], 2);
            Ok(())
        }
    }

    mod app_integration {
        use super::*;
        use gpui::{HeadlessAppContext, Render, div, prelude::*, px};
        use std::borrow::Cow;

        const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");

        struct HeadlessTextView;

        impl Render for HeadlessTextView {
            fn render(
                &mut self,
                _window: &mut gpui::Window,
                _cx: &mut gpui::Context<Self>,
            ) -> impl IntoElement {
                div()
                    .size_full()
                    .bg(rgb(0x102030))
                    .p(px(2.0))
                    .font_family("Lilex")
                    .text_size(px(12.0))
                    .text_color(rgb(0xffffff))
                    .child("A")
            }
        }

        #[test]
        fn headless_app_context_renders_parley_text_through_vello() -> Result<()> {
            use gpui::PlatformTextSystem as _;

            let text_system = Arc::new(crate::ParleyTextSystem::new_without_system_fonts("Lilex"));
            text_system.add_fonts(vec![Cow::Borrowed(LILEX)])?;
            let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(()), || {
                VelloHeadlessRenderer::new()
                    .map(|renderer| Box::new(renderer) as Box<dyn PlatformHeadlessRenderer>)
                    .map_err(|error| {
                        log::error!("failed to create test Vello headless renderer: {error}");
                        error
                    })
                    .ok()
            });
            let window = cx.open_window(size(px(32.0), px(20.0)), |_window, cx| {
                cx.new(|_| HeadlessTextView)
            })?;
            let image = cx.capture_screenshot(window.into())?;

            assert_eq!(image.dimensions(), (64, 40));
            assert_pixel_near(&image, 60, 36, [0x10, 0x20, 0x30, 0xff], 2);
            let bright_pixels = image
                .pixels()
                .filter(|pixel| pixel.0[0] > 0x80 && pixel.0[1] > 0x80 && pixel.0[2] > 0x80)
                .count();
            assert!(
                bright_pixels > 10,
                "expected the Parley glyph run to produce visible bright pixels"
            );
            Ok(())
        }
    }
}
