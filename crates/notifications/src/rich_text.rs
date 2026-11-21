use mecha_wayland::mechanix_widgets::{self as mw, TextContextExt};
use mecha_wayland::prelude::*;

use crate::markup::Run;
use crate::style::text;

pub struct RichTextBuilder {
    pub runs: Vec<Run>,
}

pub struct RichText {
    /// One text node per run, in order.
    texts: Vec<Handle<mw::Text>>,
    wrap: TextWrap,
}

impl Build for RichTextBuilder {
    type Widget = RichText;
}

impl Widget for RichText {
    type Builder = RichTextBuilder;
    fn build(builder: RichTextBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() =
            LayoutStyle::default().row().min_width(px(0.0));
        let texts = builder
            .runs
            .into_iter()
            .map(|run| s.spawn(me, run_text(run, TextWrap::NoWrap)))
            .collect();
        Self {
            texts,
            wrap: TextWrap::NoWrap,
        }
    }
}

fn run_text(run: Run, wrap: TextWrap) -> mw::TextBuilder {
    let words = mw::text(run.words)
        .variant(text::CARD_BODY.variant)
        .color(text::CARD_BODY.color)
        .max_lines(1)
        .overflow(TextOverflow::Ellipsis)
        .wrap(wrap);
    let Some(style) = run.style else {
        return words;
    };
    // TODO: Link and Slant
    words.weight(style.weight).decoration(style.decoration)
}

pub trait RichTextExt {
    fn set_run(&mut self, run: Vec<Run>);
    fn set_text_wrap(&mut self, wrap: TextWrap);
}

impl RichTextExt for Context<'_, RichText> {
    fn set_run(&mut self, run: Vec<Run>) {
        let me = self.handle();
        let old_texts = std::mem::take(&mut self.me().texts);
        let mut new_texts: Vec<Handle<mw::Text>> = Vec::new();
        for (index, run) in run.into_iter().enumerate() {
            match old_texts.get(index) {
                Some(&text) => {
                    if let Some(mut node) = self.at(text) {
                        restyle(&mut node, run);
                    }
                    new_texts.push(text);
                }
                None => {
                    let wrap = self.me().wrap;
                    new_texts.push(self.spawn(me, run_text(run, wrap)));
                }
            }
        }
        // remove the additional old text that are not required
        for &text in old_texts.iter().skip(new_texts.len()) {
            self.remove(text);
        }
        self.me().texts = new_texts;
    }

    fn set_text_wrap(&mut self, wrap: TextWrap) {
        self.me().wrap = wrap;
        for text in self.me().texts.clone() {
            if let Some(mut node) = self.at(text) {
                TextContextExt::set_wrap(&mut node, wrap);
            }
        }
    }
}

fn restyle(text: &mut Context<'_, mw::Text>, run: Run) {
    let (weight, decoration) = match run.style {
        Some(style) => (style.weight, style.decoration),
        None => (FontWeight::Regular, TextDecoration::None),
    };
    text.set_text(run.words);
    text.set_weight(weight);
    text.set_decoration(decoration);
}
