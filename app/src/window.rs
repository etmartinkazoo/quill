//! The main window: folders in the sidebar, the notes of the one chosen (or of a search), and
//! the note open in the editor. A note is saved as you type, and a note left empty goes away.

use crate::editor::{Editor, Style};
use crate::theme;
use chrono::{DateTime, Local};
use gtk4::{self as gtk, gdk, gio, glib, prelude::*};
use libadwaita::{self as adw, prelude::*};
use quill_core::note;
use quill_core::store::{Note, Store, StoreError, View};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// How long typing has to pause before the note is saved.
const SAVE_AFTER: Duration = Duration::from_millis(600);

pub struct Window {
    pub window: adw::ApplicationWindow,
    pub(crate) store: Store,
    pub(crate) editor: Rc<Editor>,
    folder_list: gtk::ListBox,
    /// What each row of the folder list shows, by index.
    folder_views: RefCell<Vec<View>>,
    note_list: gtk::ListBox,
    /// The notes listed, by row index, and the heading each is filed under.
    listed: RefCell<Vec<Note>>,
    sections: Rc<RefCell<Vec<String>>>,
    outer: adw::OverlaySplitView,
    inner: adw::NavigationSplitView,
    list_page: adw::NavigationPage,
    pub(crate) search: gtk::SearchEntry,
    list_stack: gtk::Stack,
    list_empty: adw::StatusPage,
    editor_stack: gtk::Stack,
    date: adw::WindowTitle,
    pin: gtk::ToggleButton,
    move_menu: gtk::MenuButton,
    format: gtk::MenuButton,
    checklist: gtk::Button,
    delete: gtk::Button,
    recover: gtk::Button,
    empty_deleted: gtk::Button,
    toasts: adw::ToastOverlay,
    view: RefCell<View>,
    current: Cell<Option<i64>>,
    /// The editor has changes not yet saved.
    dirty: Cell<bool>,
    pending: RefCell<Option<glib::SourceId>>,
    /// Set while the window itself changes the lists, so their signals don't act.
    quiet: Cell<bool>,
}

impl Window {
    pub fn new(app: &adw::Application, store: Store) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Quill")
            .default_width(1100)
            .default_height(720)
            .width_request(360)
            .height_request(320)
            .build();

        // Folders.
        let folder_list = gtk::ListBox::new();
        folder_list.add_css_class("navigation-sidebar");
        let new_folder = gtk::Button::builder()
            .child(&adw::ButtonContent::builder().icon_name("folder-new-symbolic").label("New Folder").build())
            .action_name("win.new-folder")
            .tooltip_text("New Folder (Ctrl+Shift+N)")
            .margin_start(6)
            .margin_end(6)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        new_folder.add_css_class("flat");
        let folders = adw::ToolbarView::new();
        folders.add_top_bar(
            &adw::HeaderBar::builder()
                .show_end_title_buttons(false)
                .title_widget(&adw::WindowTitle::new("Quill", ""))
                .build(),
        );
        folders.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&folder_list)
                .vexpand(true)
                .build(),
        ));
        folders.add_bottom_bar(&new_folder);

        // The note list.
        let note_list = gtk::ListBox::new();
        note_list.add_css_class("navigation-sidebar");
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search all notes")
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(6)
            .build();
        let list_empty = adw::StatusPage::builder().icon_name("accessories-text-editor-symbolic").build();
        list_empty.add_css_class("compact");
        let list_stack = gtk::Stack::new();
        list_stack.add_named(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&note_list)
                .vexpand(true)
                .build(),
            Some("notes"),
        );
        list_stack.add_named(&list_empty, Some("empty"));
        let toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Show Folders")
            .active(true)
            .build();
        let add = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("New Note (Ctrl+N)")
            .action_name("win.new")
            .build();
        let empty_deleted = gtk::Button::builder().label("Empty").action_name("win.empty-deleted").visible(false).build();
        empty_deleted.add_css_class("destructive-action");
        let list_header = adw::HeaderBar::builder().show_end_title_buttons(false).build();
        list_header.pack_start(&toggle);
        list_header.pack_end(&add);
        list_header.pack_end(&empty_deleted);
        let list_view = adw::ToolbarView::new();
        list_view.add_top_bar(&list_header);
        list_view.add_top_bar(&search);
        list_view.set_content(Some(&list_stack));
        let list_page = adw::NavigationPage::builder().title("All Notes").tag("list").child(&list_view).build();

        // The editor.
        let editor = Editor::new();
        let editor_stack = gtk::Stack::new();
        editor_stack.add_named(&editor.root, Some("note"));
        editor_stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("accessories-text-editor-symbolic")
                .title("No Note Selected")
                .description("Press Ctrl+N for a new note")
                .build(),
            Some("empty"),
        );
        let date = adw::WindowTitle::new("", "");
        let pin = gtk::ToggleButton::builder().icon_name("view-pin-symbolic").tooltip_text("Pin Note").build();
        let move_menu = gtk::MenuButton::builder().icon_name("folder-symbolic").tooltip_text("Move to Folder").build();
        let format_menu = gio::Menu::new();
        for (label, action) in [
            ("Heading", "win.heading"),
            ("Subheading", "win.subheading"),
            ("Body", "win.body"),
            ("Bulleted List", "win.bullets"),
            ("Checklist", "win.checklist"),
        ] {
            format_menu.append(Some(label), Some(action));
        }
        let format = gtk::MenuButton::builder().label("Aa").tooltip_text("Format").menu_model(&format_menu).build();
        let checklist = gtk::Button::builder()
            .icon_name("checkbox-checked-symbolic")
            .tooltip_text("Checklist (Ctrl+Shift+L)")
            .action_name("win.checklist")
            .build();
        let delete = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Delete Note").action_name("win.delete-note").build();
        let recover = gtk::Button::builder()
            .label("Recover")
            .action_name("win.recover-note")
            .action_target(&0i64.to_variant())
            .build();
        recover.add_css_class("suggested-action");
        let editor_header = adw::HeaderBar::builder().title_widget(&date).build();
        editor_header.pack_start(&format);
        editor_header.pack_start(&checklist);
        editor_header.pack_end(&delete);
        editor_header.pack_end(&move_menu);
        editor_header.pack_end(&pin);
        editor_header.pack_end(&recover);
        let editor_view = adw::ToolbarView::new();
        editor_view.add_top_bar(&editor_header);
        editor_view.set_content(Some(&editor_stack));
        let editor_page = adw::NavigationPage::builder().title("Note").tag("note").child(&editor_view).build();

        let inner = adw::NavigationSplitView::builder()
            .sidebar(&list_page)
            .content(&editor_page)
            .min_sidebar_width(260.0)
            .max_sidebar_width(340.0)
            .sidebar_width_fraction(0.32)
            .build();
        let outer = adw::OverlaySplitView::builder()
            .sidebar(&folders)
            .content(&inner)
            .min_sidebar_width(200.0)
            .max_sidebar_width(240.0)
            .build();
        toggle.bind_property("active", &outer, "show-sidebar").bidirectional().sync_create().build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&outer));
        window.set_content(Some(&toasts));
        let breakpoint = |width: f64| {
            adw::Breakpoint::new(adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                width,
                adw::LengthUnit::Sp,
            ))
        };
        // Narrower: the folders slide over the notes; narrower still, one pane at a time.
        let narrow = breakpoint(900.0);
        narrow.add_setter(&outer, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&outer, "show-sidebar", Some(&false.to_value()));
        window.add_breakpoint(narrow);
        let phone = breakpoint(560.0);
        phone.add_setter(&outer, "collapsed", Some(&true.to_value()));
        phone.add_setter(&outer, "show-sidebar", Some(&false.to_value()));
        phone.add_setter(&inner, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(phone);

        let this = Rc::new(Self {
            window,
            store,
            editor,
            folder_list,
            folder_views: RefCell::default(),
            note_list,
            listed: RefCell::default(),
            sections: Rc::default(),
            outer,
            inner,
            list_page,
            search,
            list_stack,
            list_empty,
            editor_stack,
            date,
            pin,
            move_menu,
            format,
            checklist,
            delete,
            recover,
            empty_deleted,
            toasts,
            view: RefCell::new(View::All),
            current: Cell::new(None),
            dirty: Cell::new(false),
            pending: RefCell::default(),
            quiet: Cell::new(false),
        });
        this.connect(&this);
        let weak = Rc::downgrade(&this);
        let palette = theme::follow(move |palette| {
            if let Some(this) = weak.upgrade() {
                this.editor.set_palette(&palette);
            }
        });
        this.editor.set_palette(&palette);

        this.reload_folders();
        this.reload_notes();
        let first = this.listed.borrow().first().map(|note| note.id);
        this.open(first);
        this
    }

    fn connect(&self, this: &Rc<Self>) {
        let action = |name: &str, run: fn(&Rc<Self>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(this);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    run(&this);
                }
            });
            self.window.add_action(&action);
        };
        action("new", |this| this.new_note());
        action("new-folder", |this| this.new_folder());
        action("search", |this| {
            this.inner.set_show_content(false);
            this.search.grab_focus();
        });
        action("delete-note", |this| this.delete_note());
        action("pin-note", |this| {
            if let Some(note) = this.current_note() {
                this.set_pinned(!note.pinned);
            }
        });
        action("empty-deleted", |this| this.empty_deleted());
        action("heading", |this| this.editor.set_style(Style::Heading));
        action("subheading", |this| this.editor.set_style(Style::Subheading));
        action("body", |this| this.editor.set_style(Style::Body));
        action("bullets", |this| this.editor.set_style(Style::Bullets));
        action("checklist", |this| this.editor.set_style(Style::Checklist));

        let with_id = |name: &str, run: fn(&Rc<Self>, i64)| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::INT64));
            let weak = Rc::downgrade(this);
            action.connect_activate(move |_, target| {
                if let (Some(this), Some(id)) = (weak.upgrade(), target.and_then(|t| t.get::<i64>())) {
                    run(&this, id);
                }
            });
            self.window.add_action(&action);
        };
        with_id("recover-note", |this, id| this.recover_note(id));
        with_id("move-note", |this, folder| this.move_note(folder));
        with_id("rename-folder", |this, id| this.rename_folder(id));
        with_id("delete-folder", |this, id| this.delete_folder(id));

        let weak = Rc::downgrade(this);
        self.editor.connect_changed(move || {
            if let Some(this) = weak.upgrade() {
                this.edited();
            }
        });

        let weak = Rc::downgrade(this);
        self.pin.connect_toggled(move |pin| {
            if let Some(this) = weak.upgrade().filter(|this| !this.quiet.get()) {
                this.set_pinned(pin.is_active());
            }
        });

        // Lists change in an idle callback: a row's own signal mustn't remove it.
        let weak = Rc::downgrade(this);
        self.folder_list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade().filter(|this| !this.quiet.get()) else { return };
            let Some(view) = row.and_then(|row| this.folder_views.borrow().get(row.index() as usize).cloned()) else { return };
            let this = Rc::clone(&this);
            glib::idle_add_local_once(move || this.choose_view(view));
        });
        let weak = Rc::downgrade(this);
        self.folder_list.connect_row_activated(move |_, _| {
            if let Some(this) = weak.upgrade().filter(|this| this.outer.is_collapsed()) {
                this.outer.set_show_sidebar(false);
            }
        });
        let weak = Rc::downgrade(this);
        self.note_list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade().filter(|this| !this.quiet.get()) else { return };
            let Some(id) = row.and_then(|row| this.listed.borrow().get(row.index() as usize).map(|note| note.id)) else { return };
            let this = Rc::clone(&this);
            glib::idle_add_local_once(move || this.open(Some(id)));
        });
        let weak = Rc::downgrade(this);
        self.note_list.connect_row_activated(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.inner.set_show_content(true);
            }
        });
        let sections = Rc::clone(&self.sections);
        self.note_list.set_header_func(move |row, before| {
            let sections = sections.borrow();
            let section = sections.get(row.index() as usize);
            let previous = before.and_then(|before| sections.get(before.index() as usize));
            match section.filter(|section| Some(*section) != previous) {
                Some(section) => {
                    let label = gtk::Label::builder()
                        .label(section)
                        .xalign(0.0)
                        .margin_start(12)
                        .margin_top(if row.index() == 0 { 4 } else { 14 })
                        .margin_bottom(4)
                        .build();
                    label.add_css_class("caption-heading");
                    label.add_css_class("dim-label");
                    row.set_header(Some(&label));
                }
                None => row.set_header(None::<&gtk::Widget>),
            }
        });

        let weak = Rc::downgrade(this);
        self.search.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.reload_notes();
                this.keep_current_listed(0);
            }
        });
        let weak = Rc::downgrade(this);
        self.search.connect_stop_search(move |search| {
            search.set_text("");
            if let Some(this) = weak.upgrade() {
                this.editor.focus();
            }
        });

        let weak = Rc::downgrade(this);
        self.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.leave_current();
            }
            glib::Propagation::Proceed
        });
    }

    /// The store's answer, or `None` with the error shown.
    fn ok<T>(&self, result: Result<T, StoreError>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!("{error}");
                self.toast(&error.to_string());
                None
            }
        }
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    fn current_note(&self) -> Option<Note> {
        self.current.get().and_then(|id| self.ok(self.store.note(id)).flatten())
    }

    pub(crate) fn listed_ids(&self) -> Vec<i64> {
        self.listed.borrow().iter().map(|note| note.id).collect()
    }

    pub(crate) fn current(&self) -> Option<i64> {
        self.current.get()
    }

    fn reload_folders(&self) {
        let folders = self.ok(self.store.folders()).unwrap_or_default();
        let (all, deleted) = self.ok(self.store.counts()).unwrap_or_default();
        let first = folders.first().map(|folder| folder.id);
        let view = self.view.borrow().clone();
        self.quiet.set(true);
        self.folder_list.remove_all();
        let mut views = vec![View::All];
        self.folder_list.append(&folder_row("view-list-symbolic", "All Notes", all));
        for folder in &folders {
            let row = folder_row("folder-symbolic", &folder.name, folder.notes);
            let (id, deletable) = (folder.id, Some(folder.id) != first);
            let click = gtk::GestureClick::builder().button(3).build();
            click.connect_pressed(move |gesture, _, x, y| {
                let menu = gio::Menu::new();
                menu.append_item(&item("Rename…", "win.rename-folder", id));
                if deletable {
                    menu.append_item(&item("Delete…", "win.delete-folder", id));
                }
                if let Some(row) = gesture.widget() {
                    popup(&row, &menu, x, y);
                }
            });
            row.add_controller(click);
            self.folder_list.append(&row);
            views.push(View::Folder(folder.id));
        }
        if deleted > 0 || view == View::Deleted {
            self.folder_list.append(&folder_row("user-trash-symbolic", "Recently Deleted", deleted));
            views.push(View::Deleted);
        }
        if let Some(index) = views.iter().position(|v| *v == view) {
            self.folder_list.select_row(self.folder_list.row_at_index(index as i32).as_ref());
        }
        *self.folder_views.borrow_mut() = views;
        self.quiet.set(false);

        let menu = gio::Menu::new();
        for folder in &folders {
            menu.append_item(&item(&folder.name.replace('_', "__"), "win.move-note", folder.id));
        }
        self.move_menu.set_menu_model(Some(&menu));
    }

    fn view_title(&self, view: &View) -> String {
        match view {
            View::All => "All Notes".to_owned(),
            View::Folder(id) => self
                .ok(self.store.folders())
                .and_then(|folders| folders.into_iter().find(|folder| folder.id == *id))
                .map_or_else(|| "Notes".to_owned(), |folder| folder.name),
            View::Deleted => "Recently Deleted".to_owned(),
            View::Search(_) => "Search".to_owned(),
        }
    }

    /// Lists the chosen folder's notes, or the search's, keeping the open note selected.
    fn reload_notes(&self) {
        let text = self.search.text();
        let view = match text.trim() {
            "" => self.view.borrow().clone(),
            text => View::Search(text.to_owned()),
        };
        let notes = self.ok(self.store.notes(&view)).unwrap_or_default();
        let now = Local::now();
        let pinned_first = !matches!(view, View::Deleted | View::Search(_));
        *self.sections.borrow_mut() = notes
            .iter()
            .map(|note| {
                if pinned_first && note.pinned {
                    "Pinned".to_owned()
                } else {
                    note::section(note.modified.with_timezone(&Local), now.date_naive())
                }
            })
            .collect();
        self.quiet.set(true);
        self.note_list.remove_all();
        for note in &notes {
            let row = note_row(note, now);
            let click = gtk::GestureClick::builder().button(3).build();
            let (id, pinned, deleted) = (note.id, note.pinned, note.deleted.is_some());
            let list = self.note_list.downgrade();
            click.connect_pressed(move |gesture, _, x, y| {
                let Some(row) = gesture.widget().and_downcast::<gtk::ListBoxRow>() else { return };
                if let Some(list) = list.upgrade() {
                    list.select_row(Some(&row));
                }
                let menu = gio::Menu::new();
                if deleted {
                    menu.append_item(&item("Recover", "win.recover-note", id));
                    menu.append(Some("Delete Now…"), Some("win.delete-note"));
                } else {
                    menu.append(Some(if pinned { "Unpin Note" } else { "Pin Note" }), Some("win.pin-note"));
                    menu.append(Some("Delete"), Some("win.delete-note"));
                }
                popup(&row, &menu, x, y);
            });
            row.add_controller(click);
            self.note_list.append(&row);
        }
        if let Some(index) = notes.iter().position(|note| Some(note.id) == self.current.get()) {
            self.note_list.select_row(self.note_list.row_at_index(index as i32).as_ref());
        }
        self.quiet.set(false);
        self.note_list.invalidate_headers();

        let (title, description) = match &view {
            View::Search(_) => ("No Results", "No note has all those words"),
            View::Deleted => ("No Deleted Notes", "Deleted notes stay here for 30 days"),
            _ => ("No Notes", "Press Ctrl+N for a new note"),
        };
        self.list_empty.set_title(title);
        self.list_empty.set_description(Some(description));
        self.list_stack.set_visible_child_name(if notes.is_empty() { "empty" } else { "notes" });
        self.list_page.set_title(&self.view_title(&view));
        self.empty_deleted.set_visible(view == View::Deleted && !notes.is_empty());
        *self.listed.borrow_mut() = notes;
    }

    fn reload(&self) {
        self.reload_folders();
        self.reload_notes();
    }

    /// When the open note isn't listed any more, opens the one now at `index`, or the last.
    fn keep_current_listed(self: &Rc<Self>, index: usize) {
        let ids = self.listed_ids();
        if self.current.get().is_some_and(|id| ids.contains(&id)) {
            return;
        }
        let next = ids.get(index).or(ids.last()).copied();
        self.open(next);
    }

    pub(crate) fn choose_view(self: &Rc<Self>, view: View) {
        let removed = self.leave_current();
        *self.view.borrow_mut() = view;
        self.quiet.set(true);
        self.search.set_text("");
        self.quiet.set(false);
        if removed {
            self.reload_folders();
        }
        self.reload_notes();
        self.current.set(None);
        let first = self.listed.borrow().first().map(|note| note.id);
        self.open(first);
    }

    /// Opens a note in the editor, after saving the one open (or forgetting it, when empty).
    pub(crate) fn open(self: &Rc<Self>, id: Option<i64>) {
        if id.is_some() && id == self.current.get() {
            return;
        }
        let removed = self.leave_current();
        self.current.set(id);
        if removed {
            self.reload();
        } else if let Some(index) = self.listed_ids().iter().position(|listed| Some(*listed) == id) {
            self.quiet.set(true);
            self.note_list.select_row(self.note_list.row_at_index(index as i32).as_ref());
            self.quiet.set(false);
        }
        self.show_current();
    }

    fn show_current(&self) {
        let note = self.current_note();
        let editable = note.as_ref().is_some_and(|note| note.deleted.is_none());
        let deleted = note.as_ref().is_some_and(|note| note.deleted.is_some());
        for widget in [self.pin.upcast_ref::<gtk::Widget>(), self.move_menu.upcast_ref(), self.format.upcast_ref(), self.checklist.upcast_ref()] {
            widget.set_visible(editable);
        }
        self.recover.set_visible(deleted);
        self.delete.set_visible(note.is_some());
        self.delete.set_tooltip_text(Some(if deleted { "Delete Now" } else { "Delete Note" }));
        let Some(note) = note else {
            self.current.set(None);
            self.editor_stack.set_visible_child_name("empty");
            self.date.set_title("");
            return;
        };
        self.editor.load(&note.body);
        self.editor.set_editable(editable);
        self.editor_stack.set_visible_child_name("note");
        self.date.set_title(&note::long_date(note.modified.with_timezone(&Local)));
        self.recover.set_action_target_value(Some(&note.id.to_variant()));
        self.quiet.set(true);
        self.pin.set_active(note.pinned);
        self.quiet.set(false);
    }

    fn edited(self: &Rc<Self>) {
        self.dirty.set(true);
        if let Some(source) = self.pending.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(SAVE_AFTER, move || {
            let Some(this) = weak.upgrade() else { return };
            this.pending.borrow_mut().take();
            if this.save_now() {
                this.reload_notes();
                if let Some(note) = this.current_note() {
                    this.date.set_title(&note::long_date(note.modified.with_timezone(&Local)));
                }
            }
        });
        *self.pending.borrow_mut() = Some(source);
    }

    /// Saves the open note's unsaved changes; says whether its text changed.
    pub(crate) fn save_now(&self) -> bool {
        if let Some(source) = self.pending.borrow_mut().take() {
            source.remove();
        }
        if !self.dirty.replace(false) {
            return false;
        }
        let Some(id) = self.current.get() else { return false };
        self.ok(self.store.save_body(id, &self.editor.text())).unwrap_or(false)
    }

    /// Saves the open note, and deletes it if it's empty; says whether it was deleted.
    fn leave_current(&self) -> bool {
        self.save_now();
        match self.current_note() {
            Some(note) if note.deleted.is_none() && note.body.trim().is_empty() => {
                self.ok(self.store.delete_forever(note.id));
                self.current.set(None);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn new_note(self: &Rc<Self>) {
        self.leave_current();
        let Some(first) = self.ok(self.store.first_folder()) else { return };
        let folder = match *self.view.borrow() {
            View::Folder(id) => id,
            _ => first,
        };
        if *self.view.borrow() == View::Deleted {
            *self.view.borrow_mut() = View::All;
        }
        self.quiet.set(true);
        self.search.set_text("");
        self.quiet.set(false);
        let Some(id) = self.ok(self.store.create_note(folder, "")) else { return };
        self.current.set(Some(id));
        self.reload();
        self.show_current();
        self.inner.set_show_content(true);
        self.editor.focus();
    }

    fn set_pinned(&self, pinned: bool) {
        let Some(id) = self.current.get() else { return };
        self.save_now();
        self.ok(self.store.set_pinned(id, pinned));
        self.quiet.set(true);
        self.pin.set_active(pinned);
        self.quiet.set(false);
        self.pin.set_tooltip_text(Some(if pinned { "Unpin Note" } else { "Pin Note" }));
        self.reload_notes();
    }

    pub(crate) fn delete_note(self: &Rc<Self>) {
        let Some(note) = self.current_note() else { return };
        let index = self.listed_ids().iter().position(|id| *id == note.id).unwrap_or(0);
        if note.deleted.is_some() {
            let weak = Rc::downgrade(self);
            confirm(&self.window, "Delete This Note?", "It can't be recovered.", "Delete", move || {
                if let Some(this) = weak.upgrade() {
                    this.ok(this.store.delete_forever(note.id));
                    this.current.set(None);
                    this.reload();
                    this.keep_current_listed(index);
                }
            });
            return;
        }
        if self.leave_current() {
            self.reload();
            self.keep_current_listed(index);
            return;
        }
        if self.ok(self.store.trash(note.id)).is_none() {
            return;
        }
        let toast = adw::Toast::builder()
            .title("Moved to Recently Deleted")
            .button_label("Undo")
            .action_name("win.recover-note")
            .action_target(&note.id.to_variant())
            .build();
        self.toasts.add_toast(toast);
        self.current.set(None);
        self.reload();
        self.keep_current_listed(index);
    }

    pub(crate) fn recover_note(self: &Rc<Self>, id: i64) {
        self.leave_current();
        let index = self.listed_ids().iter().position(|listed| *listed == id).unwrap_or(0);
        let Some(folder) = self.ok(self.store.recover(id)) else { return };
        if *self.view.borrow() == View::Deleted {
            self.toast(&format!("Recovered to {}", self.view_title(&View::Folder(folder))));
            self.current.set(None);
            self.reload();
            self.keep_current_listed(index);
        } else {
            self.current.set(None);
            self.reload();
            self.open(Some(id));
        }
    }

    fn move_note(self: &Rc<Self>, folder: i64) {
        let Some(id) = self.current.get() else { return };
        self.save_now();
        let index = self.listed_ids().iter().position(|listed| *listed == id).unwrap_or(0);
        if self.ok(self.store.move_note(id, folder)).is_none() {
            return;
        }
        self.toast(&format!("Moved to {}", self.view_title(&View::Folder(folder))));
        self.reload();
        self.keep_current_listed(index);
    }

    fn empty_deleted(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        confirm(
            &self.window,
            "Empty Recently Deleted?",
            "Its notes are deleted for good.",
            "Empty",
            move || {
                if let Some(this) = weak.upgrade() {
                    this.ok(this.store.empty_deleted());
                    this.choose_view(View::All);
                }
            },
        );
    }

    fn new_folder(self: &Rc<Self>) {
        let Some(name) = self.ok(self.store.new_folder_name()) else { return };
        let weak = Rc::downgrade(self);
        ask_name(&self.window, "New Folder", &name, "Create", move |name| {
            if let Some(this) = weak.upgrade()
                && let Some(id) = this.ok(this.store.create_folder(&name)) {
                    this.choose_view(View::Folder(id));
                    this.reload_folders();
                }
        });
    }

    fn rename_folder(self: &Rc<Self>, id: i64) {
        let name = self.view_title(&View::Folder(id));
        let weak = Rc::downgrade(self);
        ask_name(&self.window, "Rename Folder", &name, "Rename", move |name| {
            if let Some(this) = weak.upgrade() {
                this.ok(this.store.rename_folder(id, &name));
                this.reload();
            }
        });
    }

    fn delete_folder(self: &Rc<Self>, id: i64) {
        let Some(folder) = self.ok(self.store.folders()).and_then(|f| f.into_iter().find(|f| f.id == id)) else { return };
        let body = match folder.notes {
            0 => String::new(),
            1 => "Its note moves to Recently Deleted.".to_owned(),
            n => format!("Its {n} notes move to Recently Deleted."),
        };
        let weak = Rc::downgrade(self);
        confirm(&self.window, &format!("Delete “{}”?", folder.name), &body, "Delete", move || {
            let Some(this) = weak.upgrade() else { return };
            this.leave_current();
            if this.ok(this.store.delete_folder(id)).is_some() {
                if *this.view.borrow() == View::Folder(id) {
                    this.choose_view(View::All);
                } else {
                    this.reload();
                    this.keep_current_listed(0);
                }
                this.reload_folders();
            }
        });
    }
}

fn item(label: &str, action: &str, id: i64) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(&id.to_variant()));
    item
}

/// A context menu at a point in `parent`.
fn popup(parent: &impl IsA<gtk::Widget>, menu: &gio::Menu, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_parent(parent);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    // Unparented after the chosen item's action has run.
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

fn folder_row(icon: &str, name: &str, notes: usize) -> gtk::ListBoxRow {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.append(&gtk::Image::from_icon_name(icon));
    line.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
    let count = gtk::Label::new(Some(&notes.to_string()));
    count.add_css_class("dim-label");
    count.add_css_class("numeric");
    line.append(&count);
    gtk::ListBoxRow::builder().child(&line).build()
}

fn note_row(note: &Note, now: DateTime<Local>) -> gtk::ListBoxRow {
    let title = gtk::Label::builder()
        .label(note::title(&note.body).unwrap_or_else(|| "New Note".to_owned()))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("heading");
    let date = gtk::Label::new(Some(&note::short_date(note.modified.with_timezone(&Local), now)));
    date.add_css_class("caption-heading");
    let preview = gtk::Label::builder()
        .label(note::preview(&note.body).unwrap_or_else(|| "No additional text".to_owned()))
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    preview.add_css_class("caption");
    preview.add_css_class("dim-label");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.append(&date);
    line.append(&preview);
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(4)
        .margin_end(4)
        .build();
    column.append(&title);
    column.append(&line);
    gtk::ListBoxRow::builder().child(&column).build()
}

/// Asks before something that can't be undone; runs `confirmed` on yes.
fn confirm(window: &adw::ApplicationWindow, heading: &str, body: &str, verb: &str, confirmed: impl Fn() + 'static) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("yes", verb)]);
    dialog.set_response_appearance("yes", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, response| {
        if response == "yes" {
            confirmed();
        }
    });
    dialog.present(Some(window));
}

/// Asks for a folder's name; runs `named` with it, trimmed.
fn ask_name(window: &adw::ApplicationWindow, heading: &str, initial: &str, verb: &str, named: impl Fn(String) + 'static) {
    let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .extra_child(&entry)
        .default_response("yes")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("yes", verb)]);
    dialog.set_response_appearance("yes", adw::ResponseAppearance::Suggested);
    let weak = dialog.downgrade();
    entry.connect_changed(move |entry| {
        if let Some(dialog) = weak.upgrade() {
            dialog.set_response_enabled("yes", !entry.text().trim().is_empty());
        }
    });
    let text = entry.clone();
    dialog.connect_response(None, move |_, response| {
        let name = text.text().trim().to_owned();
        if response == "yes" && !name.is_empty() {
            named(name);
        }
    });
    dialog.present(Some(window));
    entry.grab_focus();
    entry.select_region(0, -1);
}
