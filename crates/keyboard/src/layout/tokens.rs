//! Resolution of `MechanixTheme` design tokens and bundled fonts by name, so
//! `layout.yaml` can refer to colours, shapes, spacing and fonts by token
//! instead of hard-coded values.

use mecha_wayland::prelude::{Atlas, ColorRole, FontId, Shape, Spacing};
use assets::fonts::GEIST_MONO as GEIST_MONO_BYTES;

/// The [`ColorRole`] for a kebab-case role name, or `None` if it isn't one.
pub fn color_role(name: &str) -> Option<ColorRole> {
    use ColorRole::*;
    Some(match name {
        "primary" => Primary,
        "on-primary" => OnPrimary,
        "primary-container" => PrimaryContainer,
        "on-primary-container" => OnPrimaryContainer,
        "secondary" => Secondary,
        "on-secondary" => OnSecondary,
        "secondary-container" => SecondaryContainer,
        "on-secondary-container" => OnSecondaryContainer,
        "error" => Error,
        "on-error" => OnError,
        "error-container" => ErrorContainer,
        "on-error-container" => OnErrorContainer,
        "success" => Success,
        "on-success" => OnSuccess,
        "success-container" => SuccessContainer,
        "on-success-container" => OnSuccessContainer,
        "primary-fixed" => PrimaryFixed,
        "primary-fixed-dim" => PrimaryFixedDim,
        "on-primary-fixed" => OnPrimaryFixed,
        "on-primary-fixed-variant" => OnPrimaryFixedVariant,
        "secondary-fixed" => SecondaryFixed,
        "secondary-fixed-dim" => SecondaryFixedDim,
        "on-secondary-fixed" => OnSecondaryFixed,
        "on-secondary-fixed-variant" => OnSecondaryFixedVariant,
        "surface-dim" => SurfaceDim,
        "surface" => Surface,
        "surface-bright" => SurfaceBright,
        "surface-container-lowest" => SurfaceContainerLowest,
        "surface-container-low" => SurfaceContainerLow,
        "surface-container" => SurfaceContainer,
        "surface-container-high" => SurfaceContainerHigh,
        "surface-container-highest" => SurfaceContainerHighest,
        "on-surface" => OnSurface,
        "on-surface-variant" => OnSurfaceVariant,
        "outline" => Outline,
        "outline-variant" => OutlineVariant,
        "inverse-surface" => InverseSurface,
        "inverse-on-surface" => InverseOnSurface,
        "inverse-primary" => InversePrimary,
        "scrim" => Scrim,
        "shadow" => Shadow,
        "background" => Background,
        "on-background" => OnBackground,
        "surface-tint" => SurfaceTint,
        _ => return None,
    })
}

/// The [`Shape`] for a kebab-case token name, or `None` if it isn't one. `full`
/// (a pill, half the component height) is excluded: keys are sized by the flex
/// layout, so their height isn't known when the radius is set.
pub fn shape(name: &str) -> Option<Shape> {
    use Shape::*;
    Some(match name {
        "none" => None,
        "extra-small" => ExtraSmall,
        "small" => Small,
        "medium" => Medium,
        "large" => Large,
        "large-increased" => LargeIncreased,
        "extra-large" => ExtraLarge,
        "extra-large-increased" => ExtraLargeIncreased,
        "extra-extra-large" => ExtraExtraLarge,
        _ => return Option::None,
    })
}

/// Every spacing token, smallest first.
const SPACING_TOKENS: [Spacing; 18] = [
    Spacing::Space0,
    Spacing::Space25,
    Spacing::Space50,
    Spacing::Space75,
    Spacing::Space100,
    Spacing::Space125,
    Spacing::Space150,
    Spacing::Space175,
    Spacing::Space200,
    Spacing::Space250,
    Spacing::Space300,
    Spacing::Space400,
    Spacing::Space450,
    Spacing::Space500,
    Spacing::Space600,
    Spacing::Space700,
    Spacing::Space800,
    Spacing::Space900,
];

/// The spacing token whose px value is exactly `px`, or `None` if there isn't one.
pub fn spacing(px: f32) -> Option<Spacing> {
    SPACING_TOKENS.into_iter().find(|s| s.dp() == px)
}

// ── fonts ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Geist,
    GeistMono,
}

impl Font {
    pub fn from_name(name: &str) -> Option<Font> {
        match name {
            "geist" => Some(Font::Geist),
            "geist-mono" => Some(Font::GeistMono),
            _ => None,
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Font::Geist => include_bytes!("../../resources/Geist-Regular.ttf"),
            Font::GeistMono => GEIST_MONO_BYTES,
        }
    }
}

/// The loaded `FontId`s for the bundled fonts, used to spawn text labels.
#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    geist: FontId,
    geist_mono: FontId,
}

impl Fonts {
    pub fn load(atlas: &mut Atlas) -> Self {
        let mut add = |font: Font| {
            atlas
                .add_font(font.bytes())
                .unwrap_or_else(|e| panic!("bundled font {font:?} loads: {e:?}"))
        };
        Fonts {
            geist: add(Font::Geist),
            geist_mono: add(Font::GeistMono),
        }
    }

    pub fn id(&self, font: Font) -> FontId {
        match font {
            Font::Geist => self.geist,
            Font::GeistMono => self.geist_mono,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_map_to_roles() {
        assert_eq!(
            color_role("surface-container-low"),
            Some(ColorRole::SurfaceContainerLow)
        );
        assert_eq!(color_role("on-surface"), Some(ColorRole::OnSurface));
    }

    #[test]
    fn unknown_name_is_none() {
        assert_eq!(color_role("surface-contianer-low"), None);
        assert_eq!(color_role("SurfaceContainerLow"), None);
    }

    #[test]
    fn known_names_map_to_shapes() {
        assert_eq!(shape("none"), Some(Shape::None));
        assert_eq!(shape("small"), Some(Shape::Small));
    }

    #[test]
    fn unknown_or_full_is_none() {
        assert_eq!(shape("round"), Option::None);
        assert_eq!(shape("full"), Option::None);
    }

    #[test]
    fn token_values_map_to_spacing() {
        assert_eq!(spacing(8.0), Some(Spacing::Space100));
        assert_eq!(spacing(12.0), Some(Spacing::Space150));
    }

    #[test]
    fn non_token_value_is_none() {
        assert_eq!(spacing(5.0), None);
        assert_eq!(spacing(8.5), None);
    }

    #[test]
    fn known_names_map_to_fonts() {
        assert_eq!(Font::from_name("geist"), Some(Font::Geist));
        assert_eq!(Font::from_name("geist-mono"), Some(Font::GeistMono));
    }

    #[test]
    fn unknown_name_is_none_font() {
        assert_eq!(Font::from_name("roboto-mono"), None);
        assert_eq!(Font::from_name("Geist"), None);
    }

    #[test]
    fn bundled_fonts_load() {
        let fonts = Fonts::load(&mut Atlas::new());
        assert_ne!(fonts.id(Font::Geist), fonts.id(Font::GeistMono));
    }
}
