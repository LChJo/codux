use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalOwner {
    pub terminal_id: String,
    pub key: String,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
}

impl Store {
    pub fn claim_terminal_owner(
        &self,
        project: &str,
        id: &str,
        pid: u32,
        started_at: u64,
    ) -> R<()> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let changed=tx.execute("UPDATE terminals SET owner_pid=?3,owner_started_at=?4,active=1 WHERE project_id=?1 AND terminal_id=?2 AND (owner_pid IS NOT ?3 OR owner_started_at IS NOT ?4 OR active=0)",params![project,id,pid,started_at]).map_err(err)?;
        if changed == 0 && Self::terminal_conn(&tx, project, id)?.is_none() {
            return Err("blackboard.terminal.not_found".into());
        }
        if changed > 0 {
            Self::bump(&tx, project)?;
        }
        tx.commit().map_err(err)
    }
    pub fn active_terminal_owners(&self, project: &str) -> R<Vec<TerminalOwner>> {
        let conn = self.connect()?;
        let mut query=conn.prepare("SELECT terminal_id,owner_pid,owner_started_at,binding_key FROM terminals WHERE project_id=?1 AND active=1 ORDER BY ordinal DESC").map_err(err)?;
        query
            .query_map([project], |row| {
                Ok(TerminalOwner {
                    terminal_id: row.get(0)?,
                    key: row.get(3)?,
                    pid: row.get(1)?,
                    started_at: row.get(2)?,
                })
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)
    }
    pub fn expire_terminal_owner(&self, project: &str, owner: &TerminalOwner) -> R<bool> {
        let mut conn = self.connect()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(err)?;
        let changed=tx.execute("UPDATE terminals SET active=0,automatic=0 WHERE project_id=?1 AND terminal_id=?2 AND owner_pid IS ?3 AND owner_started_at IS ?4 AND active=1",params![project,owner.terminal_id,owner.pid,owner.started_at]).map_err(err)?;
        if changed > 0 {
            Self::bump(&tx, project)?;
        }
        tx.commit().map_err(err)?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_expiry_is_atomic_with_reclaim_and_does_not_publish_content_events() {
        let root =
            std::env::temp_dir().join(format!("codux-terminal-owner-{}", uuid::Uuid::new_v4()));
        let store = Store::open(&root).unwrap();
        store.register_terminal("p", "pane", "t", "Agent").unwrap();
        store.claim_terminal_owner("p", "t", 11, 100).unwrap();
        let old = store.active_terminal_owners("p").unwrap().remove(0);
        store.claim_terminal_owner("p", "t", 12, 200).unwrap();
        assert!(!store.expire_terminal_owner("p", &old).unwrap());
        assert!(store.terminal("p", "t").unwrap().active);
        store.set_terminal_state("p", "t", true, true).unwrap();
        let current = store.active_terminal_owners("p").unwrap().remove(0);
        let before = store.snapshot("p").unwrap();
        assert!(store.expire_terminal_owner("p", &current).unwrap());
        let after = store.snapshot("p").unwrap();
        assert!(!after.terminals[0].active);
        assert!(!after.terminals[0].automatic);
        assert!(after.revision > before.revision);
        assert_eq!(after.cursor, before.cursor);
        assert!(!store.expire_terminal_owner("p", &current).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
