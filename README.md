# Quill

Notes for [Omarchy](https://omarchy.org), in the manner of Apple Notes, and a companion to [Signet](https://github.com/etmartinkazoo/signet): native GTK 4 and libadwaita, written in Rust, following your live Omarchy theme. Notes are kept on this computer only.

A quill is what you write with before you press the seal.

## What it does

- **Folders** in the sidebar, with **All Notes** above them and **Recently Deleted** below. Notes is always there. Right-click a folder to rename or delete it. Deleting a folder moves its notes to Recently Deleted.
- **Notes** listed newest first and filed under Pinned, Today, Yesterday, Previous 7 Days, Previous 30 Days, then by month and year. Each shows its title, when it was edited and the start of its text.
- **Writing:** the first line is the note's title. Everything is saved as you type. A new note left empty goes away when you leave it.
- **Checklists:** type `[] ` at the start of a line, press **Ctrl+Shift+L**, or use the checklist button. Click a circle to tick it. A ticked item is struck through.
- **Bullets:** type `- ` or `* ` at the start of a line. Enter continues a list, and Enter on an empty item ends it.
- **Headings:** the **Aa** menu, or **Ctrl+Shift+H** (heading), **Ctrl+Shift+J** (subheading) and **Ctrl+Shift+B** (body text).
- **Pin** a note to keep it at the top of its folder. **Move** it to another folder from the folder button.
- **Search** finds the notes that have every word you type, in every folder.
- **Recently Deleted** keeps a deleted note for 30 days. **Recover** puts it back in its folder (or in Notes, if its folder is gone). A toast offers **Undo** right after you delete.

Notes are stored as Markdown: checklists as `- [ ]` and `- [x]`, bullets as `- `, headings as `#` and `##`. The editor shows these as ○, ◉ and •.

## Keyboard

| Key | Action |
| --- | --- |
| Ctrl+N | New note |
| Ctrl+Shift+N | New folder |
| Ctrl+F | Search |
| Ctrl+Shift+L | Checklist |
| Ctrl+Shift+H / J / B | Heading / subheading / body |
| Escape in search | Clear the search |
| Ctrl+Z, Ctrl+Shift+Z | Undo, redo |
| Ctrl+W, Ctrl+Q | Close |

## Build and install

Needs Rust 1.92+, GTK 4.18+, libadwaita 1.9+ and SQLite. All of these are on Omarchy already, except Rust.

```sh
cargo build --release
install -Dm755 target/release/quill ~/.local/bin/quill
install -Dm644 packaging/io.brem.Quill.desktop ~/.local/share/applications/io.brem.Quill.desktop
```

Debug builds use the app-id `io.brem.Quill.Devel`, so they run beside an installed build. Your notes are in `~/.local/share/quill/quill.db`: back up that file to back up your notes. Set `QUILL_LOG=debug` to see what Quill does. Logs never include what your notes say.

## Layout and tests

- `core/`: no GTK. `note` (titles, previews, date headings, and the Markdown ⇄ editor marks) and `store` (SQLite: folders, notes, Recently Deleted).
- `app/`: the window, the editor, and the Omarchy theme (shared with Signet).

`cargo test --workspace` runs the unit tests. The real window can be driven by script too:

```sh
# Writes a checklist, files a pinned note in a new folder, searches, deletes and recovers;
# PNGs and report.txt into DIR. It writes to the store it opens, so point XDG_DATA_HOME elsewhere.
XDG_DATA_HOME=$(mktemp -d) GDK_BACKEND=broadway QUILL_SELFTEST=DIR cargo run
```

License: MIT.
