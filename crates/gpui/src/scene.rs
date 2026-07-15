use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels, Point, Radians, ScaledPixels,
    Size, bounds_tree::BoundsTree,
};
use std::{
    fmt::Debug,
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
    sync::Arc,
};

#[expect(missing_docs)]
pub type DrawOrder = u32;

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub glyph_runs: Vec<VectorGlyphRun>,
    pub vector_images: Vec<VectorImage>,
    pub vector_svgs: Vec<VectorSvg>,
    pub surfaces: Vec<PaintSurface>,
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.glyph_runs.clear();
        self.vector_images.clear();
        self.vector_svgs.clear();
        self.surfaces.clear();
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        let clipped_bounds = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            return;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::GlyphRun(glyph_run) => {
                glyph_run.order = order;
                self.glyph_runs.push(glyph_run.clone());
            }
            Primitive::VectorImage(image) => {
                image.order = order;
                self.vector_images.push(image.clone());
            }
            Primitive::VectorSvg(svg) => {
                svg.order = order;
                self.vector_svgs.push(svg.clone());
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
        }
        self.paint_operations
            .push(PaintOperation::Primitive(primitive));
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
            }
        }
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.glyph_runs.sort_by_key(|glyph_run| glyph_run.order);
        self.vector_images.sort_by_key(|image| image.order);
        self.vector_svgs.sort_by_key(|svg| svg.order);
        self.surfaces.sort_by_key(|surface| surface.order);
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            glyph_runs_start: 0,
            glyph_runs_iter: self.glyph_runs.iter().peekable(),
            vector_images_start: 0,
            vector_images_iter: self.vector_images.iter().peekable(),
            vector_svgs_start: 0,
            vector_svgs_iter: self.vector_svgs.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Path,
    Underline,
    GlyphRun,
    VectorImage,
    VectorSvg,
    Surface,
}

pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
}

#[derive(Clone)]
#[expect(missing_docs)]
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    GlyphRun(VectorGlyphRun),
    VectorImage(VectorImage),
    VectorSvg(VectorSvg),
    Surface(PaintSurface),
}

#[expect(missing_docs)]
impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::GlyphRun(glyph_run) => &glyph_run.bounds,
            Primitive::VectorImage(image) => &image.bounds,
            Primitive::VectorSvg(svg) => &svg.bounds,
            Primitive::Surface(surface) => &surface.bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::GlyphRun(glyph_run) => &glyph_run.content_mask,
            Primitive::VectorImage(image) => &image.content_mask,
            Primitive::VectorSvg(svg) => &svg.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    glyph_runs_start: usize,
    glyph_runs_iter: Peekable<slice::Iter<'a, VectorGlyphRun>>,
    vector_images_start: usize,
    vector_images_iter: Peekable<slice::Iter<'a, VectorImage>>,
    vector_svgs_start: usize,
    vector_svgs_iter: Peekable<slice::Iter<'a, VectorSvg>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.glyph_runs_iter.peek().map(|run| run.order),
                PrimitiveKind::GlyphRun,
            ),
            (
                self.vector_images_iter.peek().map(|image| image.order),
                PrimitiveKind::VectorImage,
            ),
            (
                self.vector_svgs_iter.peek().map(|svg| svg.order),
                PrimitiveKind::VectorSvg,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::GlyphRun => {
                let glyph_runs_start = self.glyph_runs_start;
                let mut glyph_runs_end = glyph_runs_start + 1;
                self.glyph_runs_iter.next();
                while self
                    .glyph_runs_iter
                    .next_if(|run| (run.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    glyph_runs_end += 1;
                }
                self.glyph_runs_start = glyph_runs_end;
                Some(PrimitiveBatch::GlyphRuns(glyph_runs_start..glyph_runs_end))
            }
            PrimitiveKind::VectorImage => {
                let images_start = self.vector_images_start;
                let mut images_end = images_start + 1;
                self.vector_images_iter.next();
                while self
                    .vector_images_iter
                    .next_if(|image| (image.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    images_end += 1;
                }
                self.vector_images_start = images_end;
                Some(PrimitiveBatch::VectorImages(images_start..images_end))
            }
            PrimitiveKind::VectorSvg => {
                let svgs_start = self.vector_svgs_start;
                let mut svgs_end = svgs_start + 1;
                self.vector_svgs_iter.next();
                while self
                    .vector_svgs_iter
                    .next_if(|svg| (svg.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    svgs_end += 1;
                }
                self.vector_svgs_start = svgs_end;
                Some(PrimitiveBatch::VectorSvgs(svgs_start..svgs_end))
            }

            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[allow(missing_docs)]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    GlyphRuns(Range<usize>),
    VectorImages(Range<usize>),
    VectorSvgs(Range<usize>),
    Surfaces(Range<usize>),
}

#[derive(Default, Debug, Copy, Clone)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Copy, Clone)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: bool,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

/// A positioned glyph in a renderer-independent vector glyph run.
#[derive(Clone, Copy, Debug)]
pub struct VectorGlyph {
    /// The glyph identifier in the run's font.
    pub id: crate::GlyphId,
    /// The device-space baseline position of the glyph.
    pub position: Point<ScaledPixels>,
}

/// A run of positioned glyph outlines ready for vector rendering.
#[derive(Clone, Debug)]
pub struct VectorGlyphRun {
    /// The scene draw order.
    pub order: DrawOrder,
    /// Bounds used for culling and ordering.
    pub bounds: Bounds<ScaledPixels>,
    /// The rectangular content mask active while painting.
    pub content_mask: ContentMask<ScaledPixels>,
    /// Foreground color used by monochrome and foreground-color glyphs.
    pub color: Hsla,
    /// The exact font resource and collection index used during shaping.
    pub font: linebender_resource_handle::FontData,
    /// Font size in device pixels per em.
    pub font_size: ScaledPixels,
    /// Normalized variable-font coordinates in F2Dot14 representation.
    pub normalized_coords: Arc<[i16]>,
    /// Positioned glyphs in this run.
    pub glyphs: Arc<[VectorGlyph]>,
}

impl From<VectorGlyphRun> for Primitive {
    fn from(glyph_run: VectorGlyphRun) -> Self {
        Primitive::GlyphRun(glyph_run)
    }
}

/// An image retained in the scene for direct renderer consumption.
#[derive(Clone, Debug)]
pub struct VectorImage {
    /// The scene draw order.
    pub order: DrawOrder,
    /// Destination bounds in device pixels.
    pub bounds: Bounds<ScaledPixels>,
    /// The rectangular content mask active while painting.
    pub content_mask: ContentMask<ScaledPixels>,
    /// Destination corner radii.
    pub corner_radii: Corners<ScaledPixels>,
    /// Decoded image frames.
    pub image: Arc<crate::RenderImage>,
    /// Frame to draw.
    pub frame_index: usize,
    /// Whether the image should be rendered in grayscale.
    pub grayscale: bool,
    /// Opacity multiplier.
    pub opacity: f32,
}

impl From<VectorImage> for Primitive {
    fn from(image: VectorImage) -> Self {
        Primitive::VectorImage(image)
    }
}

/// An SVG tree retained in the scene for direct vector rendering.
#[derive(Clone)]
pub struct VectorSvg {
    /// The scene draw order.
    pub order: DrawOrder,
    /// Destination bounds in device pixels.
    pub bounds: Bounds<ScaledPixels>,
    /// The rectangular content mask active while painting.
    pub content_mask: ContentMask<ScaledPixels>,
    /// The parsed SVG tree.
    pub tree: Arc<usvg::Tree>,
    /// Foreground color applied to the SVG's alpha coverage.
    pub color: Hsla,
    /// Transform applied after placing the SVG in its destination bounds.
    pub transformation: TransformationMatrix,
}

impl From<VectorSvg> for Primitive {
    fn from(svg: VectorSvg) -> Self {
        Primitive::VectorSvg(svg)
    }
}

#[derive(Debug, Copy, Clone)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    /// Whether this shadow is rendered inside the element.
    pub inset: bool,
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Apply transformation to a point, mainly useful for debugging
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    #[cfg(target_os = "macos")]
    pub image_buffer: core_video::pixel_buffer::CVPixelBuffer,
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A renderer-independent command in a vector path.
#[derive(Clone, Debug, PartialEq)]
pub enum PathCommand<P: Clone + Debug + Default + PartialEq> {
    /// Starts a new contour.
    MoveTo(Point<P>),
    /// Draws a straight line from the current point.
    LineTo(Point<P>),
    /// Draws a quadratic Bézier from the current point.
    QuadTo {
        /// The quadratic control point.
        control: Point<P>,
        /// The destination point.
        to: Point<P>,
    },
    /// Closes the current contour.
    Close,
}

impl PathCommand<Pixels> {
    fn scale(&self, factor: f32) -> PathCommand<ScaledPixels> {
        match self {
            Self::MoveTo(point) => PathCommand::MoveTo(point.scale(factor)),
            Self::LineTo(point) => PathCommand::LineTo(point.scale(factor)),
            Self::QuadTo { control, to } => PathCommand::QuadTo {
                control: control.scale(factor),
                to: to.scale(factor),
            },
            Self::Close => PathCommand::Close,
        }
    }
}

/// A vector path represented by source drawing commands.
#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub commands: Vec<PathCommand<P>>,
    pub color: Background,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            commands: vec![PathCommand::MoveTo(start)],
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            commands: self
                .commands
                .iter()
                .map(|command| command.scale(factor))
                .collect(),
            color: self.color,
        }
    }

    /// Start a new contour at the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.extend_bounds([to]);
        self.commands.push(PathCommand::MoveTo(to));
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.extend_bounds([to]);
        self.commands.push(PathCommand::LineTo(to));
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.extend_bounds([ctrl, to]);
        self.commands
            .push(PathCommand::QuadTo { control: ctrl, to });
    }

    /// Append a filled triangle contour to the path.
    pub fn push_triangle(&mut self, xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>)) {
        self.extend_bounds([xy.0, xy.1, xy.2]);
        self.commands.push(PathCommand::MoveTo(xy.0));
        self.commands.push(PathCommand::LineTo(xy.1));
        self.commands.push(PathCommand::LineTo(xy.2));
        self.commands.push(PathCommand::Close);
    }

    fn extend_bounds(&mut self, points: impl IntoIterator<Item = Point<Pixels>>) {
        for point in points {
            self.bounds = self.bounds.union(&Bounds {
                origin: point,
                size: Default::default(),
            });
        }
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{point, px};
    use linebender_resource_handle::{Blob, FontData};

    fn glyph_run(order: DrawOrder) -> VectorGlyphRun {
        VectorGlyphRun {
            order,
            bounds: Bounds::default(),
            content_mask: ContentMask::default(),
            color: Hsla::default(),
            font: FontData::new(Blob::new(Arc::new(Vec::<u8>::new())), 0),
            font_size: ScaledPixels(16.),
            normalized_coords: Arc::from([]),
            glyphs: Arc::from([]),
        }
    }

    fn vector_svg(order: DrawOrder) -> VectorSvg {
        let tree = usvg::Tree::from_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#,
            &usvg::Options::default(),
        )
        .unwrap();
        VectorSvg {
            order,
            bounds: Bounds::default(),
            content_mask: ContentMask::default(),
            tree: Arc::new(tree),
            color: Hsla::default(),
            transformation: TransformationMatrix::unit(),
        }
    }

    #[test]
    fn path_commands_preserve_quadratics_contours_and_scaling() {
        let mut path = Path::new(point(px(1.0), px(2.0)));
        path.line_to(point(px(3.0), px(4.0)));
        path.curve_to(point(px(7.0), px(8.0)), point(px(5.0), px(6.0)));
        path.move_to(point(px(9.0), px(10.0)));
        path.line_to(point(px(11.0), px(12.0)));

        let scaled = path.scale(2.0);
        assert_eq!(
            scaled.commands,
            vec![
                PathCommand::MoveTo(point(ScaledPixels(2.0), ScaledPixels(4.0))),
                PathCommand::LineTo(point(ScaledPixels(6.0), ScaledPixels(8.0))),
                PathCommand::QuadTo {
                    control: point(ScaledPixels(10.0), ScaledPixels(12.0)),
                    to: point(ScaledPixels(14.0), ScaledPixels(16.0)),
                },
                PathCommand::MoveTo(point(ScaledPixels(18.0), ScaledPixels(20.0))),
                PathCommand::LineTo(point(ScaledPixels(22.0), ScaledPixels(24.0))),
            ]
        );
    }

    #[test]
    fn vector_primitives_participate_in_batch_order_and_clear() {
        let mut scene = Scene::default();
        scene.quads.push(Quad {
            order: 1,
            ..Default::default()
        });
        scene.glyph_runs.push(glyph_run(2));
        scene.vector_svgs.push(vector_svg(3));

        let batches = scene.batches().collect::<Vec<_>>();
        assert!(matches!(batches[0], PrimitiveBatch::Quads(_)));
        assert!(matches!(batches[1], PrimitiveBatch::GlyphRuns(_)));
        assert!(matches!(batches[2], PrimitiveBatch::VectorSvgs(_)));

        scene.clear();
        assert!(scene.glyph_runs.is_empty());
        assert!(scene.vector_svgs.is_empty());
        assert!(scene.batches().next().is_none());
    }
}
