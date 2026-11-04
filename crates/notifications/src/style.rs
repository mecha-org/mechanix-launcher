use mecha_wayland::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub variant: TextVariant,
    pub color: ColorRole,
}

impl TextStyle {
    const fn new(variant: TextVariant, color: ColorRole) -> Self {
        Self { variant, color }
    }
}

pub mod color {
    use super::ColorRole;

    /// Panel behind the list. Figma `#0D0D0D`.
    pub const SCREEN: ColorRole = ColorRole::SurfaceContainerLowest;
    /// Square behind the app glyph. Figma Carbon DT/200 `#232325`.
    pub const ICON_TILE: ColorRole = ColorRole::SurfaceContainerHigh;
    /// "CLEAR ALL" text button label. Figma on-surface `#F5F5F5`.
    pub const CLEAR_ALL: ColorRole = ColorRole::OnSurface;
    /// Options menu. Figma surface-container-low `#141415` (its elevation-3
    /// shadow has no equivalent yet).
    pub const MENU: ColorRole = ColorRole::SurfaceContainerLow;
    /// "Options" swipe button. Figma "Button - tonal", secondary-container
    /// `#232325`.
    pub const OPTIONS: ColorRole = ColorRole::SecondaryContainer;
    /// "Clear" swipe button. Same tonal button as Options.
    pub const CLEAR: ColorRole = ColorRole::SecondaryContainer;
    /// "Clear all" / group "Clear" chip. Figma `#303030`.
    pub const CHIP: ColorRole = ColorRole::SecondaryContainer;
    /// Unread count badge. Figma error `#FF4242`.
    pub const COUNT_BADGE: ColorRole = ColorRole::Error;
    pub const COUNT_BADGE_TEXT: ColorRole = ColorRole::OnError;
    /// Stack count badge. Figma `#CCCCCC` on `#2E2E2E`.
    pub const STACK_BADGE: ColorRole = ColorRole::InverseSurface;
    pub const STACK_BADGE_TEXT: ColorRole = ColorRole::InverseOnSurface;
    /// Heads-up "Decline" / "Accept".
    pub const DECLINE: ColorRole = ColorRole::Error;
    pub const ACCEPT: ColorRole = ColorRole::Success;
    /// Card shadow. Figma `rgba(0,0,0,0.25)`, offset 2,0, blur 1.5.
    pub const SHADOW: ColorRole = ColorRole::Shadow;
}

pub mod text {
    use super::TextStyle;
    use super::{ColorRole, TextVariant};

    /// "Today. 19 Sep". Figma Geist Light 16/20, tracking 1, `#8D8D91`.
    pub const DATE_HEADER: TextStyle =
        TextStyle::new(TextVariant::TitleSmall, ColorRole::OnSecondaryFixed);
    /// Options menu row, e.g. "Remind in 15min". Figma Geist Light 16/20,
    /// tracking 1, `#F5F5F5`.
    pub const MENU_ITEM: TextStyle = TextStyle::new(TextVariant::TitleSmall, ColorRole::OnSurface);
    /// Group label, e.g. "X/Twitter". Figma SemiBold 15, `#A2A2A2`.
    pub const GROUP_HEADER: TextStyle =
        TextStyle::new(TextVariant::LabelLarge, ColorRole::OnSecondaryContainer);
    /// Sender / summary. Figma Geist Regular 20/26, `#8D8D91`.
    pub const CARD_TITLE: TextStyle =
        TextStyle::new(TextVariant::TitleLarge, ColorRole::OnSecondaryFixed);
    /// "2m". Figma Geist Mono Regular 14/20, tracking −0.5, `#646468`.
    pub const CARD_TIME: TextStyle =
        TextStyle::new(TextVariant::LabelLarge, ColorRole::OnSurfaceVariant);
    /// Body, one line. Figma Geist Regular 16/22, `#646468`.
    pub const CARD_BODY: TextStyle =
        TextStyle::new(TextVariant::BodyLarge, ColorRole::OnSurfaceVariant);
    /// Action bar label, e.g. "Reply". Figma SemiBold 15, white.
    pub const ACTION: TextStyle = TextStyle::new(TextVariant::LabelLarge, ColorRole::OnSurface);
    /// "Options" / "Clear". Figma Geist Mono Regular 14/20, tracking −0.5,
    /// on-secondary-container `#A1A1A5`.
    pub const SWIPE_BUTTON: TextStyle =
        TextStyle::new(TextVariant::LabelLarge, ColorRole::OnSecondaryContainer);
    /// "Clear all" chip. Figma SemiBold 13, `#B0B0B0`.
    pub const CHIP: TextStyle =
        TextStyle::new(TextVariant::LabelMedium, ColorRole::OnSecondaryContainer);
    /// Badge digit. Figma SemiBold 14–16.
    pub const BADGE: TextStyle = TextStyle::new(TextVariant::LabelMedium, ColorRole::OnError);
    /// "No notifications". Figma centered caption.
    pub const EMPTY: TextStyle =
        TextStyle::new(TextVariant::BodyMedium, ColorRole::OnSurfaceVariant);
}

pub mod space {
    use super::Spacing;

    /// Space above the date row, below the tabs. Figma 8.
    pub const HEADER_TOP: Spacing = Spacing::Space100;
    /// Date row left/right padding. Figma 20.
    pub const HEADER_X: Spacing = Spacing::Space250;
    /// Date row to first card. Figma 12.
    pub const HEADER_TO_LIST: Spacing = Spacing::Space150;
    /// List left/right padding. Figma 8.
    pub const LIST_X: Spacing = Spacing::Space100;
    /// Between cards. Figma 12.
    pub const CARD_GAP: Spacing = Spacing::Space150;
    /// Card padding, all sides. Figma 12 (84 = 12 + 60 icon + 12).
    pub const CARD_PAD: Spacing = Spacing::Space150;
    /// Icon to text column, text column to time. Figma 12.
    pub const ICON_TO_TEXT: Spacing = Spacing::Space150;
    /// "CLEAR ALL" padding. Figma 6 vertical, 12 horizontal.
    pub const CLEAR_ALL_Y: Spacing = Spacing::Space75;
    pub const CLEAR_ALL_X: Spacing = Spacing::Space150;
    /// Title row to body. Figma 8.
    pub const TITLE_TO_BODY: Spacing = Spacing::Space100;
    /// Expanded content to action bar. Figma 20.
    pub const CONTENT_TO_ACTIONS: Spacing = Spacing::Space250;
    /// Card / Options / Clear while swiping. Figma 14.
    pub const SWIPE_GAP: Spacing = Spacing::Space175;
    /// Chip horizontal padding. Figma 21–24.
    pub const CHIP_X: Spacing = Spacing::Space250;
    /// Menu list top/bottom padding, and each item's top/bottom inset. Figma 2.
    pub const MENU_Y: Spacing = Spacing::Space25;
    /// Each menu item's left/right inset. Figma 4.
    pub const MENU_ITEM_X: Spacing = Spacing::Space50;
    /// Padding inside a menu row. Figma 12.
    pub const MENU_ROW_PAD: Spacing = Spacing::Space150;
}

pub mod shape {
    use super::Shape;

    /// Card. Figma corner/extra-small 4.
    pub const CARD: Shape = Shape::ExtraSmall;
    /// App icon square. Figma square corners.
    pub const APP_ICON: Shape = Shape::None;
    /// Contact avatar. Figma radius 26 on 52 (circle).
    pub const AVATAR: Shape = Shape::Full;
    /// Chips and badges. Figma pill.
    pub const PILL: Shape = Shape::Full;
}

/// Component sizes, px. Not theme tokens: they belong to this screen.
pub mod size {
    /// Panel width. Figma 540.
    pub const PANEL_WIDTH: f32 = 540.0;
    /// Panel height. Figma "bottom drawer" 540 × 620.
    pub const PANEL_HEIGHT: f32 = 620.0;
    /// Card minimum height; content sets the real height (84 when
    /// collapsed). Figma min-h 48.
    pub const CARD_MIN_HEIGHT: f32 = 48.0;
    /// App icon square. Figma 60.
    pub const ICON: f32 = 60.0;
    /// Options menu width. Figma 208; height comes from its rows.
    pub const MENU_WIDTH: f32 = 208.0;
    /// One menu row. Figma 44, inside a 2px top/bottom inset (48 per item).
    pub const MENU_ROW_HEIGHT: f32 = 44.0;
    /// Options button bottom to menu top. Figma 8.
    pub const MENU_BELOW_ANCHOR: f32 = 8.0;
    /// "CLEAR ALL" button height. Figma 48.
    pub const CLEAR_ALL_HEIGHT: f32 = 48.0;
    /// Count badge, over the icon's top-right corner. Figma 16–34 wide pill.
    pub const COUNT_BADGE: f32 = 16.0;
    /// App badge, bottom-right of the icon. Figma 22.
    pub const APP_BADGE: f32 = 22.0;
    /// "Clear all" chip. Figma 96 × 28.
    pub const CHIP_HEIGHT: f32 = 28.0;
    /// "Options" swipe button. Figma 113 × 96.
    pub const OPTIONS_WIDTH: f32 = 113.0;
    /// "Clear" swipe button at full reveal. Figma 226 × 96. It fills
    /// whatever the swipe uncovers past Options, so this is its upper end.
    pub const CLEAR_WIDTH: f32 = 226.0;
    /// How far an opened card rests to the left: Options and a
    /// same-width Clear, each after a `SWIPE_GAP` (14). Lands inside
    /// `swipe::REVEAL_MIN..REVEAL_MAX` for the 524 card.
    pub const OPEN_OFFSET: f32 = 2.0 * (OPTIONS_WIDTH + 14.0);
    /// Icon action token. Figma 48.
    pub const ACTION_ICON: f32 = 48.0;
    /// Text action token height. Figma 36.
    pub const ACTION_CHIP_HEIGHT: f32 = 36.0;
    /// Visible cards before the list peeks. Figma "3 + 1 peeking".
    pub const VISIBLE_CARDS: usize = 3;
}

/// Swipe thresholds, as a fraction of the card width.
pub mod swipe {
    /// A horizontal move past this many pixels makes the press a swipe.
    pub const SLOP: f32 = 12.0;
    /// Options + Clear start to show and stick. Figma 20 %.
    pub const REVEAL_MIN: f32 = 0.20;
    /// Upper end of the sticky reveal. Figma 60 %.
    pub const REVEAL_MAX: f32 = 0.60;
    /// Past this the card is dismissed. Figma 65 %.
    pub const DISMISS: f32 = 0.65;
}
