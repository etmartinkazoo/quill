//! Quill: notes for Omarchy, kept on this computer.

mod editor;
mod selftest;
mod theme;
mod window;

use gtk4::{gdk, glib, prelude::*};
use libadwaita as adw;
use quill_core::store::{self, Store};
use std::cell::RefCell;
use std::rc::Rc;

/// Debug builds use `.Devel`, so they run beside an installed build.
const APP_ID: &str = if cfg!(debug_assertions) { "io.brem.Quill.Devel" } else { "io.brem.Quill" };

const CSS: &str = "
.note-editor { font-size: 1.05em; }
.note-editor text { background: transparent; }
";

thread_local! {
    static WINDOW: RefCell<Option<Rc<window::Window>>> = const { RefCell::new(None) };
}

/// Opens the store in Quill's data directory, and forgets notes deleted 30 days ago.
fn open_store() -> Result<Store, String> {
    let dir = store::data_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let store = Store::open(&dir.join("quill.db")).map_err(|error| error.to_string())?;
    match store.purge() {
        Ok(0) => {}
        Ok(purged) => tracing::info!("forgot {purged} notes deleted over 30 days ago"),
        Err(error) => tracing::warn!("{error}"),
    }
    Ok(store)
}

/// The window, made on first use.
fn window(app: &adw::Application) -> Option<Rc<window::Window>> {
    if let Some(window) = WINDOW.with(|w| w.borrow().clone()) {
        return Some(window);
    }
    match open_store() {
        Ok(store) => {
            let provider = gtk4::CssProvider::new();
            provider.load_from_string(CSS);
            if let Some(display) = gdk::Display::default() {
                gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }
            let window = window::Window::new(app, store);
            if let Some(dir) = std::env::var_os(selftest::ENV) {
                selftest::start(&window, dir.into());
            }
            WINDOW.with(|w| *w.borrow_mut() = Some(Rc::clone(&window)));
            Some(window)
        }
        Err(error) => {
            eprintln!("quill: {error}");
            app.quit();
            None
        }
    }
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("QUILL_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.set_accels_for_action("win.new", &["<Control>n"]);
    app.set_accels_for_action("win.new-folder", &["<Control><Shift>n"]);
    app.set_accels_for_action("win.search", &["<Control>f"]);
    app.set_accels_for_action("win.checklist", &["<Control><Shift>l"]);
    app.set_accels_for_action("win.heading", &["<Control><Shift>h"]);
    app.set_accels_for_action("win.subheading", &["<Control><Shift>j"]);
    app.set_accels_for_action("win.body", &["<Control><Shift>b"]);
    app.set_accels_for_action("window.close", &["<Control>w", "<Control>q"]);
    app.connect_activate(|app| {
        if let Some(window) = window(app) {
            window.window.present();
        }
    });
    app.run()
}
