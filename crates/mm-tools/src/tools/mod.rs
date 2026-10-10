//! The tools this phase ships.
//!
//! Seven modules, one per capability area, and [`default_registry`] is the one place
//! that says which tools exist. It is a function rather than a static list so that a
//! caller can register an extra tool for a test or an operator can drop one, without
//! the shipped set being a property of the build that cannot be inspected.
//!
//! Every tool declares its capabilities in its own spec. Nothing here grants them:
//! [`crate::permissions::TablePermissionEngine`] decides what a caller may exercise,
//! and the executor narrows each tool to the intersection before it runs. A tool
//! registered here but ungranted is refused like any other.
//!
//! | Tool | Tier | Reversibility | Capabilities it declares |
//! |---|---|---|---|
//! | `fs.read` | 1 | reversible | `fs:read` |
//! | `fs.write` | 1 | reversible | `fs:write`, `fs:read` |
//! | `fs.list` | 1 | reversible | `fs:read` |
//! | `process.exec` | 1 | compensatable | `process:execute` |
//! | `graph.query` | 1 | reversible | `graph:read` |
//! | `tabular.query` | 1 | reversible | `sql:read` |
//! | `http.fetch` | 1 | compensatable | `net:connect` |
//! | `verify.run` | 1 | reversible | `process:execute`, `fs:read` |
//! | `pi_editor.open` | 3 | irreversible | `fs:read`, `process:execute` |

pub mod fs;
pub mod graph;
pub mod http;
pub mod pi_editor;
pub mod process;
pub mod tabular;
pub mod verify_build;

pub use fs::{FsListTool, FsReadTool, FsWriteTool};
pub use graph::GraphQueryTool;
pub use http::HttpFetchTool;
pub use pi_editor::PiEditorTool;
pub use process::ProcessExecTool;
pub use tabular::TabularQueryTool;
pub use verify_build::VerifyRunTool;

use std::sync::Arc;

use crate::error::RegistryError;
use crate::registry::ToolRegistry;

/// Every tool this phase ships, registered.
///
/// Returns a result rather than panicking: a duplicate name or an invalid spec is a bug
/// in this module, and a caller that gets a `RegistryError` can say so. The registration
/// order is the table above, and it is stable, so `mm-cli tool list` is diffable.
pub fn default_registry() -> Result<ToolRegistry, RegistryError> {
    let registry = ToolRegistry::new();
    for tool in [
        Arc::new(FsReadTool::new()) as Arc<dyn crate::registry::Tool>,
        Arc::new(FsWriteTool::new()),
        Arc::new(FsListTool::new()),
        Arc::new(ProcessExecTool::new()),
        Arc::new(GraphQueryTool::new()),
        Arc::new(TabularQueryTool::new()),
        Arc::new(HttpFetchTool::new()),
        Arc::new(VerifyRunTool::new()),
        Arc::new(PiEditorTool::new()),
    ] {
        registry.register(tool)?;
    }
    Ok(registry)
}

/// The names [`default_registry`] registers, without building it.
pub fn default_tool_names() -> Vec<&'static str> {
    vec![
        "fs.read",
        "fs.write",
        "fs.list",
        "process.exec",
        "graph.query",
        "tabular.query",
        "http.fetch",
        "verify.run",
        "pi_editor.open",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ToolName;

    #[test]
    fn the_default_registry_holds_every_named_tool() {
        let registry = default_registry().unwrap();
        assert_eq!(registry.len(), default_tool_names().len());
        for name in default_tool_names() {
            let name = ToolName::new(name).unwrap();
            assert!(registry.contains(&name), "{name} is not registered");
            registry.describe(&name).unwrap().validate().unwrap();
        }
    }

    #[test]
    fn every_registered_spec_is_internally_consistent() {
        for spec in default_registry().unwrap().list() {
            spec.validate().unwrap();
            assert!(
                !spec.permissions.is_empty(),
                "{} declares no capability",
                spec.name
            );
            assert!(
                spec.module_uri
                    .starts_with("https://metamind.dev/code/module/tools/"),
                "{} is not attributed to a tool module",
                spec.name
            );
        }
    }

    #[test]
    fn the_safety_class_of_each_tool_is_the_one_the_table_promises() {
        let registry = default_registry().unwrap();
        let spec = |name: &str| registry.describe(&ToolName::new(name).unwrap()).unwrap();
        assert_eq!(
            spec("fs.read").reversibility,
            crate::spec::Reversibility::Reversible
        );
        assert!(spec("fs.read").is_pure());
        assert!(!spec("fs.write").is_pure());
        assert_eq!(
            spec("pi_editor.open").reversibility,
            crate::spec::Reversibility::Irreversible
        );
        assert_eq!(
            spec("pi_editor.open").sandbox_tier,
            crate::sandbox::SandboxTier::MicroVm
        );
    }
}
