//! Project environment files (SPEC §34): a `tpt.toml` beside the code that
//! names a project's workloads and how they depend on each other. `tpt up`
//! / `tpt down` (SPEC §33) drive them through the daemon API in dependency
//! order.
//!
//! ```toml
//! [project]
//! name = "my-project"
//!
//! [[workload]]
//! name = "database"
//! manifest = "manifests/database.toml"     # external tpt.runtime/v1 manifest
//!
//! [[workload]]
//! name = "api"
//! depends_on = ["database"]
//! [workload.manifest.execution]            # or an inline manifest body
//! backend = "windows"
//! program = "server.exe"
//! ```
//!
//! The project entry's `name` is authoritative: it overrides the
//! `[workload] name` of the referenced manifest so dependency references
//! and `tpt up --only` stay unambiguous.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// Label applied to every workload a project starts, so `tpt down` finds
/// its own across the daemon's workload list.
pub const PROJECT_LABEL: &str = "tpt.project";

/// A parsed, resolved project environment.
#[derive(Debug, Clone)]
pub struct Project {
    /// Directory containing the project file; `manifest =` paths resolve
    /// relative to it.
    pub dir: PathBuf,
    /// Project name (`[project] name`, else the directory name).
    pub name: String,
    workloads: Vec<ProjectWorkload>,
}

/// One workload entry of a project, resolved to manifest text.
#[derive(Debug, Clone)]
pub struct ProjectWorkload {
    /// Project-local workload name (authoritative).
    pub name: String,
    /// Names of workloads that must start first.
    pub depends_on: Vec<String>,
    /// The `tpt.runtime/v1` manifest text (external file or inline).
    pub manifest: String,
}

impl Project {
    /// Loads a project file (`tpt.toml`).
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|err| {
            RuntimeError::new(
                ErrorKind::NotFound,
                format!("cannot read project file {}: {err}", path.display()),
            )
        })?;
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::parse(&text, &dir)
    }

    /// Parses project text with manifests resolved against `dir`.
    pub fn parse(text: &str, dir: &Path) -> Result<Self> {
        let mut table: toml::Table = toml::from_str(text).map_err(|err| {
            RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("invalid project file: {err}"),
            )
        })?;

        let name = match table.remove("project") {
            Some(toml::Value::Table(project)) => project
                .get("name")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            Some(_) => return Err(invalid("[project] must be a table")),
            None => None,
        }
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "project".to_owned())
        });

        let entries = match table.remove("workload") {
            Some(toml::Value::Array(entries)) => entries,
            Some(_) => return Err(invalid("[[workload]] must be an array of tables")),
            None => return Err(invalid("project file requires at least one [[workload]]")),
        };

        let mut workloads = Vec::with_capacity(entries.len());
        for entry in entries {
            workloads.push(parse_workload(entry, dir)?);
        }

        let project = Self {
            dir: dir.to_path_buf(),
            name,
            workloads,
        };
        project.validate_references()?;
        Ok(project)
    }

    /// All workload entries in declaration order.
    pub fn workloads(&self) -> &[ProjectWorkload] {
        &self.workloads
    }

    /// Lookup one entry by name.
    pub fn get(&self, name: &str) -> Option<&ProjectWorkload> {
        self.workloads.iter().find(|w| w.name == name)
    }

    /// Start order: dependencies before dependents. With `only`, the order
    /// is restricted to that workload plus its transitive dependencies.
    pub fn start_order(&self, only: Option<&str>) -> Result<Vec<&ProjectWorkload>> {
        let mut selected: BTreeSet<&str> = BTreeSet::new();
        if let Some(only) = only {
            self.collect_deps(only, &mut selected)?;
        } else {
            selected.extend(self.workloads.iter().map(|w| w.name.as_str()));
        }

        let mut ordered = Vec::with_capacity(selected.len());
        let mut done: BTreeSet<&str> = BTreeSet::new();
        // Deterministic: repeatedly take the first (declaration order)
        // workload whose dependencies are all done.
        loop {
            let next = self.workloads.iter().find(|w| {
                selected.contains(w.name.as_str())
                    && !done.contains(w.name.as_str())
                    && w.depends_on.iter().all(|dep| done.contains(dep.as_str()))
            });
            match next {
                Some(w) => {
                    done.insert(w.name.as_str());
                    ordered.push(w);
                }
                None => break,
            }
        }
        if ordered.len() != selected.len() {
            // Something in `selected` never became ready: a dependency cycle.
            let stuck: Vec<&str> = selected.difference(&done).copied().collect();
            return Err(invalid(&format!(
                "dependency cycle in project '{}': unreachable workloads {stuck:?}",
                self.name
            )));
        }
        Ok(ordered)
    }

    /// Stop order: the exact reverse of the start order.
    pub fn stop_order(&self) -> Result<Vec<&ProjectWorkload>> {
        let mut ordered = self.start_order(None)?;
        ordered.reverse();
        Ok(ordered)
    }

    fn collect_deps<'a>(&'a self, name: &'a str, out: &mut BTreeSet<&'a str>) -> Result<()> {
        if !out.insert(name) {
            return Ok(());
        }
        let workload = self
            .get(name)
            .ok_or_else(|| invalid(&format!("project '{}' has no workload '{name}'", self.name)))?;
        for dep in &workload.depends_on {
            self.collect_deps(dep, out)?;
        }
        Ok(())
    }

    fn validate_references(&self) -> Result<()> {
        let mut seen = BTreeSet::new();
        for workload in &self.workloads {
            if !seen.insert(workload.name.as_str()) {
                return Err(invalid(&format!(
                    "duplicate workload name '{}' in project '{}'",
                    workload.name, self.name
                )));
            }
        }
        for workload in &self.workloads {
            for dep in &workload.depends_on {
                if !seen.contains(dep.as_str()) {
                    return Err(invalid(&format!(
                        "workload '{}' depends on unknown workload '{dep}'",
                        workload.name
                    )));
                }
            }
        }
        // Detect cycles eagerly with a full ordering pass.
        self.start_order(None)?;
        Ok(())
    }
}

fn parse_workload(entry: toml::Value, dir: &Path) -> Result<ProjectWorkload> {
    let toml::Value::Table(mut entry) = entry else {
        return Err(invalid("[[workload]] entries must be tables"));
    };

    let name = match entry.remove("name") {
        Some(toml::Value::String(name)) => name,
        _ => return Err(invalid("[[workload]] requires a string 'name'")),
    };

    let depends_on = match entry.remove("depends_on") {
        None => Vec::new(),
        Some(toml::Value::Array(deps)) => {
            let mut out = Vec::with_capacity(deps.len());
            for dep in deps {
                match dep {
                    toml::Value::String(dep) => out.push(dep),
                    _ => return Err(invalid("'depends_on' entries must be strings")),
                }
            }
            out
        }
        Some(_) => return Err(invalid("'depends_on' must be an array of names")),
    };

    let manifest = match entry.remove("manifest") {
        Some(toml::Value::String(path)) => {
            let path = dir.join(path);
            std::fs::read_to_string(&path).map_err(|err| {
                RuntimeError::new(
                    ErrorKind::NotFound,
                    format!("cannot read manifest {}: {err}", path.display()),
                )
            })?
        }
        Some(toml::Value::Table(inline)) => {
            // Hoist the inline body to the manifest top level and add the
            // required api + [workload] sections.
            let mut manifest_table = toml::Table::new();
            manifest_table.insert(
                "api".to_owned(),
                toml::Value::String(crate::manifest::MANIFEST_API_VERSION.to_owned()),
            );
            let mut workload_table = toml::Table::new();
            workload_table.insert("name".to_owned(), toml::Value::String(name.clone()));
            manifest_table.insert("workload".to_owned(), toml::Value::Table(workload_table));
            for (key, value) in inline {
                manifest_table.insert(key, value);
            }
            toml::to_string_pretty(&manifest_table)
                .map_err(|err| invalid(&format!("inline manifest is not serializable: {err}")))?
        }
        Some(_) => return Err(invalid("'manifest' must be a path or an inline table")),
        None => {
            return Err(invalid(&format!(
                "workload '{name}' requires 'manifest' (a path or an inline table)"
            )))
        }
    };

    if !entry.is_empty() {
        let unknown: Vec<String> = entry.keys().cloned().collect();
        return Err(invalid(&format!(
            "unknown keys in [[workload]] '{name}': {unknown:?} (allowed: name, depends_on, manifest)"
        )));
    }

    Ok(ProjectWorkload {
        name,
        depends_on,
        manifest,
    })
}

/// Prepares manifest text for `workloads.create`: makes the project entry's
/// name authoritative and tags the workload with the project label.
pub fn materialize(manifest_text: &str, name: &str, project: &str) -> Result<String> {
    let mut table: toml::Table = toml::from_str(manifest_text).map_err(|err| {
        RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("invalid manifest: {err}"),
        )
    })?;

    match table.get_mut("workload") {
        Some(toml::Value::Table(workload)) => {
            workload.insert("name".to_owned(), toml::Value::String(name.to_owned()));
        }
        _ => return Err(invalid("manifest requires a [workload] table")),
    }

    let labels = table
        .entry("labels")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(labels) = labels {
        labels.insert(
            PROJECT_LABEL.to_owned(),
            toml::Value::String(project.to_owned()),
        );
    }

    toml::to_string_pretty(&table)
        .map_err(|err| invalid(&format!("manifest is not serializable: {err}")))
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidConfiguration, message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpt-project-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("manifests")).unwrap();
        dir
    }

    const EXTERNAL_MANIFEST: &str = r#"
api = "tpt.runtime/v1"
[workload]
name = "ignored-project-name-wins"
[execution]
backend = "windows"
program = "cmd.exe"
args = ["/C", "echo db"]
"#;

    #[test]
    fn parses_inline_and_external_manifests() {
        let dir = dir("parse");
        std::fs::write(dir.join("manifests").join("db.toml"), EXTERNAL_MANIFEST).unwrap();
        let project = Project::parse(
            r#"
[project]
name = "demo"

[[workload]]
name = "database"
manifest = "manifests/db.toml"

[[workload]]
name = "api"
depends_on = ["database"]

[workload.manifest.execution]
backend = "windows"
program = "ping.exe"
args = ["-n", "5", "127.0.0.1"]
"#,
            &dir,
        )
        .unwrap();

        assert_eq!(project.name, "demo");
        assert_eq!(project.workloads().len(), 2);

        let db = project.get("database").unwrap();
        assert!(db.depends_on.is_empty());
        assert!(db.manifest.contains("echo db"));

        let api = project.get("api").unwrap();
        assert_eq!(api.depends_on, ["database"]);
        // The inline body became a parseable tpt.runtime/v1 manifest.
        let manifest = crate::Manifest::parse(&api.manifest).unwrap();
        assert_eq!(manifest.workload.name, "api");
    }

    #[test]
    fn start_order_respects_dependencies_and_only() {
        let dir = dir("order");
        let project = Project::parse(
            r#"
[[workload]]
name = "api"
depends_on = ["db", "cache"]
[workload.manifest.execution]
backend = "windows"
program = "api.exe"

[[workload]]
name = "cache"
[workload.manifest.execution]
backend = "windows"
program = "cache.exe"

[[workload]]
name = "db"
[workload.manifest.execution]
backend = "windows"
program = "db.exe"

[[workload]]
name = "unrelated"
[workload.manifest.execution]
backend = "windows"
program = "other.exe"
"#,
            &dir,
        )
        .unwrap();

        let names = |order: Vec<&ProjectWorkload>| -> Vec<String> {
            order.into_iter().map(|w| w.name.clone()).collect()
        };
        assert_eq!(
            names(project.start_order(None).unwrap()),
            ["cache", "db", "api", "unrelated"]
        );
        // Both dependencies are ready first; declaration order breaks ties.
        assert_eq!(
            names(project.start_order(Some("api")).unwrap()),
            ["cache", "db", "api"]
        );
        assert_eq!(
            names(project.start_order(Some("cache")).unwrap()),
            ["cache"]
        );
        assert_eq!(
            names(project.stop_order().unwrap()),
            ["unrelated", "api", "db", "cache"]
        );
    }

    #[test]
    fn rejects_cycles_duplicates_and_unknown_deps() {
        let dir = dir("reject");
        let proc = r#"
[workload.manifest.execution]
backend = "windows"
program = "a.exe"
"#;
        let cycle = Project::parse(
            &format!(
                r#"
[[workload]]
name = "a"
depends_on = ["b"]
{proc}
[[workload]]
name = "b"
depends_on = ["a"]
{proc}
"#
            ),
            &dir,
        );
        assert_eq!(cycle.unwrap_err().kind, ErrorKind::InvalidConfiguration);

        let dup = Project::parse(
            &format!(
                r#"
[[workload]]
name = "a"
{proc}
[[workload]]
name = "a"
{proc}
"#
            ),
            &dir,
        );
        assert_eq!(dup.unwrap_err().kind, ErrorKind::InvalidConfiguration);

        let unknown = Project::parse(
            &format!(
                r#"
[[workload]]
name = "a"
depends_on = ["ghost"]
{proc}
"#
            ),
            &dir,
        );
        assert_eq!(unknown.unwrap_err().kind, ErrorKind::InvalidConfiguration);

        let unknown_only = Project::parse(
            r#"
[[workload]]
name = "a"
[workload.manifest.execution]
backend = "windows"
program = "a.exe"
"#,
            &dir,
        )
        .unwrap();
        assert_eq!(
            unknown_only.start_order(Some("ghost")).unwrap_err().kind,
            ErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn materialize_overrides_name_and_labels_project() {
        let text = materialize(EXTERNAL_MANIFEST, "database", "demo").unwrap();
        let manifest = crate::Manifest::parse(&text).unwrap();
        assert_eq!(manifest.workload.name, "database");
        assert_eq!(
            manifest.labels.get(PROJECT_LABEL).map(String::as_str),
            Some("demo")
        );

        // An existing labels table keeps its other entries.
        let with_labels = format!("{EXTERNAL_MANIFEST}\n[labels]\nteam = \"platform\"\n");
        let manifest =
            crate::Manifest::parse(&materialize(&with_labels, "db", "demo").unwrap()).unwrap();
        assert_eq!(
            manifest.labels.get("team").map(String::as_str),
            Some("platform")
        );
        assert_eq!(
            manifest.labels.get(PROJECT_LABEL).map(String::as_str),
            Some("demo")
        );
    }

    #[test]
    fn defaults_project_name_to_directory() {
        let project = Project::parse(
            "[[workload]]\nname = \"a\"\n[workload.manifest.execution]\nbackend = \"windows\"\nprogram = \"a.exe\"\n",
            Path::new("/tmp/somewhere/my-proj"),
        )
        .unwrap();
        assert_eq!(project.name, "my-proj");
    }
}
