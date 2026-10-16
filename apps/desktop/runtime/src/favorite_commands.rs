//! Local command snippets shared by desktop windows, scoped to root projects.
use crate::config::ConfigStore;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteCommand {
    pub id: String,
    pub title: String,
    pub content: String,
    pub project_id: Option<String>,
}

#[derive(Clone)]
pub struct FavoriteCommandStore {
    store: Arc<ConfigStore>,
}

impl FavoriteCommandStore {
    pub fn open(support_dir: PathBuf) -> Result<Self, String> {
        let path = support_dir.join("favorite-commands.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let value: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if let Some(commands) = value.get("commands") {
                    serde_json::from_value::<Vec<FavoriteCommand>>(commands.clone())
                        .map_err(|error| error.to_string())?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        Ok(Self {
            store: ConfigStore::for_file(path),
        })
    }

    pub fn revision(&self) -> u64 {
        self.store.revision()
    }

    pub fn list(&self, project_id: Option<&str>) -> Vec<FavoriteCommand> {
        let mut commands = self
            .store
            .get_as::<Vec<FavoriteCommand>>("commands")
            .unwrap_or_default();
        commands.retain(|command| {
            command.project_id.is_none() || command.project_id.as_deref() == project_id
        });
        // Stable sort keeps newest-first insertion order within each scope.
        commands.sort_by_key(|command| command.project_id.is_none());
        commands
    }

    pub fn upsert(&self, mut command: FavoriteCommand) -> Result<FavoriteCommand, String> {
        command.title = command.title.trim().to_string();
        if command.title.is_empty() || command.content.trim().is_empty() {
            return Err("favorite.validation.empty".into());
        }
        if command
            .project_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err("favorite.validation.project".into());
        }
        self.store.update_sync(|snapshot| {
            let mut commands: Vec<FavoriteCommand> = snapshot
                .get("commands")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| error.to_string())?
                .unwrap_or_default();
            if command.id.is_empty() {
                command.id = uuid::Uuid::new_v4().to_string();
                commands.insert(0, command.clone());
            } else if let Some(existing) = commands.iter_mut().find(|item| item.id == command.id) {
                *existing = command.clone();
            } else {
                return Err("favorite.message.not_found".into());
            }
            snapshot.insert(
                "commands".into(),
                serde_json::to_value(commands).map_err(|error| error.to_string())?,
            );
            Ok(())
        })?;
        Ok(command)
    }

    pub fn remove(&self, id: &str) -> Result<bool, String> {
        self.store.update_sync(|snapshot| {
            let mut commands: Vec<FavoriteCommand> = snapshot
                .get("commands")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| error.to_string())?
                .unwrap_or_default();
            let count = commands.len();
            commands.retain(|item| item.id != id);
            let removed = commands.len() != count;
            snapshot.insert(
                "commands".into(),
                serde_json::to_value(commands).map_err(|error| error.to_string())?,
            );
            Ok(removed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("codux-favorites-{}", uuid::Uuid::new_v4())))
        }
        fn store(&self) -> FavoriteCommandStore {
            FavoriteCommandStore::open(self.0.clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn command(title: &str, project: Option<&str>) -> FavoriteCommand {
        FavoriteCommand {
            id: String::new(),
            title: title.into(),
            content: format!("  {title}\n第二行  "),
            project_id: project.map(str::to_string),
        }
    }

    #[test]
    fn favorite_commands_scope_order_and_shared_project_worktrees() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let global = store.upsert(command("global", None)).unwrap();
        let project = store.upsert(command("project", Some("root-a"))).unwrap();
        let latest = store.upsert(command("latest", Some("root-a"))).unwrap();
        store.upsert(command("other", Some("root-b"))).unwrap();
        assert_eq!(
            store.list(Some("root-a")),
            vec![latest, project, global.clone()]
        );
        assert_eq!(store.list(None), vec![global]);
        assert_eq!(
            fixture.store().list(Some("root-a")),
            store.list(Some("root-a"))
        );
    }

    #[test]
    fn favorite_commands_crud_preserves_content_and_persists() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut saved = store.upsert(command("检查", Some("root-a"))).unwrap();
        assert!(!saved.id.is_empty());
        assert_eq!(saved.content, "  检查\n第二行  ");
        saved.title = "renamed".into();
        saved.project_id = None;
        assert_eq!(store.upsert(saved.clone()).unwrap(), saved);
        assert_eq!(fixture.store().list(None), vec![saved.clone()]);
        let disk: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.0.join("favorite-commands.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(disk["commands"][0]["content"], saved.content);
        // Open an independent store, bypassing the process-local registry, as
        // a restarted application would do.
        let restarted = Fixture::new();
        std::fs::create_dir_all(&restarted.0).unwrap();
        std::fs::copy(
            fixture.0.join("favorite-commands.json"),
            restarted.0.join("favorite-commands.json"),
        )
        .unwrap();
        assert_eq!(restarted.store().list(None), vec![saved.clone()]);
        assert!(store.remove(&saved.id).unwrap());
        assert!(!store.remove(&saved.id).unwrap());
        assert!(fixture.store().list(None).is_empty());
    }

    #[test]
    fn favorite_commands_multiple_windows_observe_each_others_changes() {
        let fixture = Fixture::new();
        let first = fixture.store();
        let second = fixture.store();
        let revision = second.revision();
        let one = first.upsert(command("one", None)).unwrap();
        assert!(second.revision() > revision);
        let two = second.upsert(command("two", None)).unwrap();
        assert_eq!(first.list(None), vec![two, one.clone()]);
        assert!(second.remove(&one.id).unwrap());
        assert_eq!(first.list(None).len(), 1);
    }

    #[test]
    fn favorite_commands_validation_and_failed_write_leave_snapshot_unchanged() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let saved = store.upsert(command("one", None)).unwrap();
        let revision = store.revision();
        let mut invalid = command("   ", None);
        assert!(store.upsert(invalid.clone()).is_err());
        invalid.title = "valid".into();
        invalid.content = " \n ".into();
        assert!(store.upsert(invalid).is_err());
        std::fs::remove_file(fixture.0.join("favorite-commands.json")).unwrap();
        std::fs::create_dir(fixture.0.join("favorite-commands.json")).unwrap();
        assert!(store.upsert(command("cannot save", None)).is_err());
        assert_eq!(store.revision(), revision);
        assert_eq!(store.list(None), vec![saved]);
    }

    #[test]
    fn favorite_commands_do_not_overwrite_corrupt_data() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(&fixture.0).unwrap();
        let path = fixture.0.join("favorite-commands.json");
        std::fs::write(&path, "broken json").unwrap();
        assert!(FavoriteCommandStore::open(fixture.0.clone()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken json");
    }
}
