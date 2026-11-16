//! One text field per text-input-v3 content purpose, in a scrolling column,
//! for exercising the on-screen keyboard's layout selection on a device.
//! Run under a Wayland session: `cargo run -p keyboard --example field_tester`.

use mecha_wayland::prelude::*;

const FIELDS: &[(&str, ContentPurpose, bool)] = &[
    ("Normal", ContentPurpose::Normal, false),
    ("Email", ContentPurpose::Email, false),
    ("Url", ContentPurpose::Url, false),
    ("Password", ContentPurpose::Password, true),
    ("Pin", ContentPurpose::Pin, false),
    ("Number", ContentPurpose::Number, false),
    ("Phone", ContentPurpose::Phone, false),
    ("Terminal", ContentPurpose::Terminal, false),
    ("Terminal+hidden", ContentPurpose::Terminal, true),
];

struct Form;
struct FormBuilder {
    font: FontId,
}

impl Build for FormBuilder {
    type Widget = Form;
}

impl Widget for Form {
    type Builder = FormBuilder;
    fn build(b: FormBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let label_color = Color::rgb(0.8, 0.8, 0.82);
        let field_color = Color::rgb(0.92, 0.92, 0.95);
        let field_bg = Color::rgb(0.18, 0.18, 0.2);
        let field_border = Color::rgb(0.4, 0.4, 0.45);

        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().column().fill();

        let list = s.spawn(
            me,
            scrollable().style(
                LayoutStyle::default()
                    .column()
                    .gap(px(6.0))
                    .fill()
                    .min_height(px(0.0))
                    .padding_all(px(16.0)),
            ),
        );
        for &(label, purpose, hidden) in FIELDS {
            s.spawn(list, text(b.font, label).size(14).color(label_color));
            let mut field = input(b.font)
                .size(18)
                .style(
                    LayoutStyle::default()
                        .size(percent(100.0), px(40.0))
                        .shrink(0.0),
                )
                .border(1.0, field_border)
                .background(field_bg)
                .color(field_color)
                .content_purpose(purpose);
            if hidden {
                field = field.content_hint(ContentHint::SENSITIVEDATA | ContentHint::HIDDENTEXT);
            }
            s.spawn(list, field);
        }
        // Room to scroll the last field above the keyboard.
        s.spawn(
            list,
            div().style(LayoutStyle::default().min_height(px(400.0))),
        );
        Form
    }
}

struct Shell;
struct ShellBuilder {
    root: NodeId,
    font: FontId,
}
impl Build for ShellBuilder {
    type Widget = Shell;
}
impl Widget for Shell {
    type Builder = ShellBuilder;
    fn build(b: ShellBuilder, _me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let win = s.spawn(
            b.root,
            window()
                .title("field tester")
                .clear(Color::rgb(0.12, 0.12, 0.14))
                .layout(LayoutStyle::default().column().size(px(540.0), px(620.0))),
        );
        s.on::<CloseRequested>(win, |ctx, _| ctx.signal(Stop));
        s.spawn(win, FormBuilder { font: b.font });
        Shell
    }
}

fn main() {
    let mut app = App::new();
    app.add_module(LayoutModule)
        .add_module(PaintModule)
        .add_module(WindowModule)
        .add_module(InteractivityModule)
        .add_module(AnimationModule)
        .add_module(ScrollModule)
        .add_module(RenderModule::default())
        .insert_resource(Atlas::new());
    app.add_module(RingModule::default())
        .add_module(
            WaylandModule::new()
                .bind::<WlCompositor>()
                .bind::<ZwpLinuxDmabufV1>()
                .bind::<XdgWmBase>()
                .bind::<WlSeat>(),
        )
        .add_module(PresentationModule {
            app_id: "mecha.field_tester".into(),
            budget: Budget::default(),
        })
        .add_module(TextInputModule);

    let font = app
        .resource_mut::<Atlas>()
        .add_font(include_bytes!("../resources/Geist-Regular.ttf"))
        .expect("Geist loads");

    let root = app.root();
    app.spawn(root, ShellBuilder { root, font });
    app.run();
}
