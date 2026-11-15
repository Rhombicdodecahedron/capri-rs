//! Her memory in one SQLite file on the user's PC. Nothing leaves the machine from here.
//! SQLite is blocking, so every call runs on tokio's blocking pool.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use capri_application::ports::{MemoryError, MemoryStore};
use capri_domain::{Message, Role};
use rusqlite::{Connection, params};

#[derive(Debug, Clone)]
pub struct SqliteMemory {
    conn: Arc<Mutex<Connection>>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS facts (
    id      INTEGER PRIMARY KEY,
    text    TEXT NOT NULL UNIQUE,
    created INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS turns (
    id      INTEGER PRIMARY KEY,
    role    TEXT NOT NULL,
    content TEXT NOT NULL,
    created INTEGER NOT NULL DEFAULT (unixepoch())
);
PRAGMA journal_mode = WAL;
";

fn err(e: impl std::fmt::Display) -> MemoryError {
    MemoryError(e.to_string())
}

impl SqliteMemory {
    pub fn open(path: &Path) -> Result<Self, MemoryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        Self::with(Connection::open(path).map_err(err)?)
    }

    pub fn in_memory() -> Result<Self, MemoryError> {
        Self::with(Connection::open_in_memory().map_err(err)?)
    }

    fn with(conn: Connection) -> Result<Self, MemoryError> {
        conn.execute_batch(SCHEMA).map_err(err)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T, MemoryError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let c = conn.lock().map_err(|_| MemoryError("memory lock poisoned".into()))?;
            f(&c).map_err(err)
        })
        .await
        .map_err(err)?
    }
}

/// Lowercase words of three letters or more: enough for keyword recall without an embedding model.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_lowercase).collect()
}

/// Facts sharing the most words with the query first; ties go to the newest. When nothing matches,
/// the newest facts are returned, so she still knows the basics.
pub fn rank(facts: Vec<(String, i64)>, query: &str, limit: usize) -> Vec<String> {
    let q = words(query);
    let mut scored: Vec<(usize, i64, String)> = facts
        .into_iter()
        .map(|(text, created)| {
            let fw = words(&text);
            let score = q.iter().filter(|w| fw.contains(w)).count();
            (score, created, text)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    scored.into_iter().take(limit).map(|(_, _, t)| t).collect()
}

#[async_trait]
impl MemoryStore for SqliteMemory {
    async fn remember(&self, fact: &str) -> Result<(), MemoryError> {
        let fact = fact.trim().to_string();
        self.run(move |c| c.execute("INSERT OR IGNORE INTO facts(text) VALUES (?1)", params![fact]).map(|_| ())).await
    }

    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<String>, MemoryError> {
        let facts = self
            .run(|c| {
                let mut st = c.prepare("SELECT text, created FROM facts")?;
                let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(rank(facts, query, limit))
    }

    async fn forget(&self, fact_contains: &str) -> Result<usize, MemoryError> {
        let needle = format!("%{}%", fact_contains.trim().replace('%', "\\%").replace('_', "\\_"));
        self.run(move |c| c.execute("DELETE FROM facts WHERE text LIKE ?1 ESCAPE '\\'", params![needle])).await
    }

    async fn log_turn(&self, turn: &Message) -> Result<(), MemoryError> {
        let role = match turn.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System | Role::Tool => return Ok(()),
        };
        let content = turn.content.clone();
        self.run(move |c| {
            c.execute("INSERT INTO turns(role, content) VALUES (?1, ?2)", params![role, content]).map(|_| ())
        })
        .await
    }

    async fn recent_turns(&self, limit: usize) -> Result<Vec<Message>, MemoryError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut turns = self
            .run(move |c| {
                let mut st = c.prepare("SELECT role, content FROM turns ORDER BY id DESC LIMIT ?1")?;
                let rows = st.query_map(params![limit], |r| {
                    let role: String = r.get(0)?;
                    let content: String = r.get(1)?;
                    Ok(if role == "assistant" { Message::assistant(content) } else { Message::user(content) })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        turns.reverse();
        Ok(turns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn remembers_recalls_and_forgets() {
        let m = SqliteMemory::in_memory().unwrap();
        m.remember("drives a 1989 BMW E30").await.unwrap();
        m.remember("has a cat").await.unwrap();
        m.remember("has a cat").await.unwrap();
        let r = m.recall("what car do I drive? the BMW", 1).await.unwrap();
        assert_eq!(r, vec!["drives a 1989 BMW E30".to_string()]);
        assert_eq!(m.recall("x", 10).await.unwrap().len(), 2, "duplicates are ignored");
        assert_eq!(m.forget("cat").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn keeps_chat_log_in_order() {
        let m = SqliteMemory::in_memory().unwrap();
        for i in 0..5 {
            m.log_turn(&Message::user(format!("q{i}"))).await.unwrap();
            m.log_turn(&Message::assistant(format!("a{i}"))).await.unwrap();
        }
        let t = m.recent_turns(4).await.unwrap();
        assert_eq!(t.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["q3", "a3", "q4", "a4"]);
    }

    #[test]
    fn words_split_on_punctuation_and_keep_accents() {
        assert_eq!(words("C'est l'été!"), vec!["est", "été"]);
    }
}
