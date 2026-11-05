use mecha_wayland::prelude::*;

use crate::indicators::{indicator_layout, load_svg_icon};

use assets::icons::bluetooth as svg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BluetoothState {
    Connected,
    #[default]
    On,
    Off,
    Disconnected,
}

impl From<&::services::BluetoothState> for BluetoothState {
    fn from(state: &::services::BluetoothState) -> Self {
        match state {
            ::services::BluetoothState::Connected { .. } => Self::Connected,
            ::services::BluetoothState::On => Self::On,
            ::services::BluetoothState::Off => Self::Off,
        }
    }
}

impl BluetoothState {
    #[inline]
    pub fn is_visible(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BluetoothIcons {
    pub default: SpriteId,
    pub connected: SpriteId,
    pub slash: SpriteId,
    pub x: SpriteId,
}

impl BluetoothIcons {
    pub fn load(atlas: &mut Atlas, master: u32) -> Result<Self, Error> {
        Ok(Self {
            default: load_svg_icon(atlas, svg::DEFAULT, master)?,
            connected: load_svg_icon(atlas, svg::CONNECTED, master)?,
            slash: load_svg_icon(atlas, svg::SLASH, master)?,
            x: load_svg_icon(atlas, svg::X, master)?,
        })
    }

    pub fn sprite(&self, state: BluetoothState) -> SpriteId {
        match state {
            BluetoothState::Connected => self.connected,
            BluetoothState::On => self.default,
            BluetoothState::Off => self.slash,
            BluetoothState::Disconnected => self.x,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BluetoothIndicator {
    pub icons: BluetoothIcons,
    pub state: BluetoothState,
    pub icon_node: Handle<Icon>,
}

pub fn bluetooth_indicator(icons: BluetoothIcons) -> BluetoothIndicatorBuilder {
    BluetoothIndicatorBuilder {
        icons,
        state: BluetoothState::default(),
    }
}

pub struct BluetoothIndicatorBuilder {
    pub icons: BluetoothIcons,
    pub state: BluetoothState,
}

impl BluetoothIndicatorBuilder {
    pub fn state(mut self, state: BluetoothState) -> Self {
        self.state = state;
        self
    }
}

impl Build for BluetoothIndicatorBuilder {
    type Widget = BluetoothIndicator;
}

impl Widget for BluetoothIndicator {
    type Builder = BluetoothIndicatorBuilder;

    fn build(b: BluetoothIndicatorBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = indicator_layout(b.state.is_visible());

        let sprite = b.icons.sprite(b.state);
        let color = s.color(ColorRole::OnSurface);
        let icon_node = s.spawn(me, icon(sprite).color(color));

        s.on_theme(me, move |ctx| {
            let color = ctx.color(ColorRole::OnSurface);
            ctx.at(icon_node).unwrap().set_color(color);
        });

        BluetoothIndicator {
            icons: b.icons,
            state: b.state,
            icon_node,
        }
    }
}

impl BluetoothIndicator {
    #[inline]
    pub fn state(&self) -> BluetoothState {
        self.state
    }
}

pub trait BluetoothIndicatorContext {
    fn state(&mut self) -> BluetoothState;
    fn set_state(&mut self, state: BluetoothState);
}

impl BluetoothIndicatorContext for Context<'_, BluetoothIndicator> {
    #[inline]
    fn state(&mut self) -> BluetoothState {
        self.me().state
    }

    fn set_state(&mut self, state: BluetoothState) {
        if self.me().state == state {
            return;
        }
        self.me().state = state;
        let visible = state.is_visible();
        self.set_style(indicator_layout(visible));

        if visible {
            let sprite = self.me().icons.sprite(state);
            let icon_node = self.me().icon_node;
            self.at(icon_node).unwrap().set_sprite(sprite);
        }
    }
}
