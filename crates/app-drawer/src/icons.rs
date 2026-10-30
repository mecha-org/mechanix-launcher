//! Freedesktop icon lookup for `Icon=` values, and loading the result into
//! the toolkit's sprite atlas for the drawer's rows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mecha_wayland::prelude::*;

/// 48px times the largest scale we target (2.25), rounded up to 3x. PNGs are
/// fixed pixels, so pick one that stays sharp after a rescale.
pub(crate) const PNG_MIN_PX: u32 = 144;

/// Largest PNG side kept in the atlas; bigger ones are halved to fit.
const PNG_MAX_PX: u32 = 256;

/// Logical size every row icon is drawn at.
const ICON_PX: u32 = 48;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum IconFile {
    Svg(PathBuf),
    Png(PathBuf),
}

/// `gtk-icon-theme-name` from `<config_home>/gtk-3.0/settings.ini`, unquoted.
pub(crate) fn active_theme(config_home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config_home.join("gtk-3.0/settings.ini")).ok()?;
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "gtk-icon-theme-name" {
            return None;
        }
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The `Inherits=` list of `theme`, from the first `index.theme` found.
fn inherits(bases: &[PathBuf], theme: &str) -> Vec<String> {
    let Some(text) = bases
        .iter()
        .find_map(|b| std::fs::read_to_string(b.join(theme).join("index.theme")).ok())
    else {
        return Vec::new();
    };
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Inherits="))
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) struct IconDirs {
    bases: Vec<PathBuf>,
    themes: Vec<String>,
    pixmaps: Vec<PathBuf>,
}

impl IconDirs {
    pub(crate) fn new(data_dirs: &[PathBuf], active: Option<&str>) -> IconDirs {
        let bases: Vec<PathBuf> = data_dirs.iter().map(|d| d.join("icons")).collect();
        let mut pixmaps: Vec<PathBuf> = data_dirs.iter().map(|d| d.join("pixmaps")).collect();
        let system = PathBuf::from("/usr/share/pixmaps");
        if !pixmaps.contains(&system) {
            pixmaps.push(system);
        }

        let mut themes: Vec<String> = Vec::new();
        let mut queue: std::collections::VecDeque<String> =
            active.map(String::from).into_iter().collect();
        while let Some(t) = queue.pop_front() {
            if t == "hicolor" || themes.contains(&t) {
                continue;
            }
            queue.extend(inherits(&bases, &t));
            themes.push(t);
        }
        themes.push("hicolor".to_string());
        IconDirs {
            bases,
            themes,
            pixmaps,
        }
    }

    pub(crate) fn find(&self, icon: &str) -> Option<IconFile> {
        let path = Path::new(icon);
        if path.is_absolute() {
            return Some(match path.extension().and_then(|e| e.to_str()) {
                Some("svg") => IconFile::Svg(path.to_path_buf()),
                _ => IconFile::Png(path.to_path_buf()),
            });
        }
        let name = ["png", "svg", "xpm"]
            .iter()
            .find_map(|ext| icon.strip_suffix(&format!(".{ext}")))
            .unwrap_or(icon);

        for theme in &self.themes {
            if let Some(found) = self.find_in_theme(theme, name) {
                return Some(found);
            }
        }
        for dir in &self.pixmaps {
            for (ext, make) in [
                ("svg", IconFile::Svg as fn(PathBuf) -> IconFile),
                ("png", IconFile::Png),
            ] {
                let p = dir.join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(make(p));
                }
            }
        }
        None
    }

    fn find_in_theme(&self, theme: &str, name: &str) -> Option<IconFile> {
        let svg = format!("{name}.svg");
        let png = format!("{name}.png");
        let mut pngs: Vec<(u32, PathBuf)> = Vec::new();
        for base in &self.bases {
            let root = base.join(theme);
            for dir in subdirs(&root.join("scalable")) {
                let p = dir.join(&svg);
                if p.is_file() {
                    return Some(IconFile::Svg(p));
                }
            }
            for size_dir in subdirs(&root) {
                let Some(n) = size_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.split_once('x'))
                    .and_then(|(a, b)| (a == b).then(|| a.parse::<u32>().ok()).flatten())
                else {
                    continue;
                };
                for dir in subdirs(&size_dir) {
                    let p = dir.join(&png);
                    if p.is_file() {
                        pngs.push((n, p));
                    }
                }
            }
        }
        pngs.sort();
        let pick = pngs
            .iter()
            .find(|(n, _)| *n >= PNG_MIN_PX)
            .or_else(|| pngs.last())?;
        Some(IconFile::Png(pick.1.clone()))
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowIcon {
    /// Drawn with `widgets::image`, untinted.
    Image(SpriteId),
    /// The generic app glyph, drawn with `widgets::icon` in `OnSurfaceVariant`.
    Glyph(SpriteId),
}

pub(crate) struct IconCache {
    glyph: SpriteId,
    /// `None` marks a file that failed once, so it is not retried or re-logged.
    loaded: HashMap<IconFile, Option<SpriteId>>,
}

impl Resource for IconCache {}

impl IconCache {
    pub(crate) fn new(atlas: &mut Atlas) -> Self {
        let glyph = atlas
            .insert_svg(assets::icons::APP, ICON_PX)
            .expect("embedded app glyph is a valid SVG");
        IconCache {
            glyph,
            loaded: HashMap::new(),
        }
    }

    pub(crate) fn get(&mut self, atlas: &mut Atlas, file: Option<&IconFile>) -> RowIcon {
        let Some(file) = file else {
            return RowIcon::Glyph(self.glyph);
        };
        let id = *self.loaded.entry(file.clone()).or_insert_with(|| {
            load(atlas, file)
                .map_err(|e| tracing::warn!(?file, "icon unusable, using generic glyph: {e}"))
                .ok()
        });
        match id {
            Some(id) => RowIcon::Image(id),
            None => RowIcon::Glyph(self.glyph),
        }
    }
}

fn load(atlas: &mut Atlas, file: &IconFile) -> Result<SpriteId, String> {
    match file {
        IconFile::Svg(p) => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            atlas
                .insert_svg_image(bytes, ICON_PX)
                .map_err(|e| e.to_string())
        }
        IconFile::Png(p) => {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            let bitmap = Bitmap::from_png(&bytes)
                .map_err(|e| e.to_string())?
                .fit(PNG_MAX_PX);
            atlas
                .insert(Class::Image, &bitmap)
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#f00"/></svg>"##;

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("app-drawer-icons-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(root: &Path, rel: &str) -> PathBuf {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, SVG).unwrap();
        p
    }

    fn png_bytes(side: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, side, side);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&vec![200u8; (side * side * 4) as usize])
            .unwrap();
        w.finish().unwrap();
        out
    }

    fn dirs(root: &Path, active: Option<&str>) -> IconDirs {
        IconDirs::new(&[root.to_path_buf()], active)
    }

    #[test]
    fn prefers_scalable_svg() {
        let r = tmp("svg");
        let svg = touch(&r, "icons/hicolor/scalable/apps/foo.svg");
        touch(&r, "icons/hicolor/256x256/apps/foo.png");
        assert_eq!(dirs(&r, None).find("foo"), Some(IconFile::Svg(svg)));
    }

    #[test]
    fn picks_smallest_png_at_least_min() {
        let r = tmp("png");
        for n in [48, 256, 512] {
            touch(&r, &format!("icons/hicolor/{n}x{n}/apps/foo.png"));
        }
        let want = r.join("icons/hicolor/256x256/apps/foo.png");
        assert_eq!(dirs(&r, None).find("foo.png"), Some(IconFile::Png(want)));

        let r = tmp("png-small");
        for n in [32, 64] {
            touch(&r, &format!("icons/hicolor/{n}x{n}/apps/foo.png"));
        }
        let want = r.join("icons/hicolor/64x64/apps/foo.png");
        assert_eq!(dirs(&r, None).find("foo"), Some(IconFile::Png(want)));
    }

    #[test]
    fn active_theme_then_inherits_then_hicolor() {
        let r = tmp("inherit");
        fs::create_dir_all(r.join("icons/T")).unwrap();
        fs::write(
            r.join("icons/T/index.theme"),
            "[Icon Theme]\nName=T\nInherits=U\n",
        )
        .unwrap();
        let u = touch(&r, "icons/U/scalable/apps/foo.svg");
        touch(&r, "icons/hicolor/scalable/apps/foo.svg");
        assert_eq!(dirs(&r, Some("T")).find("foo"), Some(IconFile::Svg(u)));
    }

    #[test]
    fn absolute_path_and_pixmaps() {
        let r = tmp("abs");
        let abs = touch(&r, "elsewhere/bar.png");
        let d = dirs(&r, None);
        assert_eq!(
            d.find(abs.to_str().unwrap()),
            Some(IconFile::Png(abs.clone()))
        );
        let pix = touch(&r, "pixmaps/baz.png");
        assert_eq!(d.find("baz"), Some(IconFile::Png(pix)));
        assert_eq!(d.find("nope"), None);
    }

    #[test]
    fn active_theme_reads_gtk_settings() {
        let r = tmp("gtk");
        fs::create_dir_all(r.join("gtk-3.0")).unwrap();
        fs::write(
            r.join("gtk-3.0/settings.ini"),
            "[Settings]\ngtk-theme-name=x\ngtk-icon-theme-name=\"Adwaita\"\n",
        )
        .unwrap();
        assert_eq!(active_theme(&r), Some("Adwaita".to_string()));
        assert_eq!(active_theme(&r.join("missing")), None);
    }

    #[test]
    fn cache_reuses_sprites() {
        let r = tmp("cache");
        let svg = touch(&r, "a.svg");
        let png = r.join("b.png");
        fs::write(&png, png_bytes(300)).unwrap();
        let mut atlas = Atlas::new();
        let mut cache = IconCache::new(&mut atlas);

        let first = |i: RowIcon| match i {
            RowIcon::Image(id) => id,
            RowIcon::Glyph(_) => panic!("expected image"),
        };
        for f in [IconFile::Svg(svg), IconFile::Png(png)] {
            let a = first(cache.get(&mut atlas, Some(&f)));
            let b = first(cache.get(&mut atlas, Some(&f)));
            assert_eq!(a, b);
        }

        let missing = IconFile::Png(r.join("gone.png"));
        assert!(matches!(
            cache.get(&mut atlas, Some(&missing)),
            RowIcon::Glyph(_)
        ));
        assert!(matches!(cache.get(&mut atlas, None), RowIcon::Glyph(_)));
    }
}
