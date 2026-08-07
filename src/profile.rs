use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::session::PaneId;
use toml::{Table, Value};

const MAX_TABS: usize = 32;
const MAX_LEAVES: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Profile {
    pub(crate) name: String,
    pub(crate) directory: ResolvedDirectory,
    pub(crate) tabs: Vec<TabRoot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TabRoot {
    pub(crate) node: Node,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Node {
    Leaf {
        target: LaunchTarget,
        directory: ResolvedDirectory,
    },
    Split {
        direction: SplitDirection,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SplitDirection {
    LeftRight,
    TopBottom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LaunchTarget {
    Shell,
    Command {
        executable: PathBuf,
        arguments: Vec<String>,
    },
}

#[allow(dead_code)] // consumed by the T26F launch-plan handoff
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PaneLaunch {
    pub(crate) pane: PaneId,
    pub(crate) target: LaunchTarget,
    pub(crate) directory: ResolvedDirectory,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResolvedDirectory(Option<PathBuf>);

impl ResolvedDirectory {
    #[allow(dead_code)]
    pub(crate) fn path(&self) -> Option<&Path> {
        self.0.as_deref()
    }
}

#[allow(dead_code)] // consumed by the T26E session builder
impl Node {
    pub(crate) fn leaf(&self) -> Option<(&LaunchTarget, &ResolvedDirectory)> {
        match self {
            Self::Leaf { target, directory } => Some((target, directory)),
            Self::Split { .. } => None,
        }
    }

    pub(crate) fn split(&self) -> Option<(SplitDirection, &Node, &Node)> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                direction,
                first,
                second,
            } => Some((*direction, first, second)),
        }
    }
}

pub(crate) fn parse(document: &Table) -> Result<BTreeMap<String, Profile>, String> {
    let Some(value) = document.get("profiles") else {
        return Ok(BTreeMap::new());
    };
    let profiles = value
        .as_table()
        .ok_or_else(|| error("profiles", "expected a table"))?;
    let mut parsed = BTreeMap::new();
    for (name, value) in profiles {
        let path = format!("profiles.{name}");
        validate_name(name, &path)?;
        let profile = parse_profile(name, value, &path)?;
        parsed.insert(name.clone(), profile);
    }
    Ok(parsed)
}

fn parse_profile(name: &str, value: &Value, path: &str) -> Result<Profile, String> {
    let table = table(value, path)?;
    for field in table.keys() {
        if !matches!(field.as_str(), "directory" | "tabs") {
            return Err(error(&format!("{path}.{field}"), "unknown field"));
        }
    }

    let directory = table
        .get("directory")
        .map(|value| parse_directory(value, &format!("{path}.directory")))
        .transpose()?
        .unwrap_or_default();
    let tabs_value = table
        .get("tabs")
        .ok_or_else(|| error(path, "missing required field 'tabs'"))?;
    let tabs = tabs_value
        .as_array()
        .ok_or_else(|| error(&format!("{path}.tabs"), "expected an array"))?;
    if tabs.is_empty() {
        return Err(error(&format!("{path}.tabs"), "must not be empty"));
    }
    if tabs.len() > MAX_TABS {
        return Err(error(
            &format!("{path}.tabs"),
            "must contain at most 32 tabs",
        ));
    }

    let mut roots = Vec::with_capacity(tabs.len());
    for (index, value) in tabs.iter().enumerate() {
        let node_path = format!("{path}.tabs[{index}]");
        let (node, leaves) = parse_node(value, &node_path, &directory)?;
        if leaves > MAX_LEAVES {
            return Err(error(&node_path, "must contain at most 16 leaf panes"));
        }
        roots.push(TabRoot { node });
    }

    Ok(Profile {
        name: name.to_owned(),
        directory,
        tabs: roots,
    })
}

fn parse_node(
    value: &Value,
    path: &str,
    inherited: &ResolvedDirectory,
) -> Result<(Node, usize), String> {
    let table = table(value, path)?;
    let fields = ["shell", "command", "split"]
        .into_iter()
        .filter(|field| table.contains_key(*field))
        .collect::<Vec<_>>();
    for field in table.keys() {
        if !matches!(field.as_str(), "shell" | "command" | "split" | "directory")
            && !(field == "panes" && table.contains_key("split"))
        {
            return Err(error(&format!("{path}.{field}"), "unknown field"));
        }
    }
    if fields.len() != 1 {
        return Err(error(
            path,
            "must contain exactly one of shell, command, or split",
        ));
    }

    match fields[0] {
        "shell" => {
            if table.get("shell").and_then(Value::as_bool) != Some(true) {
                return Err(error(&format!("{path}.shell"), "expected true"));
            }
            let directory = parse_leaf_directory(table, path, inherited)?;
            Ok((
                Node::Leaf {
                    target: LaunchTarget::Shell,
                    directory,
                },
                1,
            ))
        }
        "command" => {
            let command_path = format!("{path}.command");
            let values = table
                .get("command")
                .and_then(Value::as_array)
                .ok_or_else(|| error(&command_path, "expected a non-empty array of strings"))?;
            if values.is_empty() || values.iter().any(|value| value.as_str().is_none()) {
                return Err(error(
                    &command_path,
                    "expected a non-empty array of strings",
                ));
            }
            let strings = values
                .iter()
                .map(|value| value.as_str().expect("validated command string"))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let executable = PathBuf::from(&strings[0]);
            if strings[0].is_empty() || !executable.is_absolute() || strings[0].contains('\0') {
                return Err(error(
                    &format!("{command_path}[0]"),
                    "expected a non-empty absolute executable path",
                ));
            }
            let directory = parse_leaf_directory(table, path, inherited)?;
            Ok((
                Node::Leaf {
                    target: LaunchTarget::Command {
                        executable,
                        arguments: strings.into_iter().skip(1).collect(),
                    },
                    directory,
                },
                1,
            ))
        }
        "split" => {
            if table.contains_key("directory") {
                return Err(error(
                    &format!("{path}.directory"),
                    "split nodes must not specify a directory",
                ));
            }
            let direction_path = format!("{path}.split");
            let direction = match table.get("split").and_then(Value::as_str) {
                Some("left-right") => SplitDirection::LeftRight,
                Some("top-bottom") => SplitDirection::TopBottom,
                _ => {
                    return Err(error(
                        &direction_path,
                        "expected 'left-right' or 'top-bottom'",
                    ));
                }
            };
            let panes_path = format!("{path}.panes");
            let panes = table
                .get("panes")
                .and_then(Value::as_array)
                .ok_or_else(|| error(&panes_path, "expected exactly two child nodes"))?;
            if panes.len() != 2 {
                return Err(error(&panes_path, "expected exactly two child nodes"));
            }
            let (first, first_leaves) =
                parse_node(&panes[0], &format!("{panes_path}[0]"), inherited)?;
            let (second, second_leaves) =
                parse_node(&panes[1], &format!("{panes_path}[1]"), inherited)?;
            Ok((
                Node::Split {
                    direction,
                    first: Box::new(first),
                    second: Box::new(second),
                },
                first_leaves + second_leaves,
            ))
        }
        _ => unreachable!("validated node field"),
    }
}

fn parse_leaf_directory(
    table: &Table,
    path: &str,
    inherited: &ResolvedDirectory,
) -> Result<ResolvedDirectory, String> {
    table
        .get("directory")
        .map(|value| parse_directory(value, &format!("{path}.directory")))
        .transpose()
        .map(|directory| directory.unwrap_or_else(|| inherited.clone()))
}

fn parse_directory(value: &Value, path: &str) -> Result<ResolvedDirectory, String> {
    let value = value
        .as_str()
        .ok_or_else(|| error(path, "expected an absolute directory path"))?;
    let path_value = PathBuf::from(value);
    if value.is_empty() || value.contains('\0') || !path_value.is_absolute() {
        return Err(error(path, "expected an absolute directory path"));
    }
    Ok(ResolvedDirectory(Some(path_value)))
}

fn table<'a>(value: &'a Value, path: &str) -> Result<&'a Table, String> {
    value
        .as_table()
        .ok_or_else(|| error(path, "expected an inline table"))
}

fn validate_name(name: &str, path: &str) -> Result<(), String> {
    if (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(error(path, "profile name must match [A-Za-z0-9_-]{1,64}"))
    }
}

fn error(path: &str, message: &str) -> String {
    format!("configuration field '{path}': {message}")
}

#[cfg(test)]
mod tests {
    use super::{LaunchTarget, Node, ResolvedDirectory, SplitDirection, parse};
    use std::path::Path;
    use toml::{Table, Value};

    fn parse_source(
        source: &str,
    ) -> Result<std::collections::BTreeMap<String, super::Profile>, String> {
        parse(&source.parse::<Table>().map_err(|error| error.to_string())?)
    }

    fn shell() -> &'static str {
        "{ shell = true }"
    }

    #[test]
    fn parses_default_named_profiles_and_targets() {
        let shell = shell();
        let profiles = parse_source(&format!(
            "[profiles.default]\ntabs = [{shell}, {{ command = [\"/bin/echo\", \"hello world\", \"\"] }}]\n\n[profiles.dev]\ntabs = [{{ split = \"left-right\", panes = [{shell}, {{ split = \"top-bottom\", panes = [{shell}, {shell}] }}] }}]"
        ))
        .unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(matches!(
            profiles["default"].tabs[0].node,
            Node::Leaf {
                target: LaunchTarget::Shell,
                ..
            }
        ));
        assert!(matches!(
            profiles["default"].tabs[1].node,
            Node::Leaf {
                target: LaunchTarget::Command { .. },
                ..
            }
        ));
        assert!(matches!(
            profiles["dev"].tabs[0].node,
            Node::Split {
                direction: SplitDirection::LeftRight,
                ..
            }
        ));
    }

    #[test]
    fn resolves_directory_inheritance_and_override_without_filesystem_work() {
        let profiles = parse_source(
            "[profiles.default]\ndirectory = \"/does/not/exist\"\ntabs = [{ shell = true, directory = \"/override\" }, { shell = true }]",
        )
        .unwrap();
        let profile = &profiles["default"];
        assert_eq!(profile.directory.path(), Some(Path::new("/does/not/exist")));
        assert!(
            matches!(&profile.tabs[0].node, Node::Leaf { directory, .. } if directory.path() == Some(Path::new("/override")))
        );
        assert!(
            matches!(&profile.tabs[1].node, Node::Leaf { directory, .. } if directory.path() == Some(Path::new("/does/not/exist")))
        );
    }

    #[test]
    fn rejects_missing_empty_and_oversized_profiles() {
        let error = parse_source("[profiles.default]\ndirectory = \"/tmp\"").unwrap_err();
        assert!(
            error.contains("profiles.default': missing required field"),
            "{error}"
        );
        assert!(
            parse_source("[profiles.default]\ntabs = []")
                .unwrap_err()
                .contains("profiles.default.tabs': must not be empty")
        );
        let tabs = std::iter::repeat(shell())
            .take(33)
            .collect::<Vec<_>>()
            .join(", ");
        assert!(
            parse_source(&format!("[profiles.default]\ntabs = [{tabs}]"))
                .unwrap_err()
                .contains("at most 32 tabs")
        );
        fn split_tree(leaves: usize) -> String {
            if leaves == 1 {
                shell().to_owned()
            } else {
                let first = leaves / 2;
                format!(
                    "{{ split = \"left-right\", panes = [{}, {}] }}",
                    split_tree(first),
                    split_tree(leaves - first)
                )
            }
        }
        let node = split_tree(17);
        assert!(
            parse_source(&format!("[profiles.default]\ntabs = [{node}]"))
                .unwrap_err()
                .contains("at most 16 leaf panes")
        );
    }

    #[test]
    fn rejects_invalid_nodes_with_complete_paths() {
        for (source, path) in [
            (
                "tabs = [{ shell = false }]",
                "profiles.default.tabs[0].shell",
            ),
            (
                "tabs = [{ command = [\"relative\"] }]",
                "profiles.default.tabs[0].command[0]",
            ),
            (
                "tabs = [{ command = [\"\"] }]",
                "profiles.default.tabs[0].command[0]",
            ),
            (
                "tabs = [{ split = \"diagonal\", panes = [{ shell = true }, { shell = true }] }]",
                "profiles.default.tabs[0].split",
            ),
            (
                "tabs = [{ split = \"left-right\", panes = [{ shell = true }] }]",
                "profiles.default.tabs[0].panes",
            ),
            (
                "tabs = [{ shell = true, title = \"tab\" }]",
                "profiles.default.tabs[0].title",
            ),
            (
                "tabs = [{ shell = true, panes = [] }]",
                "profiles.default.tabs[0].panes",
            ),
            (
                "tabs = [{ shell = true, directory = \"relative\" }]",
                "profiles.default.tabs[0].directory",
            ),
            (
                "tabs = [{ split = \"left-right\", directory = \"/tmp\", panes = [{ shell = true }, { shell = true }] }]",
                "profiles.default.tabs[0].directory",
            ),
        ] {
            let source = format!("[profiles.default]\n{source}");
            assert!(
                parse_source(&source).unwrap_err().contains(path),
                "{source}"
            );
        }
        assert!(parse_source("[profiles.bad.name]\ntabs = [{ shell = true }]").is_err());
    }

    #[test]
    fn rejects_conflicting_and_unknown_nested_fields() {
        let error =
            parse_source("[profiles.default]\ntabs = [{ shell = true, command = [\"/bin/sh\"] }]")
                .unwrap_err();
        assert!(error.contains("profiles.default.tabs[0]"));
        let error = parse_source(
            "[profiles.default]\ntabs = [{ split = \"left-right\", panes = [{ shell = true, extra = true }, { shell = true }] }]",
        )
        .unwrap_err();
        assert!(error.contains("profiles.default.tabs[0].panes[0].extra"));
        let error = parse_source(
            "[profiles.default]\ntabs = [{ shell = true, tab_title = \"not supported\" }]",
        )
        .unwrap_err();
        assert!(error.contains("tab_title"));
    }

    #[test]
    fn keeps_absent_directory_unset() {
        let profiles = parse_source("[profiles.default]\ntabs = [{ shell = true }]").unwrap();
        assert_eq!(profiles["default"].directory, ResolvedDirectory::default());
    }

    #[test]
    fn rejects_non_table_profiles() {
        let document = [("profiles".to_owned(), Value::String("bad".into()))]
            .into_iter()
            .collect::<Table>();
        assert!(parse(&document).unwrap_err().contains("profiles"));
    }
}
