use crate::*;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
mod terminal_owner;
pub use terminal_owner::TerminalOwner;

#[derive(Clone, Debug)]
pub struct Store {
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
}
type R<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Store {
    pub fn open(support_dir: impl AsRef<Path>) -> R<Self> {
        let root = support_dir.as_ref().join("blackboard");
        std::fs::create_dir_all(&root).map_err(err)?;
        let store = Self {
            path: root.join("blackboard.sqlite3"),
            root,
        };
        let mut conn = store.connect()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;
    CREATE TABLE IF NOT EXISTS projects(project_id TEXT PRIMARY KEY,next_ordinal INTEGER NOT NULL DEFAULT 1,revision INTEGER NOT NULL DEFAULT 0,rules_revision INTEGER NOT NULL DEFAULT 0,layout TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS entries(sequence INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL,component TEXT NOT NULL,id TEXT NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL,metadata TEXT NOT NULL,terminal_id TEXT,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(project_id,component,id));
    CREATE INDEX IF NOT EXISTS entries_by_project ON entries(project_id,component,sequence);
    CREATE TABLE IF NOT EXISTS events(sequence INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL,id TEXT NOT NULL UNIQUE,payload TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS events_by_project ON events(project_id,sequence);
    CREATE TABLE IF NOT EXISTS terminals(project_id TEXT NOT NULL,binding_key TEXT NOT NULL,terminal_id TEXT NOT NULL,ordinal INTEGER NOT NULL,title TEXT NOT NULL,active INTEGER NOT NULL,automatic INTEGER NOT NULL,automatic_generation INTEGER NOT NULL DEFAULT 0,rules_revision INTEGER NOT NULL,carried_rules TEXT NOT NULL,PRIMARY KEY(project_id,binding_key),UNIQUE(project_id,terminal_id),UNIQUE(project_id,ordinal));
    CREATE TABLE IF NOT EXISTS context_sources(project_id TEXT NOT NULL,terminal_id TEXT NOT NULL,path TEXT NOT NULL,PRIMARY KEY(project_id,terminal_id));
  ").map_err(err)?;
        if conn
            .prepare("SELECT automatic_generation FROM terminals")
            .is_err()
        {
            if let Err(error) = conn.execute_batch(
                "ALTER TABLE terminals ADD COLUMN automatic_generation INTEGER NOT NULL DEFAULT 0",
            ) {
                conn.prepare("SELECT automatic_generation FROM terminals")
                    .map_err(|_| err(error))?;
            }
        }
        if conn
            .prepare("SELECT owner_pid,owner_started_at FROM terminals")
            .is_err()
        {
            for (column, kind) in [("owner_pid", "INTEGER"), ("owner_started_at", "INTEGER")] {
                if conn
                    .prepare(&format!("SELECT {column} FROM terminals"))
                    .is_err()
                {
                    if let Err(error) = conn
                        .execute_batch(&format!("ALTER TABLE terminals ADD COLUMN {column} {kind}"))
                    {
                        conn.prepare(&format!("SELECT {column} FROM terminals"))
                            .map_err(|_| err(error))?;
                    }
                }
            }
        }
        Self::migrate_default_layouts(&mut conn)?;
        Ok(store)
    }
    fn migrate_default_layouts(conn: &mut Connection) -> R<()> {
        if conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(err)?
            >= 1
        {
            return Ok(());
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        if tx
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .map_err(err)?
            == 0
        {
            let legacy: SplitNode = serde_json::from_value(json!({"kind":"split","axis":"horizontal","ratio":0.45,"a":{"kind":"split","axis":"vertical","ratio":0.53,"a":{"kind":"leaf","component":"summary"},"b":{"kind":"leaf","component":"recent"}},"b":{"kind":"leaf","component":"notes"}})).map_err(err)?;
            let layouts = {
                let mut query = tx
                    .prepare("SELECT project_id,layout FROM projects")
                    .map_err(err)?;
                query
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(err)?
            };
            for (project, raw) in layouts {
                let Ok(mut layout) = serde_json::from_str::<Layout>(&raw) else {
                    continue;
                };
                if layout.tree.as_ref() == Some(&legacy) {
                    layout.tree = Layout::default().tree;
                    tx.execute(
                        "UPDATE projects SET layout=?2,revision=revision+1 WHERE project_id=?1",
                        params![project, serde_json::to_string(&layout).map_err(err)?],
                    )
                    .map_err(err)?;
                }
            }
            tx.pragma_update(None, "user_version", 1).map_err(err)?;
        }
        tx.commit().map_err(err)
    }
    pub(crate) fn connect(&self) -> R<Connection> {
        let conn = Connection::open(&self.path).map_err(err)?;
        conn.busy_timeout(Duration::from_secs(10)).map_err(err)?;
        Ok(conn)
    }
    fn ensure(tx: &Transaction<'_>, project: &str) -> R<()> {
        if project.trim().is_empty() {
            return Err("blackboard.project.required".into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO projects(project_id,layout) VALUES(?1,?2)",
            params![
                project,
                serde_json::to_string(&Layout::default()).map_err(err)?
            ],
        )
        .map_err(err)?;
        Ok(())
    }
    fn bump(tx: &Transaction<'_>, project: &str) -> R<()> {
        tx.execute(
            "UPDATE projects SET revision=revision+1 WHERE project_id=?1",
            [project],
        )
        .map_err(err)?;
        Ok(())
    }
    fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
        let metadata: String = row.get(5)?;
        let component: String = row.get(1)?;
        Ok(Entry {
            id: row.get(0)?,
            component: Component::parse(&component)
                .map_err(|e| rusqlite::Error::InvalidParameterName(e))?,
            title: row.get(2)?,
            body: row.get(3)?,
            terminal_id: row.get(4)?,
            metadata: serde_json::from_str(&metadata).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            sequence: row.get(8)?,
        })
    }
    fn read_conn(
        conn: &Connection,
        project: &str,
        component: Component,
        id: &str,
    ) -> R<Option<Entry>> {
        conn.query_row("SELECT id,component,title,body,terminal_id,metadata,created_at,updated_at,sequence FROM entries WHERE project_id=?1 AND component=?2 AND id=?3",params![project,component.key(),id],Self::entry).optional().map_err(err)
    }
    fn list_conn(conn: &Connection, project: &str, component: Option<Component>) -> R<Vec<Entry>> {
        let mut query=conn.prepare("SELECT id,component,title,body,terminal_id,metadata,created_at,updated_at,sequence FROM entries WHERE project_id=?1 AND (?2 IS NULL OR component=?2) ORDER BY CASE WHEN component='recent' THEN updated_at ELSE sequence END DESC,sequence DESC").map_err(err)?;
        query
            .query_map(params![project, component.map(Component::key)], Self::entry)
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)
    }
    pub fn list(&self, project: &str, component: Component) -> R<Vec<Entry>> {
        Self::list_conn(&self.connect()?, project, Some(component))
    }
    pub fn read(&self, project: &str, component: Component, id: &str) -> R<Entry> {
        Self::read_conn(&self.connect()?, project, component, id)?
            .ok_or_else(|| "blackboard.entry.not_found".into())
    }
    pub fn terminal(&self, project: &str, id: &str) -> R<TerminalBinding> {
        Self::terminal_conn(&self.connect()?, project, id)?
            .ok_or_else(|| "blackboard.terminal.not_found".into())
    }
    fn binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<TerminalBinding> {
        let rules: String = row.get(7)?;
        Ok(TerminalBinding {
            key: row.get(0)?,
            terminal_id: row.get(1)?,
            ordinal: row.get(2)?,
            title: row.get(3)?,
            active: row.get(4)?,
            automatic: row.get(5)?,
            automatic_generation: row.get(8)?,
            rules_revision: row.get(6)?,
            carried_rules: serde_json::from_str(&rules).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        })
    }
    fn terminal_conn(conn: &Connection, project: &str, id: &str) -> R<Option<TerminalBinding>> {
        conn.query_row("SELECT binding_key,terminal_id,ordinal,title,active,automatic,rules_revision,carried_rules,automatic_generation FROM terminals WHERE project_id=?1 AND terminal_id=?2",params![project,id],Self::binding).optional().map_err(err)
    }
    fn rules(conn: &Connection, project: &str) -> R<Vec<Entry>> {
        Ok(Self::list_conn(conn, project, Some(Component::Rules))?
            .into_iter()
            .rev()
            .filter(|r| {
                r.metadata
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            })
            .collect())
    }
    fn actor(conn: &Connection, project: &str, component: Component, actor: &Actor) -> R<Actor> {
        if actor.origin != Origin::Terminal {
            return Ok(actor.clone());
        }
        if ![
            Component::Summary,
            Component::Notes,
            Component::Images,
            Component::Web,
        ]
        .contains(&component)
        {
            return Err("blackboard.component.read_only".into());
        }
        let id = actor
            .terminal_id
            .as_deref()
            .ok_or("blackboard.terminal.required")?;
        let binding = Self::terminal_conn(conn, project, id)?
            .filter(|t| t.active)
            .ok_or("blackboard.terminal.inactive")?;
        Ok(Actor {
            automatic: binding.automatic,
            ..actor.clone()
        })
    }
    fn publish(
        tx: &Transaction<'_>,
        project: &str,
        entry: &Entry,
        operation: &str,
        actor: &Actor,
    ) -> R<()> {
        let state=tx.query_row("SELECT COALESCE(SUM(CASE WHEN component='git' AND COALESCE(json_extract(metadata,'$.dirty'),1)=1 THEN 1 ELSE 0 END),0)>0,COALESCE(SUM(component='notes'),0),COALESCE(SUM(component='images'),0),COALESCE(SUM(component='knowledge' AND COALESCE(json_extract(metadata,'$.kind'),'')!='library'),0),COALESCE(SUM(component='rules' AND COALESCE(json_extract(metadata,'$.enabled'),1)=1),0) FROM entries WHERE project_id=?1",[project],|r|Ok(json!({"git":r.get::<_,bool>(0)?,"notes":r.get::<_,u64>(1)?,"images":r.get::<_,u64>(2)?,"knowledge":r.get::<_,u64>(3)?,"rules":r.get::<_,u64>(4)?}))).map_err(err)?;
        let event = Event {
            sequence: 0,
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project.into(),
            component: entry.component,
            operation: operation.into(),
            entry: entry.clone(),
            terminal_id: actor.terminal_id.clone(),
            terminal_key: actor
                .terminal_id
                .as_deref()
                .map(|id| Self::terminal_conn(tx, project, id))
                .transpose()?
                .flatten()
                .map(|t| t.key),
            automatic: actor.automatic,
            at: now_ms(),
            state,
        };
        tx.execute(
            "INSERT INTO events(project_id,id,payload) VALUES(?1,?2,?3)",
            params![
                project,
                event.id,
                serde_json::to_string(&event).map_err(err)?
            ],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM events WHERE project_id=?1 AND sequence NOT IN (SELECT sequence FROM events WHERE project_id=?1 ORDER BY sequence DESC LIMIT 256)",[project]).map_err(err)?;
        Self::bump(tx, project)
    }
    pub fn write(
        &self,
        project: &str,
        component: Component,
        id: Option<&str>,
        title: &str,
        body: &str,
        metadata: Value,
        actor: Actor,
    ) -> R<Entry> {
        if title.trim().is_empty() && body.trim().is_empty() {
            return Err("blackboard.content.required".into());
        }
        if body.len() > 8 * 1024 * 1024 || title.len() > 1024 || !metadata.is_object() {
            return Err("blackboard.content.invalid".into());
        }
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        Self::ensure(&tx, project)?;
        let actor = Self::actor(&tx, project, component, &actor)?;
        let id = id
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let old = Self::read_conn(&tx, project, component, &id)?;
        if let Some(old) = &old {
            if old.title == title.trim() && old.body == body && old.metadata == metadata {
                return Ok(old.clone());
            }
        }
        let timestamp = now_ms();
        tx.execute("INSERT INTO entries(project_id,component,id,title,body,metadata,terminal_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8) ON CONFLICT(project_id,component,id) DO UPDATE SET title=excluded.title,body=excluded.body,metadata=excluded.metadata,terminal_id=COALESCE(excluded.terminal_id,entries.terminal_id),updated_at=excluded.updated_at",params![project,component.key(),id,title.trim(),body,metadata.to_string(),actor.terminal_id,timestamp]).map_err(err)?;
        if [Component::Summary, Component::Notes].contains(&component) {
            tx.execute("DELETE FROM entries WHERE project_id=?1 AND component=?2 AND sequence NOT IN (SELECT sequence FROM entries WHERE project_id=?1 AND component=?2 ORDER BY sequence DESC LIMIT 64)",params![project, component.key()]).map_err(err)?;
        }
        if component == Component::Rules {
            tx.execute(
                "UPDATE projects SET rules_revision=rules_revision+1 WHERE project_id=?1",
                [project],
            )
            .map_err(err)?;
        }
        let entry =
            Self::read_conn(&tx, project, component, &id)?.ok_or("blackboard.entry.not_found")?;
        let operation = if component == Component::Summary && old.is_none() {
            "generated"
        } else if old.is_some() {
            "updated"
        } else {
            "created"
        };
        Self::publish(&tx, project, &entry, operation, &actor)?;
        tx.commit().map_err(err)?;
        Ok(entry)
    }
    pub fn remove(&self, project: &str, component: Component, id: &str, actor: Actor) -> R<Entry> {
        let asset_path = if component == Component::Images {
            self.image_path(project, id).ok()
        } else {
            None
        };
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let actor = Self::actor(&tx, project, component, &actor)?;
        let entry =
            Self::read_conn(&tx, project, component, id)?.ok_or("blackboard.entry.not_found")?;
        if component == Component::Knowledge && entry.metadata["kind"] == "library" {
            let documents = Self::list_conn(&tx, project, Some(Component::Knowledge))?
                .into_iter()
                .filter(|e| e.metadata.get("library").and_then(Value::as_str) == Some(id))
                .collect::<Vec<_>>();
            for document in documents {
                tx.execute(
                    "DELETE FROM entries WHERE project_id=?1 AND component='knowledge' AND id=?2",
                    params![project, document.id],
                )
                .map_err(err)?;
                Self::publish(&tx, project, &document, "deleted", &actor)?;
            }
        }
        tx.execute(
            "DELETE FROM entries WHERE project_id=?1 AND component=?2 AND id=?3",
            params![project, component.key(), id],
        )
        .map_err(err)?;
        if component == Component::Rules {
            tx.execute(
                "UPDATE projects SET rules_revision=rules_revision+1 WHERE project_id=?1",
                [project],
            )
            .map_err(err)?;
        }
        if component == Component::Web
            || component == Component::Knowledge && entry.metadata["kind"] == "library"
        {
            let raw: String = tx
                .query_row(
                    "SELECT layout FROM projects WHERE project_id=?1",
                    [project],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let mut layout: Layout = serde_json::from_str(&raw).map_err(err)?;
            if component == Component::Web && layout.active_web.as_deref() == Some(id) {
                layout.active_web = None;
            }
            if component == Component::Knowledge && layout.active_library == id {
                layout.active_library = "default".into();
            }
            tx.execute(
                "UPDATE projects SET layout=?2 WHERE project_id=?1",
                params![project, serde_json::to_string(&layout).map_err(err)?],
            )
            .map_err(err)?;
        }
        Self::publish(&tx, project, &entry, "deleted", &actor)?;
        tx.commit().map_err(err)?;
        if let Some(path) = asset_path {
            std::fs::remove_file(path).map_err(err)?;
        }
        Ok(entry)
    }
    pub fn save_layout(&self, project: &str, layout: &Layout) -> R<()> {
        layout.validate()?;
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        Self::ensure(&tx, project)?;
        tx.execute(
            "UPDATE projects SET layout=?2,revision=revision+1 WHERE project_id=?1",
            params![project, serde_json::to_string(layout).map_err(err)?],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn snapshot(&self, project: &str) -> R<Snapshot> {
        self.snapshot_if_changed(project, None)?
            .ok_or_else(|| "blackboard.snapshot.required".into())
    }
    /// Check the revision in the same read transaction as the snapshot. An
    /// unchanged poll never loads or deserializes entry bodies and events.
    pub fn snapshot_if_changed(
        &self,
        project: &str,
        known_revision: Option<u64>,
    ) -> R<Option<Snapshot>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction().map_err(err)?;
        let state: Option<(u64, u64, String)> = tx
            .query_row(
                "SELECT revision,rules_revision,layout FROM projects WHERE project_id=?1",
                [project],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(err)?;
        let (revision, rules_revision, layout) = state.unwrap_or((
            0,
            0,
            serde_json::to_string(&Layout::default()).map_err(err)?,
        ));
        if known_revision == Some(revision) {
            return Ok(None);
        }
        let entries = Self::list_conn(&tx, project, None)?;
        let terminals = {
            let mut q=tx.prepare("SELECT binding_key,terminal_id,ordinal,title,active,automatic,rules_revision,carried_rules,automatic_generation FROM terminals WHERE project_id=?1 ORDER BY ordinal").map_err(err)?;
            q.query_map([project], Self::binding)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?
        };
        let events = {
            let mut q=tx.prepare("SELECT sequence,payload FROM events WHERE project_id=?1 ORDER BY sequence DESC").map_err(err)?;
            q.query_map([project], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?
            .map(|row| {
                let (seq, payload) = row.map_err(err)?;
                let mut event: Event = serde_json::from_str(&payload).map_err(err)?;
                event.sequence = seq;
                Ok(event)
            })
            .collect::<R<Vec<_>>>()?
        };
        let cursor = events.first().map_or(0, |e| e.sequence);
        Ok(Some(Snapshot {
            project_id: project.into(),
            revision,
            rules_revision,
            cursor,
            layout: serde_json::from_str(&layout).map_err(err)?,
            entries,
            terminals,
            events,
        }))
    }
    pub fn register_terminal(
        &self,
        project: &str,
        key: &str,
        id: &str,
        title: &str,
    ) -> R<TerminalBinding> {
        if key.is_empty() || id.is_empty() {
            return Err("blackboard.terminal.required".into());
        }
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        Self::ensure(&tx, project)?;
        let old: Option<(u64, String)> = tx
            .query_row(
                "SELECT ordinal,terminal_id FROM terminals WHERE project_id=?1 AND binding_key=?2",
                params![project, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(err)?;
        let ordinal = if let Some((n, _)) = &old {
            *n
        } else {
            let n = tx
                .query_row(
                    "SELECT next_ordinal FROM projects WHERE project_id=?1",
                    [project],
                    |r| r.get::<_, u64>(0),
                )
                .map_err(err)?;
            tx.execute(
                "UPDATE projects SET next_ordinal=next_ordinal+1 WHERE project_id=?1",
                [project],
            )
            .map_err(err)?;
            n
        };
        let revision: u64 = tx
            .query_row(
                "SELECT rules_revision FROM projects WHERE project_id=?1",
                [project],
                |r| r.get(0),
            )
            .map_err(err)?;
        let rules = Self::rules(&tx, project)?;
        tx.execute("INSERT INTO terminals(project_id,binding_key,terminal_id,ordinal,title,active,automatic,rules_revision,carried_rules) VALUES(?1,?2,?3,?4,?5,1,0,?6,?7) ON CONFLICT(project_id,binding_key) DO UPDATE SET terminal_id=excluded.terminal_id,title=excluded.title,active=1,automatic=0,rules_revision=excluded.rules_revision,carried_rules=excluded.carried_rules",params![project,key,id,ordinal,title,revision,serde_json::to_string(&rules).map_err(err)?]).map_err(err)?;
        if let Some((_, old_id)) = old
            && old_id != id
        {
            tx.execute(
                "UPDATE entries SET terminal_id=?3 WHERE project_id=?1 AND terminal_id=?2",
                params![project, old_id, id],
            )
            .map_err(err)?;
        }
        Self::bump(&tx, project)?;
        let binding = Self::terminal_conn(&tx, project, id)?.unwrap();
        tx.commit().map_err(err)?;
        Ok(binding)
    }
    pub fn set_terminal_state(
        &self,
        project: &str,
        id: &str,
        active: bool,
        automatic: bool,
    ) -> R<()> {
        let conn = self.connect()?;
        let changed=conn.execute("UPDATE terminals SET active=?3,automatic=?4,automatic_generation=automatic_generation+CASE WHEN ?4 THEN 1 ELSE 0 END WHERE project_id=?1 AND terminal_id=?2 AND (active!=?3 OR automatic!=?4 OR ?4=1)",params![project,id,active,automatic]).map_err(err)?;
        if changed == 0 && Self::terminal_conn(&conn, project, id)?.is_none() {
            return Err("blackboard.terminal.not_found".into());
        }
        if changed > 0 {
            conn.execute(
                "UPDATE projects SET revision=revision+1 WHERE project_id=?1",
                [project],
            )
            .map_err(err)?;
        }
        Ok(())
    }
    pub fn finish_automatic(&self, project: &str, id: &str, generation: u64) -> R<()> {
        let conn = self.connect()?;
        conn.execute("UPDATE terminals SET automatic=0 WHERE project_id=?1 AND terminal_id=?2 AND automatic_generation=?3",params![project,id,generation]).map_err(err)?;
        Ok(())
    }
    pub fn project_automatic(&self, project: &str) -> R<bool> {
        self.connect()?.query_row("SELECT EXISTS(SELECT 1 FROM terminals WHERE project_id=?1 AND active=1 AND automatic=1)",[project],|row|row.get(0)).map_err(err)
    }
    pub fn sync_rules(&self, project: &str, id: &str) -> R<TerminalBinding> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let binding = Self::terminal_conn(&tx, project, id)?
            .filter(|b| b.active)
            .ok_or("blackboard.terminal.inactive")?;
        let rules = Self::rules(&tx, project)?;
        tx.execute("UPDATE terminals SET carried_rules=?3,rules_revision=(SELECT rules_revision FROM projects WHERE project_id=?1) WHERE project_id=?1 AND terminal_id=?2",params![project,binding.terminal_id,serde_json::to_string(&rules).map_err(err)?]).map_err(err)?;
        Self::bump(&tx, project)?;
        let binding = Self::terminal_conn(&tx, project, id)?.unwrap();
        tx.commit().map_err(err)?;
        Ok(binding)
    }
    pub fn rules_prompt(&self, project: &str) -> R<String> {
        Ok(Self::rules(&self.connect()?, project)?
            .into_iter()
            .map(|r| format!("## {}\n{}", r.title, r.body))
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
    pub fn write_context(&self, project: &str, id: &str, memory: Option<&Path>) -> R<PathBuf> {
        let binding = self.terminal(project, id)?;
        if !binding.active {
            return Err("blackboard.terminal.inactive".into());
        }
        let conn = self.connect()?;
        if let Some(memory) = memory.filter(|p| !p.starts_with(self.root.join("contexts"))) {
            conn.execute("INSERT INTO context_sources(project_id,terminal_id,path) VALUES(?1,?2,?3) ON CONFLICT(project_id,terminal_id) DO UPDATE SET path=excluded.path",params![project,id,memory.to_string_lossy()]).map_err(err)?;
        }
        let source: Option<String> = conn
            .query_row(
                "SELECT path FROM context_sources WHERE project_id=?1 AND terminal_id=?2",
                params![project, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let mut text = source
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        text.push_str(concat!(
            "\n\n# Codux Blackboard\n",
            "Shared notes (主动读写, `notes`) are the default area for user requests to read or write the blackboard. ",
            "When the user says '写入黑板', '读取黑板', '从黑板中去读', 'write to the blackboard' or 'read from the blackboard', operate on `notes` unless they explicitly name another component. ",
            "Read with `codux-blackboard list notes` and `codux-blackboard read notes <id>`. ",
            "Write with `codux-blackboard add --title <title> --body <text>`, `codux-blackboard edit <id>` or `codux-blackboard remove <id>`. ",
            "Use these write commands only when the user explicitly requests a blackboard/note change or their explicitly configured project rules require it. ",
            "After writing, read back the affected notes to verify the requested change before reporting success. ",
            "If notes are empty, report that instead of substituting summaries.\n\n",
            "Terminal summaries (终端总结, `summary`) are a separate task milestone history. ",
            "A summary entry does not fulfill a request to write to the blackboard; reading summaries does not fulfill a request to read shared notes. ",
            "Use `codux-blackboard list summary` / `codux-blackboard read summary <id>` when task history is needed or explicitly requested. ",
            "After completing the user's task or an independently verifiable milestone, proactively record the outcome with `codux-blackboard summary add --title <title> --body <text>`. ",
            "Include useful verification and remaining work. Skip ordinary Q&A, tool calls, attempts and progress updates; do not duplicate an existing milestone. ",
            "Do not write task completion or progress into notes by default.\n\n",
            "Read images, webpages, knowledge and project rules with `codux-blackboard --help`. File/Git activity updates automatically.\n",
        ));
        if !binding.carried_rules.is_empty() {
            text.push_str("\n# Project hard rules\n");
            for rule in binding.carried_rules {
                text.push_str(&format!("\n## {}\n{}\n", rule.title, rule.body));
            }
        }
        let dir = self.root.join("contexts");
        std::fs::create_dir_all(&dir).map_err(err)?;
        use sha2::{Digest, Sha256};
        let filename = format!(
            "{:x}.md",
            Sha256::digest(format!("{project}/{id}").as_bytes())
        );
        let path = dir.join(filename);
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, text).map_err(err)?;
        std::fs::rename(&temporary, &path).map_err(err)?;
        Ok(path)
    }
    pub fn observe_files(
        &self,
        project: &str,
        root: &Path,
        paths: &[String],
        terminal_id: Option<String>,
    ) -> R<()> {
        for path in paths {
            let absolute = Path::new(path);
            let relative = absolute
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"));
            let Some(relative) = relative else {
                continue;
            };
            if relative.starts_with(".git/") || relative == ".git" || absolute.is_dir() {
                continue;
            }
            let metadata = std::fs::metadata(absolute).ok();
            let modified = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos().to_string());
            self.write(
                project,
                Component::Recent,
                Some(&relative),
                absolute
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
                &relative,
                json!({"path":relative,"exists":metadata.is_some(),"modified":modified}),
                Actor::system(terminal_id.clone(), self.project_automatic(project)?),
            )?;
        }
        Ok(())
    }
}
