//! Translation from GPUI's retained scene primitives to Vello scene encoding.

#[cfg(target_os = "macos")]
use core_foundation::base::TCFType as _;
#[cfg(target_os = "macos")]
use core_video::{
    pixel_buffer::{CVPixelBuffer, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange},
    r#return::kCVReturnSuccess,
};
#[cfg(target_os = "macos")]
use gpui::PaintSurface;
use gpui::{
    Background, BackgroundContent, BorderStyle, Bounds, ColorSpace as GpuiColorSpace, ContentMask,
    Corners, Edges, Hsla, ImageId, Path as GpuiPath, PathCommand, PrimitiveBatch, Quad,
    RenderImage, ScaledPixels, Scene, Shadow, TransformationMatrix, Underline, VectorGlyphRun,
    VectorImage, VectorSvg,
};
#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use vello::{
    Scene as VelloScene,
    kurbo::{Affine, BezPath, Cap, Rect, RoundedRect, RoundedRectRadii, Shape, Stroke},
    peniko::{
        BlendMode, Blob, Brush, Color, Compose, Fill, Gradient, ImageAlphaType, ImageBrush,
        ImageData, ImageFormat, Mix, color::ColorSpaceTag,
    },
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ImageCacheKey {
    image_id: ImageId,
    frame_index: usize,
    grayscale: bool,
}

struct CachedImage {
    source: Weak<RenderImage>,
    data: ImageData,
}

struct CachedSvg {
    source: Weak<usvg::Tree>,
    scene: VelloScene,
}

#[cfg(target_os = "macos")]
struct CachedSurface {
    _source: CVPixelBuffer,
    data: ImageData,
}

#[derive(Default)]
pub(crate) struct VelloResourceCache {
    images: HashMap<ImageCacheKey, CachedImage>,
    svgs: HashMap<usize, CachedSvg>,
    #[cfg(target_os = "macos")]
    surfaces: HashMap<usize, CachedSurface>,
}

impl VelloResourceCache {
    fn prune_dropped_resources(&mut self) {
        self.images
            .retain(|_, cached| cached.source.strong_count() > 0);
        self.svgs
            .retain(|_, cached| cached.source.strong_count() > 0);
    }

    fn image_data(&mut self, image: &VectorImage) -> Option<ImageData> {
        let size = image.image.size(image.frame_index);
        if size.width.0 <= 0 || size.height.0 <= 0 {
            return None;
        }

        let key = ImageCacheKey {
            image_id: image.image.id,
            frame_index: image.frame_index,
            grayscale: image.grayscale,
        };
        if let Some(cached) = self.images.get(&key)
            && cached
                .source
                .upgrade()
                .map(|source| Arc::ptr_eq(&source, &image.image))
                .unwrap_or(false)
        {
            return Some(cached.data.clone());
        }

        let source = image.image.as_bytes(image.frame_index)?;
        let mut rgba = source.to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if image.grayscale {
                let luminance = (0.2126 * pixel[0] as f32
                    + 0.7152 * pixel[1] as f32
                    + 0.0722 * pixel[2] as f32)
                    .round() as u8;
                pixel[0] = luminance;
                pixel[1] = luminance;
                pixel[2] = luminance;
            }
        }

        let data = ImageData {
            data: Blob::new(Arc::new(rgba.into_boxed_slice())),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: size.width.0 as u32,
            height: size.height.0 as u32,
        };
        self.images.insert(
            key,
            CachedImage {
                source: Arc::downgrade(&image.image),
                data: data.clone(),
            },
        );
        Some(data)
    }

    #[cfg(target_os = "macos")]
    fn prune_unused_surfaces(&mut self, scene: &Scene) {
        let active = scene
            .surfaces
            .iter()
            .map(|surface| surface.image_buffer.as_concrete_TypeRef() as usize)
            .collect::<HashSet<_>>();
        self.surfaces.retain(|key, _| active.contains(key));
    }

    #[cfg(target_os = "macos")]
    fn surface_data(&mut self, surface: &PaintSurface) -> anyhow::Result<ImageData> {
        let key = surface.image_buffer.as_concrete_TypeRef() as usize;
        if let Some(cached) = self.surfaces.get(&key) {
            return Ok(cached.data.clone());
        }

        let data = cv_pixel_buffer_image_data(&surface.image_buffer)?;
        self.surfaces.insert(
            key,
            CachedSurface {
                _source: surface.image_buffer.clone(),
                data: data.clone(),
            },
        );
        Ok(data)
    }

    fn svg_scene(&mut self, tree: &Arc<usvg::Tree>) -> &VelloScene {
        let key = Arc::as_ptr(tree) as usize;
        let cached = self.svgs.entry(key).or_insert_with(|| CachedSvg {
            source: Arc::downgrade(tree),
            scene: vello_svg::render_tree(tree),
        });
        let matches_source = cached
            .source
            .upgrade()
            .map(|source| Arc::ptr_eq(&source, tree))
            .unwrap_or(false);
        if !matches_source {
            *cached = CachedSvg {
                source: Arc::downgrade(tree),
                scene: vello_svg::render_tree(tree),
            };
        }
        &cached.scene
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnsupportedPrimitives {
    pub surfaces: usize,
}

impl UnsupportedPrimitives {
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VelloSceneStats {
    pub(crate) quads: usize,
    pub(crate) paths: usize,
    pub(crate) underlines: usize,
    pub(crate) glyph_runs: usize,
    pub(crate) glyphs: usize,
    pub(crate) glyph_orders: usize,
    pub(crate) glyph_clip_batches: usize,
    pub(crate) glyph_draws: usize,
    pub(crate) images: usize,
    pub(crate) svgs: usize,
    pub(crate) surfaces: usize,
}

pub(crate) struct VelloSceneBuild {
    pub(crate) unsupported: UnsupportedPrimitives,
    pub(crate) stats: VelloSceneStats,
}

pub(crate) fn rebuild_vello_scene(
    vello_scene: &mut VelloScene,
    resource_cache: &mut VelloResourceCache,
    gpui_scene: &Scene,
) -> VelloSceneBuild {
    resource_cache.prune_dropped_resources();
    #[cfg(target_os = "macos")]
    resource_cache.prune_unused_surfaces(gpui_scene);
    vello_scene.reset();
    let mut unsupported = UnsupportedPrimitives::default();
    let mut stats = VelloSceneStats {
        quads: gpui_scene.quads.len(),
        paths: gpui_scene.paths.len(),
        underlines: gpui_scene.underlines.len(),
        glyph_runs: gpui_scene.glyph_runs.len(),
        glyphs: gpui_scene
            .glyph_runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum(),
        images: gpui_scene.vector_images.len(),
        svgs: gpui_scene.vector_svgs.len(),
        surfaces: gpui_scene.surfaces.len(),
        ..VelloSceneStats::default()
    };

    for batch in gpui_scene.batches() {
        match batch {
            PrimitiveBatch::Quads(range) => {
                for quad in &gpui_scene.quads[range] {
                    encode_quad(vello_scene, quad);
                }
            }
            PrimitiveBatch::Paths(range) => {
                for path in &gpui_scene.paths[range] {
                    encode_path(vello_scene, path);
                }
            }
            PrimitiveBatch::Underlines(range) => {
                for underline in &gpui_scene.underlines[range] {
                    encode_underline(vello_scene, underline);
                }
            }
            PrimitiveBatch::GlyphRuns(range) => {
                let glyph_stats = encode_glyph_runs(vello_scene, &gpui_scene.glyph_runs[range]);
                stats.glyph_orders += glyph_stats.orders;
                stats.glyph_clip_batches += glyph_stats.clip_batches;
                stats.glyph_draws += glyph_stats.draws;
            }
            PrimitiveBatch::VectorImages(range) => {
                for image in &gpui_scene.vector_images[range] {
                    encode_image(vello_scene, resource_cache, image);
                }
            }
            PrimitiveBatch::VectorSvgs(range) => {
                for svg in &gpui_scene.vector_svgs[range] {
                    encode_svg(vello_scene, resource_cache, svg);
                }
            }
            PrimitiveBatch::Shadows(range) => {
                for shadow in &gpui_scene.shadows[range] {
                    encode_shadow(vello_scene, shadow);
                }
            }

            PrimitiveBatch::Surfaces(range) => {
                #[cfg(target_os = "macos")]
                for surface in &gpui_scene.surfaces[range] {
                    if let Err(error) = encode_surface(vello_scene, resource_cache, surface) {
                        log::error!("failed to render CVPixelBuffer through Vello: {error:#}");
                        unsupported.surfaces += 1;
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    unsupported.surfaces += range.len();
                }
            }
        }
    }

    VelloSceneBuild { unsupported, stats }
}

fn encode_shadow(scene: &mut VelloScene, shadow: &Shadow) {
    let shadow_rect = rect(shadow.bounds);
    if shadow_rect.width() <= 0.0 || shadow_rect.height() <= 0.0 {
        return;
    }
    let shadow_radius = maximum_radius(shadow.corner_radii);
    let std_dev = (shadow.blur_radius.0 / 2.0).max(0.0) as f64;

    with_clip(scene, shadow.content_mask, |scene| {
        if !shadow.inset {
            if std_dev == 0.0 {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color(shadow.color),
                    None,
                    &rounded_rect(shadow.bounds, shadow.corner_radii),
                );
            } else {
                scene.draw_blurred_rounded_rect(
                    Affine::IDENTITY,
                    shadow_rect,
                    color(shadow.color),
                    shadow_radius,
                    std_dev,
                );
            }
            return;
        }

        let element = rounded_rect(shadow.element_bounds, shadow.element_corner_radii);
        scene.push_layer(
            Fill::NonZero,
            BlendMode::default(),
            1.0,
            Affine::IDENTITY,
            &element,
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(shadow.color),
            None,
            &element,
        );
        scene.push_layer(
            Fill::NonZero,
            BlendMode::new(Mix::Normal, Compose::DestOut),
            1.0,
            Affine::IDENTITY,
            &element,
        );
        if std_dev == 0.0 {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::WHITE,
                None,
                &rounded_rect(shadow.bounds, shadow.corner_radii),
            );
        } else {
            scene.draw_blurred_rounded_rect(
                Affine::IDENTITY,
                shadow_rect,
                Color::WHITE,
                shadow_radius,
                std_dev,
            );
        }
        scene.pop_layer();
        scene.pop_layer();
    });
}

fn encode_quad(scene: &mut VelloScene, quad: &Quad) {
    let outer = rounded_rect(quad.bounds, quad.corner_radii);
    let has_border = border_is_visible(quad);

    with_clip(scene, quad.content_mask, |scene| {
        fill_background(scene, &quad.background, quad.bounds, &outer);

        if has_border {
            if quad.border_style == BorderStyle::Dashed {
                encode_dashed_border(scene, quad, &outer);
                return;
            }

            let mut ring = BezPath::new();
            ring.extend(outer.path_elements(0.1));
            if let Some(inner) = inner_rounded_rect(quad) {
                ring.extend(inner.path_elements(0.1));
            }
            scene.fill(
                Fill::EvenOdd,
                Affine::IDENTITY,
                color(quad.border_color),
                None,
                &ring,
            );
        }
    });
}

fn encode_path(scene: &mut VelloScene, path: &GpuiPath<ScaledPixels>) {
    let mut vector_path = BezPath::with_capacity(path.commands.len());
    for command in &path.commands {
        match command {
            PathCommand::MoveTo(to) => vector_path.move_to(point(*to)),
            PathCommand::LineTo(to) => vector_path.line_to(point(*to)),
            PathCommand::QuadTo { control, to } => {
                vector_path.quad_to(point(*control), point(*to));
            }
            PathCommand::Close => vector_path.close_path(),
        }
    }
    if vector_path.is_empty() {
        return;
    }

    with_clip(scene, path.content_mask, |scene| {
        fill_background(scene, &path.color, path.bounds, &vector_path);
    });
}

struct GlyphStyleBatch<'a> {
    prototype: &'a VectorGlyphRun,
    runs: Vec<&'a VectorGlyphRun>,
}

struct GlyphClipBatch<'a> {
    content_mask: ContentMask<ScaledPixels>,
    styles: Vec<GlyphStyleBatch<'a>>,
}

impl<'a> GlyphClipBatch<'a> {
    fn new(run: &'a VectorGlyphRun) -> Self {
        Self {
            content_mask: run.content_mask,
            styles: vec![GlyphStyleBatch {
                prototype: run,
                runs: vec![run],
            }],
        }
    }

    fn push(&mut self, run: &'a VectorGlyphRun) {
        if let Some(style) = self
            .styles
            .iter_mut()
            .find(|style| glyph_runs_are_compatible(style.prototype, run))
        {
            style.runs.push(run);
        } else {
            self.styles.push(GlyphStyleBatch {
                prototype: run,
                runs: vec![run],
            });
        }
    }
}

fn glyph_clip_batches(runs: &[VectorGlyphRun]) -> Vec<GlyphClipBatch<'_>> {
    let mut batches: Vec<GlyphClipBatch<'_>> = Vec::new();
    for run in runs {
        if let Some(batch) = batches
            .iter_mut()
            .find(|batch| batch.content_mask == run.content_mask)
        {
            batch.push(run);
        } else {
            batches.push(GlyphClipBatch::new(run));
        }
    }
    batches
}

#[derive(Default)]
struct GlyphEncodingStats {
    orders: usize,
    clip_batches: usize,
    draws: usize,
}

fn encode_glyph_runs(scene: &mut VelloScene, runs: &[VectorGlyphRun]) -> GlyphEncodingStats {
    let mut stats = GlyphEncodingStats::default();
    let mut start = 0;
    while let Some(first) = runs.get(start) {
        let end = runs[start + 1..]
            .iter()
            .position(|run| run.order != first.order)
            .map_or(runs.len(), |offset| start + offset + 1);

        stats.orders += 1;
        for clip_batch in glyph_clip_batches(&runs[start..end]) {
            stats.clip_batches += 1;
            with_clip(scene, clip_batch.content_mask, |scene| {
                for style in clip_batch.styles {
                    stats.draws += 1;
                    let prototype = style.prototype;
                    scene
                        .draw_glyphs(&prototype.font)
                        .font_size(prototype.font_size.0)
                        .normalized_coords(&prototype.normalized_coords)
                        .brush(color(prototype.color))
                        .draw(
                            Fill::NonZero,
                            style.runs.into_iter().flat_map(|run| {
                                run.glyphs.iter().map(|glyph| vello::Glyph {
                                    id: glyph.id.0,
                                    x: glyph.position.x.0,
                                    y: glyph.position.y.0,
                                })
                            }),
                        );
                }
            });
        }
        start = end;
    }
    stats
}

fn glyph_runs_are_compatible(first: &VectorGlyphRun, next: &VectorGlyphRun) -> bool {
    first.order == next.order
        && first.font == next.font
        && first.font_size == next.font_size
        && first.normalized_coords == next.normalized_coords
        && first.color == next.color
        && first.content_mask == next.content_mask
}

fn encode_svg(scene: &mut VelloScene, resource_cache: &mut VelloResourceCache, svg: &VectorSvg) {
    let intrinsic = svg.tree.size();
    if intrinsic.width() <= 0.0 || intrinsic.height() <= 0.0 {
        return;
    }

    let destination = rect(svg.bounds);
    let source = Rect::new(
        0.0,
        0.0,
        intrinsic.width() as f64,
        intrinsic.height() as f64,
    );
    let placement = Affine::translate((destination.x0, destination.y0))
        * Affine::scale_non_uniform(
            destination.width() / source.width(),
            destination.height() / source.height(),
        );
    let transform = affine(svg.transformation) * placement;
    let svg_scene = resource_cache.svg_scene(&svg.tree);

    with_clip(scene, svg.content_mask, |scene| {
        scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, transform, &source);
        scene.fill(Fill::NonZero, transform, color(svg.color), None, &source);
        scene.push_layer(
            Fill::NonZero,
            BlendMode::new(Mix::Normal, Compose::DestIn),
            1.0,
            transform,
            &source,
        );
        scene.append(&svg_scene, Some(transform));
        scene.pop_layer();
        scene.pop_layer();
    });
}

fn affine(transform: TransformationMatrix) -> Affine {
    Affine::new([
        transform.rotation_scale[0][0] as f64,
        transform.rotation_scale[1][0] as f64,
        transform.rotation_scale[0][1] as f64,
        transform.rotation_scale[1][1] as f64,
        transform.translation[0] as f64,
        transform.translation[1] as f64,
    ])
}

#[cfg(any(target_os = "macos", test))]
fn nv12_full_range_to_rgba(
    y_plane: &[u8],
    y_stride: usize,
    uv_plane: &[u8],
    uv_stride: usize,
    width: usize,
    height: usize,
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(width > 0 && height > 0, "NV12 frame has an empty size");
    anyhow::ensure!(
        y_stride >= width,
        "NV12 luma stride {y_stride} is smaller than width {width}"
    );
    let chroma_row_bytes = width.div_ceil(2).checked_mul(2).ok_or_else(|| {
        anyhow::anyhow!("NV12 chroma row byte count overflowed for width {width}")
    })?;
    anyhow::ensure!(
        uv_stride >= chroma_row_bytes,
        "NV12 chroma stride {uv_stride} is smaller than required row size {chroma_row_bytes}"
    );
    let required_y_bytes = y_stride
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("NV12 luma byte count overflowed"))?;
    let chroma_height = height.div_ceil(2);
    let required_uv_bytes = uv_stride
        .checked_mul(chroma_height)
        .ok_or_else(|| anyhow::anyhow!("NV12 chroma byte count overflowed"))?;
    anyhow::ensure!(
        y_plane.len() >= required_y_bytes,
        "NV12 luma plane is truncated: {} < {required_y_bytes}",
        y_plane.len()
    );
    anyhow::ensure!(
        uv_plane.len() >= required_uv_bytes,
        "NV12 chroma plane is truncated: {} < {required_uv_bytes}",
        uv_plane.len()
    );

    let output_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("NV12 RGBA output byte count overflowed"))?;
    let mut rgba = Vec::with_capacity(output_bytes);
    for row_index in 0..height {
        let y_row_start = row_index
            .checked_mul(y_stride)
            .ok_or_else(|| anyhow::anyhow!("NV12 luma row offset overflowed"))?;
        let uv_row_start = (row_index / 2)
            .checked_mul(uv_stride)
            .ok_or_else(|| anyhow::anyhow!("NV12 chroma row offset overflowed"))?;
        let y_row = y_plane
            .get(y_row_start..y_row_start + width)
            .ok_or_else(|| anyhow::anyhow!("NV12 luma row is truncated"))?;
        let uv_row = uv_plane
            .get(uv_row_start..uv_row_start + chroma_row_bytes)
            .ok_or_else(|| anyhow::anyhow!("NV12 chroma row is truncated"))?;

        for (column_index, y) in y_row.iter().copied().enumerate() {
            let chroma_index = (column_index / 2) * 2;
            let cb = *uv_row
                .get(chroma_index)
                .ok_or_else(|| anyhow::anyhow!("NV12 Cb sample is missing"))?
                as f32
                - 128.0;
            let cr = *uv_row
                .get(chroma_index + 1)
                .ok_or_else(|| anyhow::anyhow!("NV12 Cr sample is missing"))?
                as f32
                - 128.0;
            let y = y as f32;
            let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[
                channel(y + 1.4020 * cr),
                channel(y - 0.3441 * cb - 0.7141 * cr),
                channel(y + 1.7720 * cb),
                0xff,
            ]);
        }
    }
    Ok(rgba)
}

#[cfg(target_os = "macos")]
fn cv_pixel_buffer_image_data(buffer: &CVPixelBuffer) -> anyhow::Result<ImageData> {
    anyhow::ensure!(
        buffer.get_pixel_format() == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        "unsupported CVPixelBuffer format {}",
        buffer.get_pixel_format()
    );
    let width = buffer.get_width();
    let height = buffer.get_height();
    anyhow::ensure!(width > 0 && height > 0, "CVPixelBuffer has an empty size");

    let lock_result = buffer.lock_base_address(0);
    anyhow::ensure!(
        lock_result == kCVReturnSuccess,
        "failed to lock CVPixelBuffer: CVReturn({lock_result})"
    );

    let conversion = (|| {
        let y_stride = buffer.get_bytes_per_row_of_plane(0);
        let uv_stride = buffer.get_bytes_per_row_of_plane(1);
        let y_bytes = buffer
            .get_height_of_plane(0)
            .checked_mul(y_stride)
            .ok_or_else(|| anyhow::anyhow!("CVPixelBuffer luma byte count overflowed"))?;
        let uv_bytes = buffer
            .get_height_of_plane(1)
            .checked_mul(uv_stride)
            .ok_or_else(|| anyhow::anyhow!("CVPixelBuffer chroma byte count overflowed"))?;
        let y_address = unsafe { buffer.get_base_address_of_plane(0) };
        let uv_address = unsafe { buffer.get_base_address_of_plane(1) };
        anyhow::ensure!(
            !y_address.is_null() && !uv_address.is_null(),
            "CVPixelBuffer returned a null plane address"
        );
        let y_plane = unsafe { std::slice::from_raw_parts(y_address.cast::<u8>(), y_bytes) };
        let uv_plane = unsafe { std::slice::from_raw_parts(uv_address.cast::<u8>(), uv_bytes) };
        nv12_full_range_to_rgba(y_plane, y_stride, uv_plane, uv_stride, width, height)
    })();

    let unlock_result = buffer.unlock_base_address(0);
    anyhow::ensure!(
        unlock_result == kCVReturnSuccess,
        "failed to unlock CVPixelBuffer: CVReturn({unlock_result})"
    );
    let rgba = conversion?;

    Ok(ImageData {
        data: Blob::new(Arc::new(rgba.into_boxed_slice())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: u32::try_from(width)
            .map_err(|_| anyhow::anyhow!("CVPixelBuffer width is too large"))?,
        height: u32::try_from(height)
            .map_err(|_| anyhow::anyhow!("CVPixelBuffer height is too large"))?,
    })
}

#[cfg(target_os = "macos")]
fn encode_surface(
    scene: &mut VelloScene,
    resource_cache: &mut VelloResourceCache,
    surface: &PaintSurface,
) -> anyhow::Result<()> {
    let data = resource_cache.surface_data(surface)?;
    let source_width = data.width as f64;
    let source_height = data.height as f64;
    let destination = rect(surface.bounds);
    let transform = Affine::translate((destination.x0, destination.y0))
        * Affine::scale_non_uniform(
            destination.width() / source_width,
            destination.height() / source_height,
        );
    let brush = ImageBrush::new(data);
    with_clip(scene, surface.content_mask, |scene| {
        scene.draw_image(&brush, transform);
    });
    Ok(())
}

fn encode_image(
    scene: &mut VelloScene,
    resource_cache: &mut VelloResourceCache,
    image: &VectorImage,
) {
    let size = image.image.size(image.frame_index);
    let Some(data) = resource_cache.image_data(image) else {
        return;
    };
    let brush = ImageBrush::new(data).with_alpha(image.opacity);
    let destination = rect(image.bounds);
    let transform = Affine::translate((destination.x0, destination.y0))
        * Affine::scale_non_uniform(
            destination.width() / size.width.0 as f64,
            destination.height() / size.height.0 as f64,
        );
    let clip = rounded_rect(image.bounds, image.corner_radii);

    with_clip(scene, image.content_mask, |scene| {
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
        scene.draw_image(&brush, transform);
        scene.pop_layer();
    });
}

fn encode_underline(scene: &mut VelloScene, underline: &Underline) {
    let rect = rect(underline.bounds);
    if underline.wavy {
        let thickness = underline.thickness.0.max(1.0) as f64;
        if rect.height() <= 0.0 || rect.width() <= 0.0 {
            return;
        }
        let amplitude = thickness * 0.8;
        let wavelength = rect.height() * rect.height() / thickness;
        let angular_frequency = std::f64::consts::TAU / wavelength;
        let sample_step = (wavelength / 12.0).clamp(0.5, 2.0);
        let center_y = rect.y0 + rect.height() / 2.0;
        let mut wave = BezPath::new();
        wave.move_to((rect.x0, center_y));
        let mut x = rect.x0 + sample_step;
        while x < rect.x1 {
            let y = center_y + ((x - rect.x0) * angular_frequency).sin() * amplitude;
            wave.line_to((x, y));
            x += sample_step;
        }
        let end_y = center_y + (rect.width() * angular_frequency).sin() * amplitude;
        wave.line_to((rect.x1, end_y));
        with_clip(scene, underline.content_mask, |scene| {
            scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
            scene.stroke(
                &Stroke::new(thickness),
                Affine::IDENTITY,
                color(underline.color),
                None,
                &wave,
            );
            scene.pop_layer();
        });
        return;
    }
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    with_clip(scene, underline.content_mask, |scene| {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color(underline.color),
            None,
            &rect,
        );
    });
}

fn with_clip(
    scene: &mut VelloScene,
    mask: ContentMask<ScaledPixels>,
    draw: impl FnOnce(&mut VelloScene),
) {
    let clip = rect(mask.bounds);
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    draw(scene);
    scene.pop_layer();
}

fn rounded_rect(bounds: Bounds<ScaledPixels>, radii: Corners<ScaledPixels>) -> RoundedRect {
    RoundedRect::from_rect(
        rect(bounds),
        RoundedRectRadii::new(
            radii.top_left.0 as f64,
            radii.top_right.0 as f64,
            radii.bottom_right.0 as f64,
            radii.bottom_left.0 as f64,
        ),
    )
}

fn inner_rounded_rect(quad: &Quad) -> Option<RoundedRect> {
    let Edges {
        top,
        right,
        bottom,
        left,
    } = quad.border_widths;
    let outer = rect(quad.bounds);
    let inner = Rect::new(
        outer.x0 + left.0 as f64,
        outer.y0 + top.0 as f64,
        outer.x1 - right.0 as f64,
        outer.y1 - bottom.0 as f64,
    );
    if inner.width() <= 0.0 || inner.height() <= 0.0 {
        return None;
    }

    Some(RoundedRect::from_rect(
        inner,
        RoundedRectRadii::new(
            (quad.corner_radii.top_left.0 - left.0.max(top.0)).max(0.0) as f64,
            (quad.corner_radii.top_right.0 - right.0.max(top.0)).max(0.0) as f64,
            (quad.corner_radii.bottom_right.0 - right.0.max(bottom.0)).max(0.0) as f64,
            (quad.corner_radii.bottom_left.0 - left.0.max(bottom.0)).max(0.0) as f64,
        ),
    ))
}

fn maximum_radius(radii: Corners<ScaledPixels>) -> f64 {
    [
        radii.top_left.0,
        radii.top_right.0,
        radii.bottom_right.0,
        radii.bottom_left.0,
    ]
    .into_iter()
    .fold(0.0_f32, f32::max) as f64
}

fn encode_dashed_border(scene: &mut VelloScene, quad: &Quad, outer: &RoundedRect) {
    let widths = [
        quad.border_widths.top.0 as f64,
        quad.border_widths.right.0 as f64,
        quad.border_widths.bottom.0 as f64,
        quad.border_widths.left.0 as f64,
    ];
    let first_width = widths[0];
    if first_width > 0.0
        && widths
            .iter()
            .all(|width| (*width - first_width).abs() <= f64::EPSILON)
    {
        let stroke = dashed_stroke(first_width * 2.0, first_width);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, outer);
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            color(quad.border_color),
            None,
            outer,
        );
        scene.pop_layer();
        return;
    }

    let bounds = rect(quad.bounds);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, outer);
    stroke_dashed_line(
        scene,
        widths[0],
        (bounds.x0, bounds.y0 + widths[0] / 2.0),
        (bounds.x1, bounds.y0 + widths[0] / 2.0),
        quad.border_color,
    );
    stroke_dashed_line(
        scene,
        widths[1],
        (bounds.x1 - widths[1] / 2.0, bounds.y0),
        (bounds.x1 - widths[1] / 2.0, bounds.y1),
        quad.border_color,
    );
    stroke_dashed_line(
        scene,
        widths[2],
        (bounds.x1, bounds.y1 - widths[2] / 2.0),
        (bounds.x0, bounds.y1 - widths[2] / 2.0),
        quad.border_color,
    );
    stroke_dashed_line(
        scene,
        widths[3],
        (bounds.x0 + widths[3] / 2.0, bounds.y1),
        (bounds.x0 + widths[3] / 2.0, bounds.y0),
        quad.border_color,
    );
    scene.pop_layer();
}

fn dashed_stroke(stroke_width: f64, dash_width: f64) -> Stroke {
    Stroke::new(stroke_width)
        .with_caps(Cap::Butt)
        .with_dashes(0.0, [dash_width * 2.0, dash_width])
}

fn stroke_dashed_line(
    scene: &mut VelloScene,
    width: f64,
    start: (f64, f64),
    end: (f64, f64),
    border_color: Hsla,
) {
    if !width.is_finite() || width <= 0.0 {
        return;
    }
    let mut path = BezPath::new();
    path.move_to(start);
    path.line_to(end);
    scene.stroke(
        &dashed_stroke(width, width),
        Affine::IDENTITY,
        color(border_color),
        None,
        &path,
    );
}

fn border_is_visible(quad: &Quad) -> bool {
    quad.border_color.a > 0.0
        && (quad.border_widths.top.0 > 0.0
            || quad.border_widths.right.0 > 0.0
            || quad.border_widths.bottom.0 > 0.0
            || quad.border_widths.left.0 > 0.0)
}

fn rect(bounds: Bounds<ScaledPixels>) -> Rect {
    let x0 = bounds.origin.x.0 as f64;
    let y0 = bounds.origin.y.0 as f64;
    Rect::new(
        x0,
        y0,
        x0 + bounds.size.width.0 as f64,
        y0 + bounds.size.height.0 as f64,
    )
}

fn point(point: gpui::Point<ScaledPixels>) -> (f64, f64) {
    (point.x.0 as f64, point.y.0 as f64)
}

fn background_brush(background: &Background, bounds: Bounds<ScaledPixels>) -> Option<Brush> {
    match background.content() {
        BackgroundContent::Solid(solid) => Some(Brush::Solid(color(solid))),
        BackgroundContent::LinearGradient {
            angle,
            stops,
            color_space,
        } => {
            let rect = rect(bounds);
            let radians = (angle as f64).to_radians();
            let direction = (radians.sin(), -radians.cos());
            let extent =
                direction.0.abs() * rect.width() / 2.0 + direction.1.abs() * rect.height() / 2.0;
            let center = rect.center();
            let start = (
                center.x - direction.0 * extent,
                center.y - direction.1 * extent,
            );
            let end = (
                center.x + direction.0 * extent,
                center.y + direction.1 * extent,
            );
            let interpolation = match color_space {
                GpuiColorSpace::Srgb => ColorSpaceTag::Srgb,
                GpuiColorSpace::Oklab => ColorSpaceTag::Oklab,
            };
            Some(Brush::Gradient(
                Gradient::new_linear(start, end)
                    .with_stops([
                        (stops[0].percentage, color(stops[0].color)),
                        (stops[1].percentage, color(stops[1].color)),
                    ])
                    .with_interpolation_cs(interpolation),
            ))
        }
        BackgroundContent::PatternSlash { .. } | BackgroundContent::Checkerboard { .. } => None,
    }
}

fn fill_background(
    scene: &mut VelloScene,
    background: &Background,
    bounds: Bounds<ScaledPixels>,
    shape: &impl Shape,
) {
    if let Some(brush) = background_brush(background, bounds) {
        scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, shape);
        return;
    }

    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, shape);
    match background.content() {
        BackgroundContent::PatternSlash {
            color: fill,
            width,
            interval,
        } => {
            fill_slash_pattern(scene, bounds, fill, width, interval);
        }
        BackgroundContent::Checkerboard { color: fill, size } => {
            fill_checkerboard_pattern(scene, bounds, fill, size);
        }
        BackgroundContent::Solid(_) | BackgroundContent::LinearGradient { .. } => {}
    }
    scene.pop_layer();
}

fn fill_slash_pattern(
    scene: &mut VelloScene,
    bounds: Bounds<ScaledPixels>,
    fill: Hsla,
    width: f32,
    interval: f32,
) {
    let pattern_width = width as f64;
    let pattern_interval = interval as f64;
    let pattern_height = pattern_width + pattern_interval;
    if !pattern_height.is_finite() || pattern_height <= 0.0 || pattern_width <= 0.0 {
        return;
    }

    let bounds = rect(bounds);
    let start_index = (-bounds.height() / pattern_height).floor() as i32 - 1;
    let end_index = (bounds.width() / pattern_height).ceil() as i32 + 1;
    let margin = bounds.width() + bounds.height();
    let stroke = Stroke::new(pattern_width * std::f64::consts::FRAC_1_SQRT_2);
    for index in start_index..=end_index {
        let offset = index as f64 * pattern_height;
        let mut slash = BezPath::new();
        slash.move_to((bounds.x0 + offset - margin, bounds.y0 - margin));
        slash.line_to((
            bounds.x0 + offset + bounds.height() + margin,
            bounds.y1 + margin,
        ));
        scene.stroke(&stroke, Affine::IDENTITY, color(fill), None, &slash);
    }
}

fn fill_checkerboard_pattern(
    scene: &mut VelloScene,
    bounds: Bounds<ScaledPixels>,
    fill: Hsla,
    size: f32,
) {
    let size = size as f64;
    if !size.is_finite() || size <= 0.0 {
        return;
    }

    let bounds = rect(bounds);
    let columns = (bounds.width() / size).ceil() as usize;
    let rows = (bounds.height() / size).ceil() as usize;
    for row in 0..rows {
        for column in 0..columns {
            if (row + column).is_multiple_of(2) {
                continue;
            }
            let checker = Rect::new(
                bounds.x0 + column as f64 * size,
                bounds.y0 + row as f64 * size,
                (bounds.x0 + (column + 1) as f64 * size).min(bounds.x1),
                (bounds.y0 + (row + 1) as f64 * size).min(bounds.y1),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, color(fill), None, &checker);
        }
    }
}

fn color(color: Hsla) -> Color {
    let rgba = color.to_rgb();
    Color::new([rgba.r, rgba.g, rgba.b, rgba.a])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::RenderImage;
    use image::{Frame, RgbaImage};
    use parley::FontData;
    use smallvec::smallvec;

    fn glyph_run(order: u32) -> VectorGlyphRun {
        VectorGlyphRun {
            order,
            bounds: Bounds::default(),
            content_mask: ContentMask::default(),
            color: Hsla::default(),
            font: FontData::new(Blob::new(Arc::new(Vec::<u8>::new())), 0),
            font_size: ScaledPixels(16.0),
            normalized_coords: Arc::from([]),
            glyphs: Arc::from([]),
        }
    }

    #[test]
    fn glyph_batching_preserves_draw_order_boundaries() {
        let first = glyph_run(1);
        let same_order = first.clone();
        let mut different_order = first.clone();
        different_order.order = 2;

        assert!(glyph_runs_are_compatible(&first, &same_order));
        assert!(!glyph_runs_are_compatible(&first, &different_order));
    }

    #[test]
    fn glyph_batching_combines_nonadjacent_styles_at_the_same_order() {
        let mut red = glyph_run(1);
        red.color = gpui::red();
        let mut blue = red.clone();
        blue.color = gpui::blue();
        let runs = [red.clone(), blue, red];

        let clip_batches = glyph_clip_batches(&runs);
        let clip_batch = clip_batches
            .first()
            .expect("same-mask glyph runs should produce a clip batch");

        assert_eq!(clip_batches.len(), 1);
        assert_eq!(clip_batch.styles.len(), 2);
        assert_eq!(clip_batch.styles[0].runs.len(), 2);
        assert_eq!(clip_batch.styles[1].runs.len(), 1);
    }

    #[test]
    fn preserves_gradient_interpolation_color_space() {
        let background = gpui::linear_gradient(
            90.0,
            gpui::linear_color_stop(gpui::red(), 0.0),
            gpui::linear_color_stop(gpui::blue(), 1.0),
        )
        .color_space(gpui::ColorSpace::Oklab);
        let brush = background_brush(
            &background,
            Bounds {
                origin: gpui::point(ScaledPixels(0.0), ScaledPixels(0.0)),
                size: gpui::size(ScaledPixels(10.0), ScaledPixels(10.0)),
            },
        );
        let Some(Brush::Gradient(gradient)) = brush else {
            panic!("linear gradient should produce a gradient brush");
        };

        assert_eq!(gradient.interpolation_cs, ColorSpaceTag::Oklab);
    }

    #[test]
    fn converts_full_range_nv12_to_rgba_with_plane_strides() {
        let y_plane = [0, 64, 0xee, 0xee, 128, 255, 0xee, 0xee];
        let uv_plane = [128, 128, 0xee, 0xee];
        let rgba = nv12_full_range_to_rgba(&y_plane, 4, &uv_plane, 4, 2, 2)
            .expect("valid NV12 planes should convert");

        assert_eq!(
            rgba,
            [
                0, 0, 0, 255, 64, 64, 64, 255, 128, 128, 128, 255, 255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn caches_image_data_by_frame_and_grayscale_variant() {
        let pixels = RgbaImage::from_raw(1, 1, vec![0x10, 0x20, 0x30, 0x40])
            .expect("test image dimensions should match its pixel data");
        let source = Arc::new(RenderImage::new(smallvec![Frame::new(pixels)]));
        let mut image = VectorImage {
            order: 0,
            bounds: Bounds::default(),
            content_mask: ContentMask::default(),
            corner_radii: Corners::default(),
            image: source.clone(),
            frame_index: 0,
            grayscale: false,
            opacity: 1.0,
        };
        let mut cache = VelloResourceCache::default();

        let first = cache
            .image_data(&image)
            .expect("valid image data should be converted");
        let second = cache
            .image_data(&image)
            .expect("valid image data should be cached");
        assert_eq!(first.data.data(), &[0x30, 0x20, 0x10, 0x40]);
        assert_eq!(first.data.id(), second.data.id());
        assert_eq!(cache.images.len(), 1);

        image.grayscale = true;
        let grayscale = cache
            .image_data(&image)
            .expect("grayscale image data should be converted");
        assert_eq!(grayscale.data.data(), &[0x22, 0x22, 0x22, 0x40]);
        assert_ne!(first.data.id(), grayscale.data.id());
        assert_eq!(cache.images.len(), 2);

        drop(image);
        drop(source);
        cache.prune_dropped_resources();
        assert!(cache.images.is_empty());
    }

    #[test]
    fn caches_svg_scene_by_tree_identity() {
        let first_tree = Arc::new(
            usvg::Tree::from_str(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>"#,
                &usvg::Options::default(),
            )
            .expect("test SVG should parse"),
        );
        let mut cache = VelloResourceCache::default();

        let first_scene = cache.svg_scene(&first_tree) as *const VelloScene;
        let reused_scene = cache.svg_scene(&first_tree) as *const VelloScene;
        assert_eq!(first_scene, reused_scene);
        assert_eq!(cache.svgs.len(), 1);

        let second_tree = Arc::new(
            usvg::Tree::from_str(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><circle cx="1" cy="1" r="1"/></svg>"#,
                &usvg::Options::default(),
            )
            .expect("replacement test SVG should parse"),
        );
        cache.svg_scene(&second_tree);
        assert_eq!(cache.svgs.len(), 2);

        drop(first_tree);
        cache.prune_dropped_resources();
        assert_eq!(cache.svgs.len(), 1);

        drop(second_tree);
        cache.prune_dropped_resources();
        assert!(cache.svgs.is_empty());
    }
}
