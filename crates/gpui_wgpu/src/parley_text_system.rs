use anyhow::{Context as _, Result};
use collections::HashMap;
use fontique::{
    Attributes, Blob, Collection, CollectionOptions, FontStyle as ParleyFontStyle,
    FontWeight as ParleyFontWeight, QueryStatus, SourceCache,
};
use gpui::{
    Bounds, Font, FontId, FontMetrics, FontRun, GlyphId, LineLayout, Pixels, PlatformTextSystem,
    ShapedGlyph, ShapedRun, Size, point, size,
};
use parking_lot::RwLock;
use parley::{
    FontContext, FontData, FontFamilyName, FontFeature, FontFeatures, LayoutContext,
    PositionedLayoutItem, StyleProperty, setting::Tag,
};
use skrifa::{
    MetadataProvider as _,
    instance::{LocationRef, NormalizedCoord, Size as SkrifaSize},
};
use std::{borrow::Cow, sync::Arc};

/// An experimental `PlatformTextSystem` backed by Parley and Fontique.
///
pub struct ParleyTextSystem(RwLock<ParleyTextSystemState>);

struct ParleyTextSystemState {
    font_context: FontContext,
    layout_context: LayoutContext<usize>,
    loaded_fonts: Vec<LoadedFont>,
    loaded_font_ids: HashMap<LoadedFontKey, FontId>,
    system_font_fallback: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LoadedFontKey {
    font_data_id: u64,
    font_index: u32,
    family: String,
    attributes: FontAttributesKey,
    fallbacks: Vec<String>,
    features: Vec<FontFeatureKey>,
    normalized_coords: Arc<[i16]>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FontAttributesKey {
    width_bits: u32,
    style: FontStyleKey,
    weight_bits: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum FontStyleKey {
    Normal,
    Italic,
    Oblique(Option<u32>),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FontFeatureKey {
    tag: [u8; 4],
    value: u16,
}

struct LoadedFont {
    data: FontData,
    family: String,
    attributes: Attributes,
    fallbacks: Vec<String>,
    features: Vec<FontFeature>,
    normalized_coords: Arc<[i16]>,
    skrifa_normalized_coords: Arc<[NormalizedCoord]>,
}

impl LoadedFontKey {
    fn new(
        data: &FontData,
        family: &str,
        attributes: Attributes,
        fallbacks: &[String],
        features: &[FontFeature],
        normalized_coords: &Arc<[i16]>,
    ) -> Self {
        Self {
            font_data_id: data.data.id(),
            font_index: data.index,
            family: family.to_owned(),
            attributes: attributes.into(),
            fallbacks: fallbacks.to_vec(),
            features: features.iter().copied().map(Into::into).collect(),
            normalized_coords: normalized_coords.clone(),
        }
    }
}

impl From<Attributes> for FontAttributesKey {
    fn from(attributes: Attributes) -> Self {
        Self {
            width_bits: attributes.width.ratio().to_bits(),
            style: attributes.style.into(),
            weight_bits: attributes.weight.value().to_bits(),
        }
    }
}

impl From<ParleyFontStyle> for FontStyleKey {
    fn from(style: ParleyFontStyle) -> Self {
        match style {
            ParleyFontStyle::Normal => Self::Normal,
            ParleyFontStyle::Italic => Self::Italic,
            ParleyFontStyle::Oblique(angle) => Self::Oblique(angle.map(f32::to_bits)),
        }
    }
}

impl From<FontFeature> for FontFeatureKey {
    fn from(feature: FontFeature) -> Self {
        Self {
            tag: feature.tag.to_bytes(),
            value: feature.value,
        }
    }
}

impl LoadedFont {
    fn skrifa_font(&self) -> Result<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(self.data.data.as_ref(), self.data.index)
            .context("invalid font data")
    }

    fn skrifa_location(&self) -> LocationRef<'_> {
        LocationRef::new(&self.skrifa_normalized_coords)
    }
}

impl ParleyTextSystem {
    pub fn new(system_font_fallback: &str) -> Self {
        Self::with_system_fonts(system_font_fallback, true)
    }

    pub fn new_without_system_fonts(system_font_fallback: &str) -> Self {
        Self::with_system_fonts(system_font_fallback, false)
    }

    fn with_system_fonts(system_font_fallback: &str, system_fonts: bool) -> Self {
        let collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts,
        });
        Self(RwLock::new(ParleyTextSystemState {
            font_context: FontContext {
                collection,
                source_cache: SourceCache::default(),
            },
            layout_context: LayoutContext::new(),
            loaded_fonts: Vec::new(),
            loaded_font_ids: HashMap::default(),
            system_font_fallback: system_font_fallback.to_owned(),
        }))
    }
}

impl PlatformTextSystem for ParleyTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        let mut state = self.0.write();
        for font in fonts {
            state
                .font_context
                .collection
                .register_fonts(Blob::from(font.into_owned()), None);
        }
        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut state = self.0.write();
        let mut names = state
            .font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        names
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        self.0.write().font_id(descriptor)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.0.read().font_metrics(font_id)
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let state = self.0.read();
        let loaded = state.loaded_font(font_id)?;
        let font = loaded.skrifa_font()?;
        let glyph_id = skrifa::GlyphId::new(glyph_id.0);
        let metrics = font.glyph_metrics(SkrifaSize::unscaled(), loaded.skrifa_location());
        Ok(Bounds {
            origin: point(0.0, 0.0),
            size: size(metrics.advance_width(glyph_id).unwrap_or_default(), 0.0),
        })
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        let state = self.0.read();
        let loaded = state.loaded_font(font_id)?;
        let font = loaded.skrifa_font()?;
        let glyph_id = skrifa::GlyphId::new(glyph_id.0);
        Ok(size(
            font.glyph_metrics(SkrifaSize::unscaled(), loaded.skrifa_location())
                .advance_width(glyph_id)
                .context("invalid glyph id")?,
            0.0,
        ))
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        let state = self.0.read();
        state
            .loaded_font(font_id)
            .ok()?
            .skrifa_font()
            .ok()?
            .charmap()
            .map(ch)
            .map(|glyph| GlyphId(glyph.to_u32()))
    }

    fn font_render_data(&self, font_id: FontId) -> Option<gpui::FontRenderData> {
        let state = self.0.read();
        let loaded = state.loaded_fonts.get(font_id.0)?;
        Some(gpui::FontRenderData {
            font: loaded.data.clone(),
            normalized_coords: loaded.normalized_coords.clone(),
        })
    }

    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        self.0.write().layout_line(text, font_size, runs)
    }
}

impl ParleyTextSystemState {
    fn loaded_font(&self, font_id: FontId) -> Result<&LoadedFont> {
        self.loaded_fonts
            .get(font_id.0)
            .context("unknown Parley font id")
    }

    fn font_id(&mut self, descriptor: &Font) -> Result<FontId> {
        let family =
            gpui::font_name_with_fallbacks(descriptor.family.as_ref(), &self.system_font_fallback);
        let attributes = Attributes {
            width: Default::default(),
            style: match descriptor.style {
                gpui::FontStyle::Normal => ParleyFontStyle::Normal,
                gpui::FontStyle::Italic => ParleyFontStyle::Italic,
                gpui::FontStyle::Oblique => ParleyFontStyle::Oblique(None),
            },
            weight: ParleyFontWeight::new(descriptor.weight.0),
        };
        let mut selected = None;
        let mut query = self
            .font_context
            .collection
            .query(&mut self.font_context.source_cache);
        query.set_families([family]);
        query.set_attributes(attributes);
        query.matches_with(|font| {
            selected = Some(FontData::new(font.blob.clone(), font.index));
            QueryStatus::Stop
        });
        drop(query);

        let data = selected.with_context(|| format!("font family {family:?} was not found"))?;
        let fallbacks = descriptor
            .fallbacks
            .as_ref()
            .map(|fallbacks| fallbacks.fallback_list().to_vec())
            .unwrap_or_default();
        let features = descriptor
            .features
            .tag_value_list()
            .iter()
            .map(|(tag, value)| {
                Ok(FontFeature::new(
                    Tag::parse(tag)
                        .with_context(|| format!("invalid OpenType feature tag {tag:?}"))?,
                    u16::try_from(*value)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(self.ensure_loaded(
            data,
            family.to_owned(),
            attributes,
            fallbacks,
            features,
            Arc::from([]),
        ))
    }

    fn ensure_loaded(
        &mut self,
        data: FontData,
        family: String,
        attributes: Attributes,
        fallbacks: Vec<String>,
        features: Vec<FontFeature>,
        normalized_coords: Arc<[i16]>,
    ) -> FontId {
        let key = LoadedFontKey::new(
            &data,
            &family,
            attributes,
            &fallbacks,
            &features,
            &normalized_coords,
        );
        if let Some(id) = self.loaded_font_ids.get(&key) {
            return *id;
        }
        let skrifa_normalized_coords = Arc::from(
            normalized_coords
                .iter()
                .copied()
                .map(NormalizedCoord::from_bits)
                .collect::<Vec<_>>(),
        );
        let id = FontId(self.loaded_fonts.len());
        self.loaded_fonts.push(LoadedFont {
            data,
            family,
            attributes,
            fallbacks,
            features,
            normalized_coords,
            skrifa_normalized_coords,
        });
        self.loaded_font_ids.insert(key, id);
        id
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        let Ok(loaded) = self.loaded_font(font_id) else {
            return FontMetrics {
                units_per_em: 0,
                ascent: 0.0,
                descent: 0.0,
                line_gap: 0.0,
                underline_position: 0.0,
                underline_thickness: 0.0,
                cap_height: 0.0,
                x_height: 0.0,
                bounding_box: Bounds::default(),
            };
        };
        let Ok(font) = loaded.skrifa_font() else {
            return FontMetrics {
                units_per_em: 0,
                ascent: 0.0,
                descent: 0.0,
                line_gap: 0.0,
                underline_position: 0.0,
                underline_thickness: 0.0,
                cap_height: 0.0,
                x_height: 0.0,
                bounding_box: Bounds::default(),
            };
        };
        let metrics = font.metrics(SkrifaSize::unscaled(), loaded.skrifa_location());
        let underline = metrics.underline.unwrap_or_default();
        let bounds = metrics.bounds.unwrap_or_default();
        FontMetrics {
            units_per_em: metrics.units_per_em.into(),
            ascent: metrics.ascent,
            descent: metrics.descent,
            line_gap: metrics.leading,
            underline_position: underline.offset,
            underline_thickness: underline.thickness,
            cap_height: metrics.cap_height.unwrap_or_default(),
            x_height: metrics.x_height.unwrap_or_default(),
            bounding_box: Bounds {
                origin: point(bounds.x_min, bounds.y_min),
                size: size(bounds.x_max - bounds.x_min, bounds.y_max - bounds.y_min),
            },
        }
    }

    fn layout_line(&mut self, text: &str, font_size: Pixels, font_runs: &[FontRun]) -> LineLayout {
        if text.is_empty() {
            return LineLayout {
                font_size,
                width: Pixels::ZERO,
                ascent: Pixels::ZERO,
                descent: Pixels::ZERO,
                runs: Vec::new(),
                len: 0,
            };
        }

        let families = font_runs
            .iter()
            .map(|run| {
                let loaded = &self.loaded_fonts[run.font_id.0];
                let mut names = Vec::with_capacity(1 + loaded.fallbacks.len());
                names.push(loaded.family.as_str());
                names.extend(loaded.fallbacks.iter().map(String::as_str));
                names
                    .into_iter()
                    .map(|name| FontFamilyName::Named(Cow::Borrowed(name)))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let mut builder =
            self.layout_context
                .ranged_builder(&mut self.font_context, text, 1.0, false);
        builder.push_default(StyleProperty::FontSize(f32::from(font_size)));
        let mut offset = 0;
        for (run, family_names) in font_runs.iter().zip(&families) {
            let end = (offset + run.len).min(text.len());
            let range = offset..end;
            let loaded = &self.loaded_fonts[run.font_id.0];
            builder.push(
                StyleProperty::FontFamily(family_names.as_slice().into()),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontWeight(loaded.attributes.weight),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontStyle(loaded.attributes.style),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(&loaded.features))),
                range.clone(),
            );
            builder.push(StyleProperty::Brush(run.font_id.0), range);
            offset = end;
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        let Some(line) = layout.lines().next() else {
            return LineLayout {
                font_size,
                width: Pixels::ZERO,
                ascent: Pixels::ZERO,
                descent: Pixels::ZERO,
                runs: Vec::new(),
                len: text.len(),
            };
        };
        let metrics = *line.metrics();
        let mut shaped_runs: Vec<ShapedRun> = Vec::new();
        let mut glyph_offsets_by_run = HashMap::default();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let data = glyph_run.run().font().clone();
            let requested_id = FontId(glyph_run.style().brush);
            let requested = &self.loaded_fonts[requested_id.0];
            let resolved_id = self.ensure_loaded(
                data,
                requested.family.clone(),
                *glyph_run.run().font_attrs(),
                Vec::new(),
                requested.features.clone(),
                Arc::from(glyph_run.run().normalized_coords()),
            );
            let glyph_count = glyph_run.glyphs().count();
            let glyph_offset = glyph_offsets_by_run
                .entry(glyph_run.run().index())
                .or_insert(0);
            let cluster_indices = glyph_run
                .run()
                .visual_clusters()
                .flat_map(|cluster| {
                    let index = cluster.text_range().start;
                    let is_emoji = cluster.is_emoji();
                    cluster.glyphs().map(move |_| (index, is_emoji))
                })
                .skip(*glyph_offset)
                .take(glyph_count);
            *glyph_offset += glyph_count;
            let glyphs = glyph_run
                .positioned_glyphs()
                .zip(cluster_indices)
                .map(|(glyph, (index, is_emoji))| ShapedGlyph {
                    id: GlyphId(glyph.id),
                    position: point(glyph.x.into(), (glyph.y - metrics.baseline).into()),
                    index,
                    is_emoji,
                })
                .collect::<Vec<_>>();
            if let Some(last) = shaped_runs
                .last_mut()
                .filter(|run| run.font_id == resolved_id)
            {
                last.glyphs.extend(glyphs);
            } else {
                shaped_runs.push(ShapedRun {
                    font_id: resolved_id,
                    glyphs,
                });
            }
        }
        LineLayout {
            font_size,
            width: layout.full_width().into(),
            ascent: metrics.ascent.into(),
            descent: metrics.descent.into(),
            runs: shaped_runs,
            len: text.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{font, px};

    const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");
    const IBM_PLEX: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");

    fn test_system() -> ParleyTextSystem {
        let system = ParleyTextSystem::new_without_system_fonts("IBM Plex Sans");
        system
            .add_fonts(vec![Cow::Borrowed(LILEX), Cow::Borrowed(IBM_PLEX)])
            .unwrap();
        system
    }

    fn assert_same_face_has_distinct_font_ids(
        system: &ParleyTextSystem,
        first: &Font,
        second: &Font,
    ) {
        let first_id = system.font_id(first).expect("first font should resolve");
        let second_id = system.font_id(second).expect("second font should resolve");
        let first_render_data = system
            .font_render_data(first_id)
            .expect("first font should have render data");
        let second_render_data = system
            .font_render_data(second_id)
            .expect("second font should have render data");

        assert_eq!(first_render_data.font, second_render_data.font);
        assert_ne!(first_id, second_id);
    }

    #[test]
    fn open_type_features_are_part_of_font_identity() {
        let system = test_system();
        let default_font = font("Lilex");
        let mut font_without_ligatures = default_font.clone();
        font_without_ligatures.features = gpui::FontFeatures::disable_ligatures();

        assert_same_face_has_distinct_font_ids(&system, &default_font, &font_without_ligatures);
    }

    #[test]
    fn fallbacks_are_part_of_font_identity() {
        let system = test_system();
        let default_font = font("Lilex");
        let mut font_with_fallback = default_font.clone();
        font_with_fallback.fallbacks = Some(gpui::FontFallbacks::from_fonts(vec![
            "IBM Plex Sans".to_owned(),
        ]));

        assert_same_face_has_distinct_font_ids(&system, &default_font, &font_with_fallback);
    }

    #[test]
    fn attributes_are_part_of_font_identity() {
        let system = test_system();
        let default_font = font("Lilex");
        let mut medium_font = default_font.clone();
        medium_font.weight = gpui::FontWeight::MEDIUM;

        assert_same_face_has_distinct_font_ids(&system, &default_font, &medium_font);
    }

    #[test]
    fn skrifa_location_uses_loaded_normalized_coordinates() {
        let system = test_system();
        let default_font_id = system
            .font_id(&font("Lilex"))
            .expect("default font should resolve");
        let mut state = system.0.write();
        let (data, family, attributes, fallbacks, features) = {
            let loaded = state
                .loaded_font(default_font_id)
                .expect("default font should be loaded");
            (
                loaded.data.clone(),
                loaded.family.clone(),
                loaded.attributes,
                loaded.fallbacks.clone(),
                loaded.features.clone(),
            )
        };
        let normalized_coords: Arc<[i16]> = Arc::from([-8192, 8192]);
        let loaded_font_id = state.ensure_loaded(
            data,
            family,
            attributes,
            fallbacks,
            features,
            normalized_coords.clone(),
        );
        let loaded = state
            .loaded_font(loaded_font_id)
            .expect("font with normalized coordinates should be loaded");
        let skrifa_coords = loaded
            .skrifa_location()
            .coords()
            .iter()
            .map(|coordinate| coordinate.to_bits())
            .collect::<Vec<_>>();

        assert_eq!(skrifa_coords.as_slice(), normalized_coords.as_ref());
    }

    #[test]
    fn font_metrics_use_signed_descent() {
        let system = test_system();
        let font_id = system.font_id(&font("Lilex")).unwrap();
        let metrics = system.font_metrics(font_id);

        assert!(metrics.descent < 0.0);
    }

    #[test]
    fn registered_fonts_can_be_resolved_and_shaped() {
        let system = test_system();
        let font_id = system.font_id(&font("Lilex")).unwrap();
        let text = "Hello, Parley!";
        let layout = system.layout_line(
            text,
            px(16.),
            &[FontRun {
                len: text.len(),
                font_id,
            }],
        );

        assert!(layout.width > Pixels::ZERO);
        assert!(!layout.runs.is_empty());
        assert!(layout.runs.iter().any(|run| !run.glyphs.is_empty()));
        assert!(
            layout
                .runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .all(|glyph| glyph.index < text.len())
        );
    }

    #[test]
    fn shaped_glyph_ids_match_the_resolved_font() {
        let system = test_system();
        let font_id = system.font_id(&font("Lilex")).unwrap();
        let text = "use";
        let layout = system.layout_line(
            text,
            px(16.0),
            &[FontRun {
                len: text.len(),
                font_id,
            }],
        );
        let glyphs = layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .collect::<Vec<_>>();
        let glyph_ids = glyphs.iter().map(|glyph| glyph.id).collect::<Vec<_>>();
        let state = system.0.read();
        let charmap = state
            .loaded_font(font_id)
            .expect("registered font should remain loaded")
            .skrifa_font()
            .expect("registered font data should be valid")
            .charmap();
        let expected = text
            .chars()
            .map(|character| {
                GlyphId(
                    charmap
                        .map(character)
                        .expect("test font should contain the character")
                        .to_u32(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(glyph_ids, expected);
        assert_eq!(
            glyphs
                .first()
                .expect("test text should produce a first glyph")
                .position
                .x,
            px(0.0)
        );
    }

    #[test]
    fn glyph_indices_follow_positioned_style_segments() {
        let system = test_system();
        let default_font = font("Lilex");
        let default_font_id = system.font_id(&default_font).unwrap();
        let mut font_without_ligatures = default_font;
        font_without_ligatures.features = gpui::FontFeatures::disable_ligatures();
        let font_without_ligatures_id = system.font_id(&font_without_ligatures).unwrap();
        let text = "abCD";
        let layout = system.layout_line(
            text,
            px(16.0),
            &[
                FontRun {
                    len: 2,
                    font_id: default_font_id,
                },
                FontRun {
                    len: 2,
                    font_id: font_without_ligatures_id,
                },
            ],
        );
        let indices = layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index))
            .collect::<Vec<_>>();

        assert_eq!(indices, vec![0, 1, 2, 3]);
    }

    #[test]
    fn resolved_font_data_is_shared_with_vello() {
        let system = test_system();
        let font_id = system.font_id(&font("IBM Plex Sans")).unwrap();
        let render_data = system
            .font_render_data(font_id)
            .expect("Vello builds expose exact font data");

        assert!(!render_data.font.data.as_ref().is_empty());
        assert_eq!(render_data.font.index, 0);
    }

    #[test]
    fn font_names_include_registered_families() {
        let system = test_system();
        let names = system.all_font_names();
        assert!(names.iter().any(|name| name == "Lilex"));
        assert!(names.iter().any(|name| name == "IBM Plex Sans"));
    }
}
