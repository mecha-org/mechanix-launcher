use mecha_wayland::prelude::*;

use crate::style::{color, size};

pub struct AlbumArt {
    holder: NodeId,
    slot: NodeId,
    url: Option<String>,
}

pub struct AlbumArtBuilder {
    pub url: Option<String>,
}

impl Build for AlbumArtBuilder {
    type Widget = AlbumArt;
}

impl Widget for AlbumArt {
    type Builder = AlbumArtBuilder;

    fn build(b: AlbumArtBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = square();

        let tile = s.color(color::ICON_TILE);
        let holder = s
            .spawn(
                me,
                div()
                    .background(tile)
                    .radius(size::MEDIA_ART_RADIUS)
                    .style(square().center()),
            )
            .id();
        let sprite = b
            .url
            .as_deref()
            .and_then(|url| load(&mut s.resource_mut::<Atlas>(), url));
        let slot = match sprite {
            Some(sprite) => {
                let style = contained(&s.resource::<Atlas>(), sprite);
                s.spawn(
                    holder,
                    image(sprite).radius(size::MEDIA_ART_RADIUS).style(style),
                )
                .id()
            }
            None => s.spawn(holder, div()).id(),
        };

        AlbumArt {
            holder,
            slot,
            url: b.url,
        }
    }
}

pub trait AlbumArtExt {
    fn set_art(&mut self, url: Option<String>);
}

impl AlbumArtExt for Context<'_, AlbumArt> {
    fn set_art(&mut self, url: Option<String>) {
        if self.me().url == url {
            return;
        }
        let sprite = url
            .as_deref()
            .and_then(|url| load(&mut self.resource_mut::<Atlas>(), url));

        let (holder, old) = (self.me().holder, self.me().slot);
        self.remove(old);
        let slot = match sprite {
            Some(sprite) => {
                let style = contained(&self.resource::<Atlas>(), sprite);
                self.spawn(
                    holder,
                    image(sprite).radius(size::MEDIA_ART_RADIUS).style(style),
                )
                .id()
            }
            None => self.spawn(holder, div()).id(),
        };
        self.me().slot = slot;
        self.me().url = url;
    }
}

fn square() -> LayoutStyle {
    LayoutStyle::default()
        .size(px(size::MEDIA_ART), px(size::MEDIA_ART))
        .shrink(0.0)
}

/// The sprite's shape, scaled to fit inside the square.
fn contained(atlas: &Atlas, sprite: SpriteId) -> LayoutStyle {
    let natural = atlas.sprite(sprite).size;
    let k = size::MEDIA_ART / natural.width.max(natural.height);
    LayoutStyle::default()
        .size(px(natural.width * k), px(natural.height * k))
        .shrink(0.0)
}

// TODO: Add support for https based URI
fn load(atlas: &mut Atlas, url: &str) -> Option<SpriteId> {
    let path = url.strip_prefix("file://")?;
    let bytes = std::fs::read(path)
        .map_err(|e| eprintln!("[media] album art {path}: {e}"))
        .ok()?;
    if path.ends_with(".svg") {
        return atlas
            .insert_svg_image(bytes, size::MEDIA_ART as u32)
            .map_err(|e| eprintln!("[media] album art {path}: {e}"))
            .ok();
    }
    let bitmap = Bitmap::from_png(&bytes)
        .map_err(|e| eprintln!("[media] album art {path}: {e}"))
        .ok()?;

    let slot = (size::MEDIA_ART * atlas.scale()).ceil() as u32;
    atlas
        .insert(Class::Image, &bitmap.fit(2 * slot - 1))
        .map_err(|e| eprintln!("[media] album art {path}: {e}"))
        .ok()
}
