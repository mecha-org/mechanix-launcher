use crate::image::{Image, ImageData};
use crate::style::{color, size};
use mecha_wayland::prelude::*;

pub struct CardIconBuilder {
    pub image: Option<Image>,
}

pub struct UpdateIcon(pub Option<Image>);
impl Event for UpdateIcon {}

pub struct CardIcon {
    slot: NodeId,
}

fn square() -> LayoutStyle {
    LayoutStyle::default()
        .size(px(size::ICON), px(size::ICON))
        .shrink(0.0)
}

impl Build for CardIconBuilder {
    type Widget = CardIcon;
}

impl Widget for CardIcon {
    type Builder = CardIconBuilder;

    fn build(b: CardIconBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = square().relative().align_self(Align::Start);

        let holder = s.spawn(me, div().style(square().center()));

        let sprite = b
            .image
            .and_then(|img| load(&mut s.resource_mut::<Atlas>(), &img));
        // TODO: app glyph from `desktop-entry`, centred in the tile.
        let slot = match sprite {
            Some(sprite) => {
                let style = contained(&s.resource::<Atlas>(), sprite);
                s.spawn(holder, image(sprite).style(style)).id()
            }
            None => {
                let tile = s.color(color::ICON_TILE);
                s.spawn(holder, div().background(tile).style(square())).id()
            }
        };

        s.on::<UpdateIcon>(me, move |ctx, e| {
            let sprite =
                e.0.as_ref()
                    .and_then(|img| load(&mut ctx.resource_mut::<Atlas>(), img));
            let old = ctx.me().slot;
            ctx.remove(old);
            let slot = match sprite {
                Some(sprite) => {
                    let style = contained(&ctx.resource::<Atlas>(), sprite);
                    ctx.spawn(holder, image(sprite).style(style)).id()
                }
                None => {
                    let tile = ctx.color(color::ICON_TILE);
                    ctx.spawn(holder, div().background(tile).style(square()))
                        .id()
                }
            };
            ctx.me().slot = slot;
        });
        CardIcon { slot }
    }
}

fn contained(atlas: &Atlas, sprite: SpriteId) -> LayoutStyle {
    let (w, h) = fit(atlas.sprite(sprite).size, size::ICON);
    LayoutStyle::default().size(px(w), px(h)).shrink(0.0)
}

fn fit(natural: Size, side: f32) -> (f32, f32) {
    let (w, h) = (natural.width, natural.height);
    if w.max(h) <= side {
        (w, h)
    } else if w >= h {
        (side, h * side / w)
    } else {
        (w * side / h, side)
    }
}

fn load(atlas: &mut Atlas, image: &Image) -> Option<SpriteId> {
    let result = match image {
        Image::Path(path) => load_path(atlas, path),
        Image::Data(data) => to_bitmap(data)
            .ok_or_else(|| "unsupported image-data layout".to_string())
            .and_then(|bitmap| insert_fitted(atlas, bitmap)),
    };
    result
        .map_err(|e| eprintln!("notification image unusable: {e}"))
        .ok()
}

fn load_path(atlas: &mut Atlas, path: &str) -> Result<SpriteId, String> {
    let path = path.strip_prefix("file://").unwrap_or(path);
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if path.ends_with(".svg") {
        return atlas
            .insert_svg_image(bytes, size::ICON as u32)
            .map_err(|e| format!("{path}: {e}"));
    }
    let bitmap = Bitmap::from_png(&bytes).map_err(|e| format!("{path}: {e}"))?;
    insert_fitted(atlas, bitmap)
}

fn insert_fitted(atlas: &mut Atlas, bitmap: Bitmap) -> Result<SpriteId, String> {
    let slot = (size::ICON * atlas.scale()).ceil() as u32;
    atlas
        .insert(Class::Image, &shrink(bitmap, slot))
        .map_err(|e| e.to_string())
}

fn shrink(bitmap: Bitmap, slot: u32) -> Bitmap {
    let mut longest = bitmap.width.max(bitmap.height);
    while longest / 2 >= slot.max(1) {
        longest /= 2;
    }
    bitmap.fit(longest)
}

fn to_bitmap(d: &ImageData) -> Option<Bitmap> {
    let channels = match (d.has_alpha, d.channels) {
        (true, 4) => 4,
        (false, 3) => 3,
        _ => return None,
    };
    if d.bits_per_sample != 8 || d.width <= 0 || d.height <= 0 {
        return None;
    }
    let (w, h) = (d.width as usize, d.height as usize);
    let stride = usize::try_from(d.rowstride).ok()?;
    let row_len = w * channels;
    if stride < row_len || d.data.len() < stride * (h - 1) + row_len {
        return None;
    }
    let mut pixels = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let start = row * stride;
        for px in d.data[start..start + row_len].chunks_exact(channels) {
            pixels.extend_from_slice(&px[..3]);
            pixels.push(if channels == 4 { px[3] } else { 255 });
        }
    }
    Some(Bitmap {
        width: w as u32,
        height: h as u32,
        format: Format::Rgba8,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(width: u32, height: u32) -> Bitmap {
        Bitmap {
            width,
            height,
            format: Format::Rgba8,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    #[test]
    fn shrinking_never_drops_below_the_slot() {
        for (w, h, slot) in [
            (100, 100, 60),
            (256, 256, 60),
            (200, 100, 60),
            (512, 512, 120),
        ] {
            let b = shrink(bitmap(w, h), slot);
            let longest = b.width.max(b.height);
            assert!(
                longest >= slot && longest < 2 * slot,
                "{w}x{h} -> {longest} for {slot}"
            );
        }
    }

    #[test]
    fn a_wide_image_keeps_its_shape_inside_the_square() {
        assert_eq!(fit(Size::new(200.0, 100.0), 60.0), (60.0, 30.0));
        assert_eq!(fit(Size::new(50.0, 100.0), 60.0), (30.0, 60.0));
        assert_eq!(
            fit(Size::new(20.0, 10.0), 60.0),
            (20.0, 10.0),
            "never upscaled"
        );
    }

    #[test]
    fn small_images_are_kept_as_they_are() {
        let b = shrink(bitmap(32, 16), 60);
        assert_eq!((b.width, b.height), (32, 16));
    }
}
