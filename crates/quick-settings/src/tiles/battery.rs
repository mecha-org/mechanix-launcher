use mecha_wayland::mechanix_widgets::prelude::{
    Text, TextContextExt, TextVariant, VerticalTrim, text,
};
use mecha_wayland::prelude::*;

use services::BatteryPercentageChanged;

use super::{build_tile, use_icon_label_gap};
use crate::util::load_icon;

/// One icon per 10% step, index 0 = 10% … index 9 = 100%.
const LEVELS: [&[u8]; 10] = [
    include_bytes!("../../assets/battery/battery-10.svg"),
    include_bytes!("../../assets/battery/battery-20.svg"),
    include_bytes!("../../assets/battery/battery-30.svg"),
    include_bytes!("../../assets/battery/battery-40.svg"),
    include_bytes!("../../assets/battery/battery-50.svg"),
    include_bytes!("../../assets/battery/battery-60.svg"),
    include_bytes!("../../assets/battery/battery-70.svg"),
    include_bytes!("../../assets/battery/battery-80.svg"),
    include_bytes!("../../assets/battery/battery-90.svg"),
    include_bytes!("../../assets/battery/battery-100.svg"),
];

pub struct Battery {
    pub icon: Handle<Icon>,
    pub percentage: Handle<Text>,
    levels: [SpriteId; 10],
}

pub struct BatteryBuilder;

impl Build for BatteryBuilder {
    type Widget = Battery;
}

impl Widget for Battery {
    type Builder = BatteryBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let levels = LEVELS.map(|svg| load_icon(&mut s.resource_mut::<Atlas>(), svg));

        let icon = build_tile(me, s, LEVELS[0]);
        use_icon_label_gap(me, s);
        let percentage = s.spawn(
            me,
            text("N/A")
                .variant(TextVariant::LabelLarge)
                .weight(FontWeight::Regular)
                .vertical_trim(VerticalTrim::CapHeight)
                .style(LayoutStyle::default().shrink(0.0))
                .color(ColorRole::OnSurface),
        );

        s.on::<BatteryPercentageChanged>(me, |ctx, event| {
            ctx.set_percentage(event.percentage);
        });

        Battery {
            icon,
            percentage,
            levels,
        }
    }
}

/// Index into [`LEVELS`] for a charge percentage, rounded up to the next 10%:
/// 0–10% → 10% icon, 11–20% → 20% icon, …, 91–100% → 100% icon.
/// Unknown shows the 10% icon.
fn level_index(percentage: Option<u8>) -> usize {
    let p = percentage.unwrap_or(0).clamp(1, 100) as usize;
    p.div_ceil(10) - 1
}

pub trait BatteryContext {
    fn set_percentage(&mut self, percentage: Option<u8>);
}

impl BatteryContext for Context<'_, Battery> {
    fn set_percentage(&mut self, percentage: Option<u8>) {
        let label = match percentage {
            Some(value) => format!("{value}%"),
            None => "N/A".to_owned(),
        };
        let (icon, text, sprite) = {
            let me = self.me();
            (me.icon, me.percentage, me.levels[level_index(percentage)])
        };
        let icon_color = self.color(ColorRole::OnSurface);
        if let Some(mut ic) = self.at(icon) {
            ic.set_sprite(sprite);
            ic.set_color(icon_color);
        }
        if let Some(mut txt) = self.at(text) {
            txt.set_text(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::level_index;

    #[test]
    fn percentage_rounds_up_to_the_next_ten_percent_icon() {
        let icon = |p| (level_index(p) + 1) * 10;
        assert_eq!(icon(None), 10);
        assert_eq!(icon(Some(0)), 10);
        assert_eq!(icon(Some(10)), 10);
        assert_eq!(icon(Some(11)), 20);
        assert_eq!(icon(Some(20)), 20);
        assert_eq!(icon(Some(21)), 30);
        assert_eq!(icon(Some(90)), 90);
        assert_eq!(icon(Some(91)), 100);
        assert_eq!(icon(Some(100)), 100);
        assert_eq!(icon(Some(255)), 100);
    }
}
