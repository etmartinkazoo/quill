//! The note editor: a text view that shows the first line as the title, `#` lines as headings,
//! and checklists and bullets with their marks. A click on a checkbox ticks it, Enter continues a
//! list (and ends it on an empty item), and `- `, `* ` or `[] ` typed at the start of a line
//! become a bullet or a checkbox.

use crate::theme::{Palette, Rgb};
use gtk4::{self as gtk, gdk, glib, prelude::*};
use quill_core::note::{self, BULLET, CHECKED, Kind, UNCHECKED};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What the format menu turns the lines under the cursor into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Heading,
    Subheading,
    Body,
    Bullets,
    Checklist,
}

pub struct Editor {
    pub root: gtk::ScrolledWindow,
    pub view: gtk::TextView,
    buffer: gtk::TextBuffer,
    loading: Cell<bool>,
    changed: RefCell<Option<Box<dyn Fn()>>>,
}

fn rgba((r, g, b): Rgb) -> gdk::RGBA {
    gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0)
}

/// Chars in `text[..bytes]`: GTK counts offsets in chars.
fn chars(text: &str, bytes: usize) -> i32 {
    text[..bytes].chars().count() as i32
}

impl Editor {
    pub fn new() -> Rc<Self> {
        let buffer = gtk::TextBuffer::new(None);
        for (name, scale, bold) in [("title", 1.7, true), ("heading", 1.35, true), ("subheading", 1.15, true)] {
            let tag = gtk::TextTag::builder().name(name).scale(scale).build();
            if bold {
                tag.set_weight(700);
            }
            buffer.tag_table().add(&tag);
        }
        buffer.tag_table().lookup("title").unwrap().set_pixels_below_lines(8);
        buffer.tag_table().lookup("heading").unwrap().set_pixels_above_lines(6);
        buffer.tag_table().add(&gtk::TextTag::builder().name("mark").weight(700).scale(1.15).build());
        buffer.tag_table().add(&gtk::TextTag::builder().name("hash").build());
        buffer.tag_table().add(&gtk::TextTag::builder().name("done").strikethrough(true).build());

        let view = gtk::TextView::builder()
            .buffer(&buffer)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(32)
            .right_margin(32)
            .top_margin(20)
            .bottom_margin(48)
            .pixels_below_lines(2)
            .build();
        view.add_css_class("note-editor");
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&view)
            .vexpand(true)
            .hexpand(true)
            .build();
        let this = Rc::new(Self {
            root,
            view,
            buffer,
            loading: Cell::new(false),
            changed: RefCell::default(),
        });

        let weak = Rc::downgrade(&this);
        this.buffer.connect_changed(move |_| {
            let Some(this) = weak.upgrade() else { return };
            this.restyle();
            if !this.loading.get()
                && let Some(changed) = this.changed.borrow().as_ref() {
                    changed();
                }
        });

        // A checkbox ticks on a click, before the view moves the cursor there.
        let click = gtk::GestureClick::builder().button(1).propagation_phase(gtk::PropagationPhase::Capture).build();
        let weak = Rc::downgrade(&this);
        click.connect_pressed(move |gesture, presses, x, y| {
            let Some(this) = weak.upgrade() else { return };
            if presses == 1 && this.view.is_editable() && this.tick_at(x, y) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        this.view.add_controller(click);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
            let plain = !modifiers.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK);
            let handled = this.view.is_editable()
                && plain
                && match key {
                    gdk::Key::Return | gdk::Key::KP_Enter => this.continue_list(),
                    gdk::Key::space => this.make_mark(),
                    _ => false,
                };
            if handled { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        this.view.add_controller(keys);
        this
    }

    pub fn set_palette(&self, palette: &Palette) {
        let table = self.buffer.tag_table();
        table.lookup("mark").unwrap().set_foreground_rgba(Some(&rgba(palette.accent)));
        table.lookup("hash").unwrap().set_foreground_rgba(Some(&rgba(palette.muted)));
        table.lookup("done").unwrap().set_foreground_rgba(Some(&rgba(palette.muted)));
    }

    /// Called on every edit a person makes, not on [`Editor::load`].
    pub fn connect_changed(&self, changed: impl Fn() + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(changed));
    }

    /// Shows a note's stored text, with the cursor at its end and no undo history.
    pub fn load(&self, stored: &str) {
        self.loading.set(true);
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&note::to_display(stored));
        self.buffer.end_irreversible_action();
        self.buffer.place_cursor(&self.buffer.end_iter());
        self.loading.set(false);
    }

    /// The text to store.
    pub fn text(&self) -> String {
        let (start, end) = self.buffer.bounds();
        note::to_stored(&self.buffer.text(&start, &end, false))
    }

    pub fn set_editable(&self, editable: bool) {
        self.view.set_editable(editable);
        self.view.set_cursor_visible(editable);
    }

    pub fn focus(&self) {
        self.view.grab_focus();
    }

    fn line_text(&self, line: i32) -> Option<(gtk::TextIter, String)> {
        let start = self.buffer.iter_at_line(line)?;
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        Some((start, self.buffer.text(&start, &end, false).to_string()))
    }

    /// Styles every line from its text: the buffer is a note, small enough to do it all.
    fn restyle(&self) {
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_all_tags(&start, &end);
        for n in 0..self.buffer.line_count() {
            let Some((begin, text)) = self.line_text(n) else { continue };
            let mut stop = begin;
            stop.forward_chars(text.chars().count() as i32);
            if n == 0 {
                self.buffer.apply_tag_by_name("title", &begin, &stop);
                continue;
            }
            let parsed = note::line(&text);
            let mut mark_start = begin;
            mark_start.forward_chars(chars(&text, parsed.indent));
            let mut mark_end = begin;
            mark_end.forward_chars(chars(&text, parsed.prefix));
            match parsed.kind {
                Kind::Text => {}
                Kind::Heading | Kind::Subheading => {
                    let name = if parsed.kind == Kind::Heading { "heading" } else { "subheading" };
                    self.buffer.apply_tag_by_name(name, &begin, &stop);
                    self.buffer.apply_tag_by_name("hash", &begin, &mark_end);
                }
                Kind::Bullet => self.buffer.apply_tag_by_name("mark", &mark_start, &mark_end),
                Kind::Checklist { done } => {
                    self.buffer.apply_tag_by_name("mark", &mark_start, &mark_end);
                    if done {
                        self.buffer.apply_tag_by_name("done", &mark_end, &stop);
                    }
                }
            }
        }
    }

    /// Ticks or unticks the checkbox at a point in the view; says whether there was one.
    fn tick_at(&self, x: f64, y: f64) -> bool {
        let (bx, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let Some(iter) = self.view.iter_at_location(bx, by) else { return false };
        let Some((_, text)) = self.line_text(iter.line()) else { return false };
        let parsed = note::line(&text);
        let mark = chars(&text, parsed.indent);
        if iter.line() == 0 || !matches!(parsed.kind, Kind::Checklist { .. }) || !(mark..=mark + 1).contains(&iter.line_offset()) {
            return false;
        }
        self.tick(iter.line());
        true
    }

    /// Ticks or unticks a checklist line.
    pub fn tick(&self, line: i32) {
        let Some((_, text)) = self.line_text(line) else { return };
        let parsed = note::line(&text);
        let Kind::Checklist { done } = parsed.kind else { return };
        let Some(mut start) = self.buffer.iter_at_line_offset(line, chars(&text, parsed.indent)) else { return };
        let mut end = start;
        end.forward_char();
        self.buffer.begin_user_action();
        self.buffer.delete(&mut start, &mut end);
        self.buffer.insert(&mut start, &(if done { UNCHECKED } else { CHECKED }).to_string());
        self.buffer.end_user_action();
    }

    /// Enter in a list item: a new item after it, or, in an empty item, the end of the list.
    fn continue_list(&self) -> bool {
        let mut cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        if self.buffer.has_selection() || cursor.line() == 0 {
            return false;
        }
        let Some((start, text)) = self.line_text(cursor.line()) else { return false };
        let parsed = note::line(&text);
        let mark = match parsed.kind {
            Kind::Bullet => BULLET,
            Kind::Checklist { .. } => UNCHECKED,
            _ => return false,
        };
        if cursor.line_offset() < chars(&text, parsed.prefix) {
            return false;
        }
        self.buffer.begin_user_action();
        if text[parsed.prefix..].trim().is_empty() {
            let (mut start, mut end) = (start, start);
            end.forward_chars(text.chars().count() as i32);
            self.buffer.delete(&mut start, &mut end);
        } else {
            self.buffer.insert(&mut cursor, &format!("\n{}{mark} ", &text[..parsed.indent]));
        }
        self.buffer.end_user_action();
        self.view.scroll_mark_onscreen(&self.buffer.get_insert());
        true
    }

    /// Space after `-`, `*` or `[]` at the start of a line: a bullet or a checkbox.
    fn make_mark(&self) -> bool {
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        if self.buffer.has_selection() || cursor.line() == 0 {
            return false;
        }
        let Some(start) = self.buffer.iter_at_line(cursor.line()) else { return false };
        let before = self.buffer.text(&start, &cursor, false).to_string();
        let typed = before.trim_start_matches([' ', '\t']);
        let mark = match typed {
            "-" | "*" => BULLET,
            "[]" | "[ ]" => UNCHECKED,
            _ => return false,
        };
        let (mut from, mut to) = (cursor, cursor);
        from.backward_chars(typed.chars().count() as i32);
        self.buffer.begin_user_action();
        self.buffer.delete(&mut from, &mut to);
        self.buffer.insert(&mut from, &format!("{mark} "));
        self.buffer.end_user_action();
        true
    }

    /// Makes the lines under the cursor or selection headings, body text, bullets or a
    /// checklist; lines that already are all that become body text again.
    pub fn set_style(&self, style: Style) {
        let (a, b) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (cursor, cursor)
        });
        // The title line keeps its style.
        let lines = a.line().max(1)..=b.line();
        if lines.is_empty() {
            return;
        }
        let is = |kind: Kind| match style {
            Style::Heading => kind == Kind::Heading,
            Style::Subheading => kind == Kind::Subheading,
            Style::Bullets => kind == Kind::Bullet,
            Style::Checklist => matches!(kind, Kind::Checklist { .. }),
            Style::Body => kind == Kind::Text,
        };
        let all = lines.clone().all(|n| self.line_text(n).is_some_and(|(_, text)| is(note::line(&text).kind)));
        let style = if all { Style::Body } else { style };
        self.buffer.begin_user_action();
        for n in lines {
            let Some((_, text)) = self.line_text(n) else { continue };
            let parsed = note::line(&text);
            let keep_indent = matches!(style, Style::Bullets | Style::Checklist);
            // Remove the old mark (and, for a heading, the indent), then add the new one.
            let from = if keep_indent { parsed.indent } else { 0 };
            let old_end = match parsed.kind {
                Kind::Text => parsed.indent,
                _ => parsed.prefix,
            };
            let mark = match style {
                Style::Heading => "# ".to_owned(),
                Style::Subheading => "## ".to_owned(),
                Style::Body => String::new(),
                Style::Bullets => format!("{BULLET} "),
                Style::Checklist => match parsed.kind {
                    Kind::Checklist { done: true } => format!("{CHECKED} "),
                    _ => format!("{UNCHECKED} "),
                },
            };
            let (Some(mut start), Some(mut end)) = (
                self.buffer.iter_at_line_offset(n, chars(&text, from)),
                self.buffer.iter_at_line_offset(n, chars(&text, old_end.max(from))),
            ) else {
                continue;
            };
            self.buffer.delete(&mut start, &mut end);
            self.buffer.insert(&mut start, &mark);
        }
        self.buffer.end_user_action();
    }

    /// Types `text` at the cursor as a person would, through the key handling; for the selftest.
    pub fn type_text(&self, text: &str) {
        for c in text.chars() {
            let handled = match c {
                '\n' => self.continue_list(),
                ' ' => self.make_mark(),
                _ => false,
            };
            if !handled {
                self.buffer.insert_at_cursor(&c.to_string());
            }
        }
    }
}
