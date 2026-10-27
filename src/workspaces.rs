//! Managed workspace catalog. Filenames use IDs, never user-entered names.
use crate::storage;
use anyhow::{bail, Context, Result};
use piratewave_core::project::Project;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Index {
    active: u64,
    entries: Vec<Entry>,
}
pub struct Workspaces {
    root: PathBuf,
    index: Index,
}
impl Workspaces {
    pub fn open(root: PathBuf, legacy: &Path) -> Result<Self> {
        let catalog = root.join("index.json");
        if catalog.exists() {
            let index: Index = serde_json::from_slice(&std::fs::read(&catalog)?)?;
            let mut names = std::collections::HashSet::new();
            let mut ids = std::collections::HashSet::new();
            for entry in &index.entries {
                validate_name(&entry.name)?;
                anyhow::ensure!(
                    names.insert(entry.name.to_lowercase()) && ids.insert(entry.id),
                    "Duplicate workspace in catalog"
                );
            }
            anyhow::ensure!(ids.contains(&index.active), "Active workspace is missing");
            let store = Self { root, index };
            store.load(store.active())?;
            return Ok(store);
        }
        let mut project = if legacy.exists() {
            storage::load(legacy)?
        } else {
            Project::default()
        };
        project.name = "Workspace 1".into();
        let entry = Entry {
            id: piratewave_core::recording::RecordSlot::new("").id,
            name: project.name.clone(),
        };
        let store = Self {
            root,
            index: Index {
                active: entry.id,
                entries: vec![entry],
            },
        };
        storage::save(&store.path(store.index.active), &project)?;
        store.persist()?;
        Ok(store)
    }
    pub fn entries(&self) -> &[Entry] {
        &self.index.entries
    }
    pub fn active(&self) -> u64 {
        self.index.active
    }
    pub fn path(&self, id: u64) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }
    fn persist(&self) -> Result<()> {
        storage::save_json(&self.root.join("index.json"), &self.index)
    }
    pub fn load(&self, id: u64) -> Result<Project> {
        let entry = self
            .index
            .entries
            .iter()
            .find(|e| e.id == id)
            .context("Workspace not found")?;
        let mut project = storage::load(&self.path(id))?;
        project.name.clone_from(&entry.name);
        Ok(project)
    }
    pub fn select(&mut self, id: u64) -> Result<()> {
        anyhow::ensure!(
            self.index.entries.iter().any(|e| e.id == id),
            "Workspace not found"
        );
        let old = self.index.active;
        self.index.active = id;
        if let Err(e) = self.persist() {
            self.index.active = old;
            return Err(e);
        }
        Ok(())
    }
    pub fn unique_name(&self, requested: &str) -> String {
        let base = requested.trim();
        let base = if base.is_empty() { "Workspace" } else { base };
        let mut candidate = base.to_string();
        let mut suffix = 2;
        while self.index.entries.iter().any(|e| {
            e.name.eq_ignore_ascii_case(&candidate)
                || e.name.to_lowercase() == candidate.to_lowercase()
        }) {
            candidate = format!("{base} {suffix}");
            suffix += 1;
        }
        candidate
    }
    pub fn add(&mut self, mut project: Project) -> Result<u64> {
        project.name = self.unique_name(&project.name);
        validate_name(&project.name)?;
        let id = piratewave_core::recording::RecordSlot::new("").id;
        storage::save(&self.path(id), &project)?;
        self.index.entries.push(Entry {
            id,
            name: project.name,
        });
        if let Err(e) = self.persist() {
            self.index.entries.pop();
            return Err(e);
        }
        Ok(id)
    }
    /// Delete only after the UI has selected and loaded a replacement workspace.
    pub fn delete(&mut self, id: u64) -> Result<()> {
        anyhow::ensure!(
            id != self.active(),
            "Select another workspace before deleting"
        );
        let position = self
            .index
            .entries
            .iter()
            .position(|e| e.id == id)
            .context("Workspace not found")?;
        let path = self.path(id);
        let staged = path.with_extension("deleted");
        std::fs::rename(&path, &staged).context("Preparing workspace deletion")?;
        let entry = self.index.entries.remove(position);
        if let Err(error) = self.persist() {
            self.index.entries.insert(position, entry);
            std::fs::rename(&staged, &path).context("Restoring workspace after failed deletion")?;
            return Err(error);
        }
        if let Err(error) = std::fs::remove_file(&staged) {
            crate::logging::warn(format_args!(
                "Deleted workspace {id}; temporary file cleanup failed: {error}"
            ));
        }
        Ok(())
    }
    pub fn rename(&mut self, id: u64, name: &str) -> Result<()> {
        let name = name.trim();
        validate_name(name)?;
        if self
            .index
            .entries
            .iter()
            .any(|e| e.id != id && e.name.to_lowercase() == name.to_lowercase())
        {
            bail!("A workspace with that name already exists");
        }
        let entry = self
            .index
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .context("Workspace not found")?;
        let old = std::mem::replace(&mut entry.name, name.into());
        if let Err(e) = self.persist() {
            self.index
                .entries
                .iter_mut()
                .find(|e| e.id == id)
                .unwrap()
                .name = old;
            return Err(e);
        }
        Ok(())
    }
}
fn validate_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.trim().is_empty() && !name.chars().any(char::is_control),
        "Enter a non-empty workspace name without control characters"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clone_numbering_preserves_configuration_and_delete_persists() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-clone-delete-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let mut store = Workspaces::open(root.clone(), &root.join("absent.json")).unwrap();
        let original = store.active();
        store.rename(original, "Studio").unwrap();
        let mut project = store.load(original).unwrap();
        project.nodes[0].gain_db = -13.;
        let second = store.add(project.clone()).unwrap();
        let third = store.add(project.clone()).unwrap();
        assert_eq!(store.load(second).unwrap().name, "Studio 2");
        assert_eq!(store.load(third).unwrap().name, "Studio 3");
        assert_eq!(store.load(second).unwrap().nodes, project.nodes);
        assert!(store.delete(original).is_err());
        store.select(second).unwrap();
        store.delete(original).unwrap();
        assert!(!store.path(original).exists());
        let restored = Workspaces::open(root.clone(), &root.join("absent.json")).unwrap();
        assert_eq!(restored.active(), second);
        assert_eq!(restored.entries().len(), 2);
        assert!(restored.load(original).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn migration_names_import_and_active_selection_roundtrip() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-workspaces-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let legacy = root.join("studio.json");
        let mut project = Project::default();
        project.nodes[0].gain_db = -7.;
        storage::save(&legacy, &project).unwrap();
        let original = std::fs::read(&legacy).unwrap();
        let mut store = Workspaces::open(root.join("managed"), &legacy).unwrap();
        assert_eq!(store.load(store.active()).unwrap().name, "Workspace 1");
        assert_eq!(store.load(store.active()).unwrap().nodes[0].gain_db, -7.);
        assert_eq!(std::fs::read(&legacy).unwrap(), original);
        let first = store.active();
        project.name = "Workspace 1".into();
        let second = store.add(project.clone()).unwrap();
        let third = store.add(project).unwrap();
        assert_eq!(store.load(second).unwrap().name, "Workspace 1 2");
        assert_eq!(store.load(third).unwrap().name, "Workspace 1 3");
        assert!(store.rename(second, " workspace 1 ").is_err());
        assert!(store.rename(second, "  ").is_err());
        store.rename(second, " VR ").unwrap();
        store.select(second).unwrap();
        let restored = Workspaces::open(root.join("managed"), &legacy).unwrap();
        assert_eq!(restored.active(), second);
        assert_eq!(restored.load(second).unwrap().name, "VR");
        assert_eq!(restored.load(first).unwrap().name, "Workspace 1");
        assert!(restored.load(u64::MAX).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_import_and_corrupt_catalog_do_not_replace_workspaces() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-workspaces-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let mut store = Workspaces::open(root.clone(), &root.join("absent.json")).unwrap();
        let invalid = Project {
            version: 999,
            ..Project::default()
        };
        assert!(store.add(invalid).is_err());
        assert_eq!(store.entries().len(), 1);
        std::fs::write(root.join("index.json"), "invalid").unwrap();
        assert!(Workspaces::open(root.clone(), &root.join("absent.json")).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("index.json")).unwrap(),
            "invalid"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
