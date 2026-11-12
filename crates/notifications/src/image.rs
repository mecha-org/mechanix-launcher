use mecha_wayland::dbus::Value::{self, Bool, I32, Str, U8};
use mecha_wayland::prelude::*;

#[derive(Debug, Clone)]
pub enum Image {
    Data(ImageData),
    Path(String),
}

#[derive(Debug, Default, Clone)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub bits_per_sample: i32,
    pub channels: i32,
    pub data: Vec<u8>,
}

impl ImageData {
    pub fn from_value(value: &Value) -> Option<ImageData> {
        let Value::Struct(fields) = value else {
            return None;
        };
        let [
            I32(width),
            I32(height),
            I32(rowstride),
            Bool(has_alpha),
            I32(bits_per_sample),
            I32(channels),
            Value::Array { items, .. },
        ] = fields.as_slice()
        else {
            return None;
        };
        let data = items
            .iter()
            .map(|b| if let U8(b) = b { Some(*b) } else { None })
            .collect::<Option<Vec<u8>>>()?;
        Some(ImageData {
            width: *width,
            height: *height,
            rowstride: *rowstride,
            has_alpha: *has_alpha,
            bits_per_sample: *bits_per_sample,
            channels: *channels,
            data,
        })
    }
}

// refer to https://specifications.freedesktop.org/notification/latest-single/#id-1.6.4
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImageHint {
    ImageData,
    ImageDataDeprecated, // "image_data"
    ImagePath,
    ImagePathDeprecated, // "image_path"
    AppIcon,
    IconData,
}

impl ImageHint {
    fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "image-data" => Self::ImageData,
            "image_data" => Self::ImageDataDeprecated,
            "image-path" => Self::ImagePath,
            "image_path" => Self::ImagePathDeprecated,
            "icon_data" => Self::IconData,
            _ => return None,
        })
    }

    fn decode(self, value: &Value) -> Option<Image> {
        match (self, value) {
            (Self::ImagePath | Self::ImagePathDeprecated, Str(path)) => {
                Some(Image::Path(path.clone()))
            }
            (Self::ImagePath | Self::ImagePathDeprecated | Self::AppIcon, _) => None,
            (_, v) => ImageData::from_value(v).map(Image::Data),
        }
    }
}

pub fn select(hints: &PropMap, app_icon: &str) -> Option<Image> {
    let mut image = None;
    let mut best_kind = None;
    if app_icon.starts_with('/') || app_icon.starts_with("file://") {
        image = Some(Image::Path(app_icon.to_string()));
        best_kind = Some(ImageHint::AppIcon);
    }
    for (key, value) in hints {
        let Some(kind) = ImageHint::from_key(key) else {
            continue;
        };
        if let Some(best_kind) = best_kind {
            if best_kind < kind {
                continue;
            }
        }
        if let Some(decoded) = kind.decode(value) {
            image = Some(decoded);
            best_kind = Some(kind);
        }
    }
    image
}
