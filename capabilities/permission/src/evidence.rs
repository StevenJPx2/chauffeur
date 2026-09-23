//! Deterministic evidence for permission contracts, computed from the
//! request's facts. Paths arrive resolved by the host; containment here is
//! lexical over normalized absolute paths.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use chauffeur_core::{Resource, SignalKind};

pub const RESOURCE_PRESENT: &str = "resource_present";
pub const RESOURCE_WITHIN_WORKSPACE: &str = "resource_within_workspace";
pub const RESOURCE_NAMED_BY_USER: &str = "resource_named_by_user";

pub fn collect(kind: &SignalKind) -> HashMap<String, bool> {
    let SignalKind::PermissionRequest {
        resources,
        workspace,
        user_requests,
        ..
    } = kind
    else {
        return HashMap::new();
    };
    let present = !resources.is_empty();
    let user_text = user_requests.join("\n").to_lowercase();

    HashMap::from([
        (RESOURCE_PRESENT.to_string(), present),
        (
            RESOURCE_WITHIN_WORKSPACE.to_string(),
            present && resources.iter().all(|resource| within(workspace, resource)),
        ),
        (
            RESOURCE_NAMED_BY_USER.to_string(),
            present && resources.iter().all(|resource| named(&user_text, resource)),
        ),
    ])
}

fn within(workspace: &str, resource: &Resource) -> bool {
    match (normalize(workspace), normalize(&resource.resolved)) {
        (Some(root), Some(path)) => path.starts_with(root),
        _ => false,
    }
}

/// Absolute path with `.` and `..` resolved lexically; `None` if relative or
/// it climbs above the root.
fn normalize(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);

    if !path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() || normalized.as_os_str().is_empty() {
                    return None;
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    Some(normalized)
}

fn named(user_text: &str, resource: &Resource) -> bool {
    let requested = resource.requested.to_lowercase();
    let file_name = Path::new(&resource.requested)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase());

    (!requested.is_empty() && user_text.contains(&requested))
        || file_name.is_some_and(|name| !name.is_empty() && user_text.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(requested: &str, resolved: &str) -> Resource {
        Resource {
            requested: requested.into(),
            resolved: resolved.into(),
        }
    }

    #[test]
    fn containment_is_lexical_over_normalized_paths() {
        assert!(within(
            "/work/app",
            &resource("src/a.rs", "/work/app/src/a.rs")
        ));
        assert!(!within("/work/app", &resource("../x", "/work/app/../x")));
        assert!(!within("/work/app", &resource("x", "/work/application/x")));
        assert!(!within("/work/app", &resource("x", "relative/x")));
    }

    #[test]
    fn a_resource_is_named_by_path_or_file_name() {
        assert!(named(
            "please update readme.md",
            &resource("docs/README.md", "/w/docs/README.md")
        ));
        assert!(!named(
            "fix the tests",
            &resource("src/lib.rs", "/w/src/lib.rs")
        ));
    }
}
