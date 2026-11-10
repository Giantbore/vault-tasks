//! App-only task data (notes and lists) that doesn't belong in the Markdown line.
//! Stored per vault in `.vault-tasks.json` at the vault root, so it syncs with the vault.
//! Tasks are matched by their Obsidian block ID (`^vt-abc123`).
use crate::tasks::Task;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

pub const META_FILE: &str = ".vault-tasks.json";

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct TaskMeta {
    /// Markdown notes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Names of the app lists this task belongs to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lists: Vec<String>,
    /// Position set by dragging the task within its group (lower is higher up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
}

impl TaskMeta {
    fn is_empty(&self) -> bool {
        self.notes.trim().is_empty() && self.lists.is_empty() && self.rank.is_none()
    }
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct VaultMeta {
    #[serde(default)]
    tasks: BTreeMap<String, TaskMeta>,
}

#[derive(Default)]
pub struct MetaStore {
    vaults: HashMap<PathBuf, VaultMeta>,
}

impl MetaStore {
    pub fn load(roots: &[PathBuf]) -> Self {
        let vaults = roots
            .iter()
            .map(|r| {
                let m = fs::read_to_string(r.join(META_FILE))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                (r.clone(), m)
            })
            .collect();
        MetaStore { vaults }
    }

    pub fn get(&self, t: &Task) -> Option<&TaskMeta> {
        let id = t.block_id.as_ref()?;
        self.vaults.get(&t.vault_root)?.tasks.get(id)
    }

    pub fn in_list(&self, t: &Task, list: &str) -> bool {
        self.get(t).is_some_and(|m| m.lists.iter().any(|l| l == list))
    }

    /// Every list name used by any task.
    pub fn list_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .vaults
            .values()
            .flat_map(|m| m.tasks.values().flat_map(|t| t.lists.iter().cloned()))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Changes one task's data and saves that vault's file.
    pub fn update(&mut self, root: &Path, id: &str, f: impl FnOnce(&mut TaskMeta)) -> Result<(), String> {
        let vm = self.vaults.entry(root.to_path_buf()).or_default();
        let entry = vm.tasks.entry(id.to_string()).or_default();
        f(entry);
        if entry.is_empty() {
            vm.tasks.remove(id);
        }
        self.save(root)
    }

    /// Renames (or with `to = None`, removes) a list on every task in every vault.
    pub fn rename_list(&mut self, from: &str, to: Option<&str>) -> Result<(), String> {
        let roots: Vec<PathBuf> = self.vaults.keys().cloned().collect();
        for root in roots {
            let vm = self.vaults.get_mut(&root).unwrap();
            let mut changed = false;
            for m in vm.tasks.values_mut() {
                if let Some(i) = m.lists.iter().position(|l| l == from) {
                    changed = true;
                    match to {
                        Some(n) if !m.lists.iter().any(|l| l == n) => m.lists[i] = n.to_string(),
                        _ => {
                            m.lists.remove(i);
                        }
                    }
                }
            }
            if changed {
                vm.tasks.retain(|_, m| !m.is_empty());
                self.save(&root)?;
            }
        }
        Ok(())
    }

    fn save(&self, root: &Path) -> Result<(), String> {
        let Some(vm) = self.vaults.get(root) else { return Ok(()) };
        let path = root.join(META_FILE);
        if vm.tasks.is_empty() && !path.exists() {
            return Ok(());
        }
        let s = serde_json::to_string_pretty(vm).map_err(|e| e.to_string())?;
        fs::write(path, s + "\n").map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_notes() {
        let dir = std::env::temp_dir().join("vt-meta-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let roots = vec![dir.clone()];
        let mut m = MetaStore::load(&roots);
        m.update(&dir, "a", |t| t.lists.push("Errands".into())).unwrap();
        m.update(&dir, "b", |t| t.notes = "- one\n- two".into()).unwrap();
        let m2 = MetaStore::load(&roots);
        assert_eq!(m2.list_names(), vec!["Errands".to_string()]);
        let mut m = m2;
        m.rename_list("Errands", Some("Home")).unwrap();
        assert_eq!(m.list_names(), vec!["Home".to_string()]);
        m.rename_list("Home", None).unwrap();
        let m = MetaStore::load(&roots);
        assert!(m.list_names().is_empty());
        assert_eq!(m.vaults[&dir].tasks.len(), 1);
    }
}
