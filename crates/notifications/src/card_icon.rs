use crate::image::{Image, ImageData};
use crate::style::{color, size};
use mecha_wayland::prelude::*;

const MAX_IMAGE_WIDTH: u32 = 200;
const MAX_IMAGE_HEIGHT: u32 = 100;

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
        *s.component_mut::<LayoutStyle>(me).unwrap() = square().relative();

        let holder = s.spawn(me, div().style(square()));

        let sprite = b
            .image
            .and_then(|img| load(&mut s.resource_mut::<Atlas>(), &img));
        // TODO: app glyph from `desktop-entry`, centred in the tile.
        let slot = match sprite {
            Some(sprite) => s.spawn(holder, image(sprite).style(square())).id(),
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
                Some(sprite) => ctx.spawn(holder, image(sprite).style(square())).id(),
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
        .insert(Class::Image, &cap(bitmap).fit(slot))
        .map_err(|e| e.to_string())
}

fn cap(bitmap: Bitmap) -> Bitmap {
    let (mut w, mut h) = (bitmap.width, bitmap.height);
    while w > MAX_IMAGE_WIDTH || h > MAX_IMAGE_HEIGHT {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    bitmap.fit(w.max(h))
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
