//! The notes, in `$XDG_DATA_HOME/quill/quill.db`: folders, and each note's Markdown text. A
//! deleted note stays in Recently Deleted for 30 days, as in Apple Notes, and can be recovered
//! until then. The first folder, Notes, is always there.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// How long Recently Deleted keeps a note.
pub const KEEP_DELETED_DAYS: i64 = 30;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS folder (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS note (
    id INTEGER PRIMARY KEY,
    folder INTEGER REFERENCES folder (id) ON DELETE SET NULL,
    body TEXT NOT NULL DEFAULT '',
    pinned INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    deleted INTEGER
);
CREATE INDEX IF NOT EXISTS note_by_date ON note (deleted, modified);
";

const COLUMNS: &str = "id, folder, body, pinned, created, modified, deleted";

/// `$XDG_DATA_HOME/quill`.
pub fn data_dir() -> PathBuf {
    let base = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        _ => std::env::home_dir().unwrap_or_default().join(".local/share"),
    };
    base.join("quill")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    /// Its notes, leaving out deleted ones.
    pub notes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    /// `None` once its folder is deleted; recovering it puts it in Notes.
    pub folder: Option<i64>,
    pub body: String,
    pub pinned: bool,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub deleted: Option<DateTime<Utc>>,
}

/// Which notes to list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    All,
    Folder(i64),
    Deleted,
    /// Notes outside Recently Deleted with every word of the search in them.
    Search(String),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the note store: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the Notes folder can't be deleted")]
    FirstFolder,
}

pub struct Store {
    connection: Mutex<Connection>,
    /// The last time given out, in ms: each change gets a later one, so the newest edit always
    /// sorts first, even within a millisecond.
    clock: Mutex<i64>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::with(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(connection: Connection) -> Result<Self, StoreError> {
        connection.execute_batch(SCHEMA)?;
        connection.execute("INSERT INTO folder (name) SELECT 'Notes' WHERE NOT EXISTS (SELECT 1 FROM folder)", [])?;
        let last: i64 = connection.query_row(
            "SELECT MAX(COALESCE(MAX(modified), 0), COALESCE(MAX(deleted), 0)) FROM note",
            [],
            |row| row.get(0),
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            clock: Mutex::new(last),
        })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.connection.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn now(&self) -> i64 {
        let mut last = self.clock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *last = Utc::now().timestamp_millis().max(*last + 1);
        *last
    }

    /// Notes first, then the rest by name.
    pub fn folders(&self) -> Result<Vec<Folder>, StoreError> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT f.id, f.name, (SELECT COUNT(*) FROM note n WHERE n.folder = f.id AND n.deleted IS NULL)
             FROM folder f ORDER BY f.id = (SELECT MIN(id) FROM folder) DESC, f.name COLLATE NOCASE, f.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Folder {
                id: row.get(0)?,
                name: row.get(1)?,
                notes: row.get::<_, i64>(2)? as usize,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Notes: the folder new notes go to when no other is chosen.
    pub fn first_folder(&self) -> Result<i64, StoreError> {
        Ok(self.db().query_row("SELECT MIN(id) FROM folder", [], |row| row.get(0))?)
    }

    /// "New Folder", or "New Folder 2" and on when that's taken.
    pub fn new_folder_name(&self) -> Result<String, StoreError> {
        let names: Vec<String> = self.folders()?.into_iter().map(|folder| folder.name.to_lowercase()).collect();
        Ok((1..)
            .map(|n| if n == 1 { "New Folder".to_owned() } else { format!("New Folder {n}") })
            .find(|name| !names.contains(&name.to_lowercase()))
            .expect("a free name"))
    }

    pub fn create_folder(&self, name: &str) -> Result<i64, StoreError> {
        let db = self.db();
        db.execute("INSERT INTO folder (name) VALUES (?1)", [name.trim()])?;
        Ok(db.last_insert_rowid())
    }

    pub fn rename_folder(&self, id: i64, name: &str) -> Result<(), StoreError> {
        self.db().execute("UPDATE folder SET name = ?2 WHERE id = ?1", params![id, name.trim()])?;
        Ok(())
    }

    /// Deletes a folder; its notes go to Recently Deleted.
    pub fn delete_folder(&self, id: i64) -> Result<(), StoreError> {
        if id == self.first_folder()? {
            return Err(StoreError::FirstFolder);
        }
        let now = self.now();
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute(
            "UPDATE note SET deleted = ?2, pinned = 0 WHERE folder = ?1 AND deleted IS NULL",
            params![id, now],
        )?;
        transaction.execute("DELETE FROM folder WHERE id = ?1", [id])?;
        transaction.commit()?;
        Ok(())
    }

    /// How many notes there are outside Recently Deleted, and in it.
    pub fn counts(&self) -> Result<(usize, usize), StoreError> {
        Ok(self.db().query_row(
            "SELECT COUNT(*) FILTER (WHERE deleted IS NULL), COUNT(*) FILTER (WHERE deleted IS NOT NULL) FROM note",
            [],
            |row| Ok((row.get::<_, i64>(0)? as usize, row.get::<_, i64>(1)? as usize)),
        )?)
    }

    /// Pinned notes first, then the most recently edited; Recently Deleted by when they were deleted.
    pub fn notes(&self, view: &View) -> Result<Vec<Note>, StoreError> {
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        let filter = match view {
            View::All => "deleted IS NULL ORDER BY pinned DESC, modified DESC, id DESC".to_owned(),
            View::Folder(id) => {
                values.push((*id).into());
                "deleted IS NULL AND folder = ?1 ORDER BY pinned DESC, modified DESC, id DESC".to_owned()
            }
            View::Deleted => "deleted IS NOT NULL ORDER BY deleted DESC, id DESC".to_owned(),
            View::Search(text) => {
                let mut filter = "deleted IS NULL".to_owned();
                for word in text.split_whitespace() {
                    values.push(format!("%{}%", escape_like(word)).into());
                    filter.push_str(&format!(" AND body LIKE ?{} ESCAPE '\\'", values.len()));
                }
                filter + " ORDER BY modified DESC, id DESC"
            }
        };
        let db = self.db();
        let mut statement = db.prepare(&format!("SELECT {COLUMNS} FROM note WHERE {filter}"))?;
        let rows = statement.query_map(params_from_iter(values), note_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn note(&self, id: i64) -> Result<Option<Note>, StoreError> {
        Ok(self
            .db()
            .query_row(&format!("SELECT {COLUMNS} FROM note WHERE id = ?1"), [id], note_row)
            .optional()?)
    }

    pub fn create_note(&self, folder: i64, body: &str) -> Result<i64, StoreError> {
        let now = self.now();
        let db = self.db();
        db.execute(
            "INSERT INTO note (folder, body, created, modified) VALUES (?1, ?2, ?3, ?3)",
            params![folder, body, now],
        )?;
        Ok(db.last_insert_rowid())
    }

    /// Saves a note's text; says whether it was different.
    pub fn save_body(&self, id: i64, body: &str) -> Result<bool, StoreError> {
        let now = self.now();
        let changed = self.db().execute(
            "UPDATE note SET body = ?2, modified = ?3 WHERE id = ?1 AND body != ?2",
            params![id, body, now],
        )?;
        Ok(changed > 0)
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<(), StoreError> {
        self.db().execute("UPDATE note SET pinned = ?2 WHERE id = ?1", params![id, pinned])?;
        Ok(())
    }

    pub fn move_note(&self, id: i64, folder: i64) -> Result<(), StoreError> {
        self.db().execute("UPDATE note SET folder = ?2 WHERE id = ?1", params![id, folder])?;
        Ok(())
    }

    /// Moves a note to Recently Deleted.
    pub fn trash(&self, id: i64) -> Result<(), StoreError> {
        let now = self.now();
        self.db()
            .execute("UPDATE note SET deleted = ?2, pinned = 0 WHERE id = ?1", params![id, now])?;
        Ok(())
    }

    /// Takes a note out of Recently Deleted, into its folder or, when that's gone, Notes; gives
    /// the folder.
    pub fn recover(&self, id: i64) -> Result<i64, StoreError> {
        let first = self.first_folder()?;
        let db = self.db();
        db.execute(
            "UPDATE note SET deleted = NULL, folder = COALESCE(folder, ?2) WHERE id = ?1",
            params![id, first],
        )?;
        Ok(db.query_row("SELECT folder FROM note WHERE id = ?1", [id], |row| row.get(0))?)
    }

    pub fn delete_forever(&self, id: i64) -> Result<(), StoreError> {
        self.db().execute("DELETE FROM note WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn empty_deleted(&self) -> Result<(), StoreError> {
        self.db().execute("DELETE FROM note WHERE deleted IS NOT NULL", [])?;
        Ok(())
    }

    /// Forgets notes deleted more than [`KEEP_DELETED_DAYS`] ago; gives how many.
    pub fn purge(&self) -> Result<usize, StoreError> {
        let before = (Utc::now() - Duration::days(KEEP_DELETED_DAYS)).timestamp_millis();
        Ok(self.db().execute("DELETE FROM note WHERE deleted < ?1", [before])?)
    }
}

/// `%`, `_` and `\` matched as themselves in `LIKE … ESCAPE '\'`.
fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn time(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

fn note_row(row: &rusqlite::Row) -> rusqlite::Result<Note> {
    let deleted: Option<i64> = row.get(6)?;
    Ok(Note {
        id: row.get(0)?,
        folder: row.get(1)?,
        body: row.get(2)?,
        pinned: row.get(3)?,
        created: time(row.get(4)?),
        modified: time(row.get(5)?),
        deleted: deleted.map(time),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(notes: &[Note]) -> Vec<i64> {
        notes.iter().map(|note| note.id).collect()
    }

    #[test]
    fn notes_newest_first_and_pinned_on_top() {
        let store = Store::in_memory().unwrap();
        let notes = store.first_folder().unwrap();
        assert_eq!(store.folders().unwrap()[0].name, "Notes");
        let a = store.create_note(notes, "a").unwrap();
        let b = store.create_note(notes, "b").unwrap();
        assert_eq!(ids(&store.notes(&View::All).unwrap()), [b, a]);
        assert!(store.save_body(a, "a, again").unwrap());
        assert!(!store.save_body(a, "a, again").unwrap(), "unchanged text isn't an edit");
        assert_eq!(ids(&store.notes(&View::All).unwrap()), [a, b]);
        store.set_pinned(b, true).unwrap();
        assert_eq!(ids(&store.notes(&View::All).unwrap()), [b, a]);
    }

    #[test]
    fn folders_and_recently_deleted() {
        let store = Store::in_memory().unwrap();
        let notes = store.first_folder().unwrap();
        assert!(matches!(store.delete_folder(notes), Err(StoreError::FirstFolder)));
        assert_eq!(store.new_folder_name().unwrap(), "New Folder");
        let work = store.create_folder(" New Folder ").unwrap();
        assert_eq!(store.new_folder_name().unwrap(), "New Folder 2");
        store.rename_folder(work, "Work").unwrap();
        let plan = store.create_note(work, "plan").unwrap();
        let memo = store.create_note(notes, "memo").unwrap();
        assert_eq!(ids(&store.notes(&View::Folder(work)).unwrap()), [plan]);
        assert_eq!(store.folders().unwrap().iter().map(|f| (f.name.as_str(), f.notes)).collect::<Vec<_>>(), [("Notes", 1), ("Work", 1)]);

        store.trash(memo).unwrap();
        store.delete_folder(work).unwrap();
        assert_eq!(store.counts().unwrap(), (0, 2));
        assert_eq!(ids(&store.notes(&View::Deleted).unwrap()), [plan, memo]);
        assert_eq!(store.recover(plan).unwrap(), notes, "its folder is gone, so it goes to Notes");
        assert_eq!(store.purge().unwrap(), 0);
        store.empty_deleted().unwrap();
        assert_eq!(store.counts().unwrap(), (1, 0));
        store.move_note(plan, store.create_folder("Home").unwrap()).unwrap();
        assert_eq!(store.folders().unwrap()[0].notes, 0);
    }

    #[test]
    fn search_matches_every_word() {
        let store = Store::in_memory().unwrap();
        let notes = store.first_folder().unwrap();
        let lisbon = store.create_note(notes, "Trip to Lisbon\n- [ ] Book flights").unwrap();
        let porto = store.create_note(notes, "Porto: 50% off flights").unwrap();
        let gone = store.create_note(notes, "Lisbon flights, old").unwrap();
        store.trash(gone).unwrap();
        let search = |text: &str| ids(&store.notes(&View::Search(text.to_owned())).unwrap());
        assert_eq!(search("FLIGHTS"), [porto, lisbon]);
        assert_eq!(search("lisbon book"), [lisbon]);
        assert_eq!(search("50%"), [porto]);
        assert_eq!(search("%"), [porto], "% is itself, not a wildcard");
    }
}
