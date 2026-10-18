//! A scripted run through the real window, for checking a build without a person at the
//! keyboard: `QUILL_SELFTEST=<dir>` writes a checklist note, ticks an item, files a note in a new
//! folder and pins it, searches, deletes a note and recovers it, saves a PNG of the window after
//! each step into `<dir>`, writes `report.txt` there and quits. It writes to the store it opens:
//! point `XDG_DATA_HOME` at an empty directory first.

use crate::window::Window;
use gtk4::{self as gtk, glib, prelude::*};
use quill_core::store::View;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const ENV: &str = "QUILL_SELFTEST";

pub fn start(window: &Rc<Window>, dir: PathBuf) {
    let window = Rc::clone(window);
    glib::spawn_future_local(async move {
        let mut report = Vec::new();
        let outcome = script(&window, &dir, &mut report).await;
        report.push(match outcome {
            Ok(()) => "PASS".to_owned(),
            Err(error) => format!("FAIL: {error}"),
        });
        let _ = std::fs::write(dir.join("report.txt"), report.join("\n") + "\n");
        if let Some(app) = window.window.application() {
            app.quit();
        }
    });
}

async fn pause(ms: u64) {
    glib::timeout_future(std::time::Duration::from_millis(ms)).await;
}

/// Saves the window as it is drawn, background and any dialog included, waiting for a finished
/// frame when one is under way.
async fn shot(window: &Window, dir: &Path, name: &str) {
    pause(300).await;
    let widget = window.window.upcast_ref::<gtk::Widget>();
    for _ in 0..40 {
        let paintable = gtk::WidgetPaintable::new(Some(widget));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(widget.width()), f64::from(widget.height()));
        let renderer = widget.native().and_then(|native| native.renderer());
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), renderer) {
            let texture = renderer.render_texture(&node, None);
            if let Err(error) = texture.save_to_png(dir.join(format!("{name}.png"))) {
                eprintln!("selftest: {name}: {error}");
            }
            return;
        }
        pause(50).await;
    }
    eprintln!("selftest: {name}: nothing drawn");
}

fn check(report: &mut Vec<String>, step: &str, ok: bool, detail: impl std::fmt::Display) -> Result<(), String> {
    report.push(format!("{} {step}: {detail}", if ok { "ok " } else { "BAD" }));
    if ok { Ok(()) } else { Err(format!("{step}: {detail}")) }
}

fn body(window: &Window, id: Option<i64>) -> String {
    id.and_then(|id| window.store.note(id).ok().flatten()).map(|note| note.body).unwrap_or_default()
}

async fn script(window: &Rc<Window>, dir: &Path, report: &mut Vec<String>) -> Result<(), String> {
    pause(500).await;
    shot(window, dir, "1-start").await;

    // A checklist, typed as a person would: "[] " starts it, Enter continues it, and Enter on
    // an empty item ends it.
    window.new_note();
    window.editor.type_text("Groceries\n[] milk\neggs\nbread\n\nRemember the bags.");
    window.editor.tick(2);
    pause(900).await;
    let groceries = window.current();
    let expected = "Groceries\n- [ ] milk\n- [x] eggs\n- [ ] bread\nRemember the bags.";
    check(report, "checklist saved as Markdown", body(window, groceries) == expected, format!("{:?}", body(window, groceries)))?;
    shot(window, dir, "2-checklist").await;

    // A note in a new folder, with a heading and bullets, pinned.
    let travel = window.store.create_folder("Travel").map_err(|e| e.to_string())?;
    window.choose_view(View::Folder(travel));
    window.new_note();
    window.editor.type_text("Trip to Lisbon\n# Flights\n- TAP, Thursday\nhotel near Alfama\n\n\nPack light.");
    window.save_now();
    let lisbon = window.current();
    check(report, "bullets saved", body(window, lisbon).contains("- TAP, Thursday\n- hotel near Alfama\n\nPack light."), format!("{:?}", body(window, lisbon)))?;
    WidgetExt::activate_action(&window.window, "win.pin-note", None).map_err(|e| e.to_string())?;
    shot(window, dir, "3-folder").await;

    window.choose_view(View::All);
    check(report, "pinned note first", window.listed_ids().first().copied() == lisbon, format!("{:?}", window.listed_ids()))?;
    shot(window, dir, "4-all-notes").await;

    window.search.set_text("milk bags");
    pause(600).await;
    check(report, "search finds every word", window.listed_ids() == groceries.into_iter().collect::<Vec<_>>(), format!("{:?}", window.listed_ids()))?;
    shot(window, dir, "5-search").await;
    window.search.set_text("");
    pause(400).await;

    window.open(groceries);
    window.delete_note();
    check(report, "deleted note leaves the list", !window.listed_ids().contains(&groceries.unwrap_or(-1)), format!("{:?}", window.listed_ids()))?;
    window.choose_view(View::Deleted);
    shot(window, dir, "6-recently-deleted").await;
    window.recover_note(groceries.unwrap_or(-1));
    window.choose_view(View::All);
    check(report, "recovered", window.listed_ids().contains(&groceries.unwrap_or(-1)), format!("{:?}", window.listed_ids()))?;

    // A new note left empty goes away.
    window.new_note();
    let empty = window.current();
    window.open(lisbon);
    check(report, "empty note forgotten", window.store.note(empty.unwrap_or(-1)).ok().flatten().is_none(), "")?;
    Ok(())
}
