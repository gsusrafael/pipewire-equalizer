//! The graphical interface of `pipewire-equalizer`.
//!
//! A row of fifteen vertical sliders (one per band), a preamp slider and a
//! combo box with the presets. Every change is sent to the daemon, which
//! applies it to the running PipeWire node without interrupting the audio.

mod client;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Adjustment, Orientation};
use libadwaita as adw;
use libadwaita::prelude::*;

use client::Client;
use pweq_core::preset::{BAND_FREQUENCY_LABELS, GAIN_MAX, GAIN_MIN, NUM_BANDS};
use pweq_core::protocol::Request;

const APP_ID: &str = "io.github.gsusrafael.PipewireEqualizer";

/// The band sliders, kept so they can be updated when the daemon answers.
struct Sliders {
    bands: Vec<gtk4::Scale>,
    preamp: gtk4::Scale,
}

/// Whether the interface is updating the sliders itself.
struct Updating(Rc<RefCell<bool>>);

fn main() -> glib::ExitCode {
    let application = gtk4::Application::builder()
        .application_id(APP_ID)
        .build();

    application.connect_startup(|_| {
        adw::init().expect("cannot initialise libadwaita");
    });

    application.connect_activate(|app| {
        build_window(app);
    });

    application.run()
}

fn build_window(app: &gtk4::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("PipeWire Equalizer")
        .default_width(760)
        .default_height(520)
        .build();

    let client = Rc::new(Client::new());
    let sliders: Rc<RefCell<Option<Sliders>>> = Rc::new(RefCell::new(None));
    let updating = Rc::new(RefCell::new(false));

    // ---------------------------------------------------------------- header
    let header = adw::HeaderBar::new();

    let preset_model = gtk4::StringList::new(&[]);
    let preset_choices = gtk4::DropDown::builder()
        .model(&preset_model)
        .build();
    preset_choices.set_tooltip_text(Some("Preset"));

    let refresh = gtk4::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Reload the presets"));

    header.pack_start(&preset_choices);
    header.pack_end(&refresh);

    // ------------------------------------------------------------------ body
    let banner = adw::Banner::new("");
    banner.set_revealed(false);

    let sliders_box = gtk4::Box::new(Orientation::Horizontal, 0);
    sliders_box.set_homogeneous(true);
    sliders_box.set_margin_start(12);
    sliders_box.set_margin_end(12);
    sliders_box.set_margin_top(12);
    sliders_box.set_margin_bottom(12);

    // The preamp comes first, then one slider per band.
    let preamp_column = gtk4::Box::new(Orientation::Vertical, 4);
    preamp_column.set_halign(gtk4::Align::Center);

    let preamp_adjustment = Adjustment::new(
        0.0,
        f64::from(GAIN_MIN),
        f64::from(GAIN_MAX),
        0.5,
        1.0,
        0.0,
    );
    let preamp = gtk4::Scale::new(Orientation::Vertical, Some(&preamp_adjustment));
    preamp.set_draw_value(false);
    preamp.set_vexpand(true);
    preamp.set_valign(gtk4::Align::Fill);

    let preamp_label = gtk4::Label::new(Some("Preamp"));
    preamp_label.add_css_class("dim-label");
    preamp_label.add_css_class("caption");

    preamp_column.append(&preamp);
    preamp_column.append(&preamp_label);
    sliders_box.append(&preamp_column);

    let mut bands = Vec::with_capacity(NUM_BANDS);

    for index in 0..NUM_BANDS {
        let column = gtk4::Box::new(Orientation::Vertical, 4);
        column.set_halign(gtk4::Align::Center);

        let adjustment = Adjustment::new(
            0.0,
            f64::from(GAIN_MIN),
            f64::from(GAIN_MAX),
            0.5,
            1.0,
            0.0,
        );
        let scale = gtk4::Scale::new(Orientation::Vertical, Some(&adjustment));
        scale.set_draw_value(false);
        scale.set_hexpand(false);
        scale.set_vexpand(true);
        scale.set_valign(gtk4::Align::Fill);

        let label = gtk4::Label::new(Some(&format!("{} Hz", BAND_FREQUENCY_LABELS[index])));
        label.add_css_class("dim-label");
        label.add_css_class("caption");

        column.append(&scale);
        column.append(&label);

        sliders_box.append(&column);
        bands.push(scale);
    }

    // ----------------------------------------------------------------- footer
    let footer = gtk4::Box::new(Orientation::Horizontal, 12);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    footer.set_margin_bottom(12);

    let reset = gtk4::Button::with_label("Reset");
    let status = gtk4::Label::new(Some(""));
    status.set_hexpand(true);
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");

    footer.append(&reset);
    footer.append(&status);

    let content = gtk4::Box::new(Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&sliders_box);
    content.append(&footer);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    window.set_content(Some(&toolbar));

    // ------------------------------------------------------------- behaviour
    let show_error = {
        let banner = banner.clone();
        move |message: &str| {
            banner.set_title(message);
            banner.set_revealed(!message.is_empty());
        }
    };

    // The sliders report the value the user picked; the daemon applies it.
    {
        let client = client.clone();
        let updating = updating.clone();
        let show_error = show_error.clone();
        let sliders = sliders.clone();

        for (index, scale) in bands.iter().enumerate() {
            let client = client.clone();
            let updating = updating.clone();
            let show_error = show_error.clone();
            let sliders = sliders.clone();

            scale.connect_value_changed(move |scale| {
                if *updating.borrow() {
                    return;
                }
                let value = scale.value() as f32;
                let request = Request::SetBand {
                    band: index,
                    gain: value,
                };
                if let Err(e) = client.state(&request) {
                    show_error(&e.to_string());
                } else {
                    show_error("");
                }
                let _ = &sliders;
            });
        }

        let client = client.clone();
        let updating = updating.clone();
        let show_error = show_error.clone();
        preamp.connect_value_changed(move |scale| {
            if *updating.borrow() {
                return;
            }
            let value = scale.value() as f32;
            let request = Request::SetPreamp { db: value };
            if let Err(e) = client.state(&request) {
                show_error(&e.to_string());
            } else {
                show_error("");
            }
        });
    }

    // The preset combo box.
    {
        let client = client.clone();
        let updating = updating.clone();
        let sliders = sliders.clone();
        let show_error = show_error.clone();
        let preset_model = preset_model.clone();

        preset_choices.connect_selected_notify(move |combo| {
            if *updating.borrow() {
                return;
            }
            let Some(model) = combo.model() else { return };
            let Some(list) = model.downcast_ref::<gtk4::StringList>() else {
                return;
            };
            let Some(name) = list.string(combo.selected()) else {
                return;
            };
            if name.is_empty() {
                return;
            }

            let request = Request::LoadPreset { name: name.to_string() };
            match client.state(&request) {
                Ok(state) => {
                    show_error(&state.error);
                    apply_state(&state, &sliders, &updating);
                }
                Err(e) => show_error(&e.to_string()),
            }
        });
    }

    // The reset button.
    {
        let client = client.clone();
        let sliders = sliders.clone();
        let updating = updating.clone();
        let show_error = show_error.clone();

        reset.connect_clicked(move |_| {
            match client.state(&Request::Reset) {
                Ok(state) => {
                    show_error(&state.error);
                    apply_state(&state, &sliders, &updating);
                }
                Err(e) => show_error(&e.to_string()),
            }
        });
    }

    // The refresh button reloads the list of presets.
    {
        let client = client.clone();
        let preset_model = preset_model.clone();
        let show_error = show_error.clone();
        let updating = updating.clone();

        refresh.connect_clicked(move |_| {
            match client.get_state() {
                Ok(state) => {
                    fill_presets(&preset_model, &state.presets);
                    show_error(&state.error);
                }
                Err(e) => show_error(&e.to_string()),
            }
            let _ = &updating;
        });
    }

    *sliders.borrow_mut() = Some(Sliders { bands, preamp });

    // First contact with the daemon.
    {
        let client = client.clone();
        let sliders = sliders.clone();
        let updating = updating.clone();
        let show_error = show_error.clone();
        let preset_model = preset_model.clone();

        glib::idle_add_local_once(move || {
            match client.get_state() {
                Ok(state) => {
                    fill_presets(&preset_model, &state.presets);
                    apply_state(&state, &sliders, &updating);
                    if !state.error.is_empty() {
                        show_error(&state.error);
                    }
                }
                Err(e) => show_error(&e.to_string()),
            }
        });
    }

    window.present();
}

/// Fill the preset combo box with the names the daemon reports.
fn fill_presets(model: &gtk4::StringList, names: &[String]) {
    let count = model.n_items();
    if count > 0 {
        model.splice(0, count, &[]);
    }
    for name in names {
        model.append(name);
    }
}

/// Move the sliders to the values the daemon reports.
fn apply_state(
    state: &pweq_core::protocol::State,
    sliders: &Rc<RefCell<Option<Sliders>>>,
    updating: &Rc<RefCell<bool>>,
) {
    let borrowed = sliders.borrow();
    let Some(sliders) = borrowed.as_ref() else {
        return;
    };

    *updating.borrow_mut() = true;
    sliders.preamp.set_value(f64::from(state.preamp_db));
    for (index, scale) in sliders.bands.iter().enumerate() {
        if let Some(gain) = state.gains_db.get(index) {
            scale.set_value(f64::from(*gain));
        }
    }
    *updating.borrow_mut() = false;
}
