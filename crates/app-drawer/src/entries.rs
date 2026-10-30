//! Installed applications, read from freedesktop `.desktop` entries.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A launchable application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesktopApp {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) exec: Vec<String>,
    pub(crate) icon: Option<String>,
}

/// The fields of one displayable `[Desktop Entry]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub(crate) name: String,
    pub(crate) exec: Vec<String>,
    pub(crate) icon: Option<String>,
}

const FIELD_CODES: &str = "fFuUicdDnNvmk";

/// Parses a desktop file; `None` when it is hidden, not an application or malformed.
pub(crate) fn parse(contents: &str) -> Option<Parsed> {
    let mut in_entry = false;
    let (mut kind, mut name, mut exec, mut icon) = (None, None, None, None);
    let (mut no_display, mut hidden, mut terminal) = (false, false, false);
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Type" => kind = Some(value.to_string()),
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "Icon" => icon = Some(value.to_string()),
            "NoDisplay" => no_display = value == "true",
            "Hidden" => hidden = value == "true",
            "Terminal" => terminal = value == "true",
            _ => {}
        }
    }
    if kind.as_deref() != Some("Application") || no_display || hidden || terminal {
        return None;
    }
    let name = name.filter(|n| !n.is_empty())?;
    let exec = exec_argv(&exec?)?;
    Some(Parsed {
        name,
        exec,
        icon: icon.filter(|i| !i.is_empty()),
    })
}

/// Turns an `Exec` value into an argv, dropping field codes.
pub(crate) fn exec_argv(exec: &str) -> Option<Vec<String>> {
    // Desktop-entry string escapes first.
    let mut text = String::new();
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => text.push(' '),
            Some('n') => text.push('\n'),
            Some('t') => text.push('\t'),
            Some('r') => text.push('\r'),
            Some('\\') => text.push('\\'),
            Some(other) => {
                text.push('\\');
                text.push(other);
            }
            None => text.push('\\'),
        }
    }

    // Split into arguments; `quoted` marks arguments that were quoted so
    // a quoted `"%f"` is still a field code (it is the whole argument).
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut active = false;
    let mut in_quotes = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => in_quotes = false,
                '\\' => match chars.next() {
                    Some(n @ ('"' | '`' | '$' | '\\')) => cur.push(n),
                    Some(n) => {
                        cur.push('\\');
                        cur.push(n);
                    }
                    None => cur.push('\\'),
                },
                _ => cur.push(c),
            }
        } else if c == '"' {
            in_quotes = true;
            active = true;
        } else if c.is_whitespace() {
            if active {
                args.push(std::mem::take(&mut cur));
                active = false;
            }
        } else {
            cur.push(c);
            active = true;
        }
    }
    if in_quotes {
        return None;
    }
    if active {
        args.push(cur);
    }

    let argv: Vec<String> = args
        .into_iter()
        .filter_map(|arg| expand_codes(&arg))
        .collect();
    (!argv.is_empty()).then_some(argv)
}

/// `None` when the argument is exactly a field code; otherwise `%%` -> `%`
/// and other `%x` removed.
fn expand_codes(arg: &str) -> Option<String> {
    let mut it = arg.chars();
    if let (Some('%'), Some(c), None) = (it.next(), it.next(), it.next())
        && FIELD_CODES.contains(c)
    {
        return None;
    }
    let mut out = String::new();
    let mut chars = arg.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('%') {
            out.push('%');
        }
    }
    Some(out)
}

/// XDG data directories, highest priority first.
pub(crate) fn data_dirs(var: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let non_empty = |name: &str| var(name).filter(|v| !v.is_empty());
    let mut dirs = Vec::new();
    if let Some(home) = non_empty("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = non_empty("HOME") {
        dirs.push(Path::new(&home).join(".local/share"));
    }
    let system = non_empty("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs
}

/// Installed applications from `<dir>/applications` of each data dir, A-Z.
pub(crate) fn scan(data_dirs: &[PathBuf]) -> Vec<DesktopApp> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in data_dirs {
        let root = dir.join("applications");
        let mut files = Vec::new();
        collect(&root, &mut files);
        files.sort();
        for file in files {
            let Ok(rel) = file.strip_prefix(&root) else {
                continue;
            };
            let id = rel.to_string_lossy().replace('/', "-");
            // First ID wins even when that file is filtered out.
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&file) else {
                continue;
            };
            if let Some(p) = parse(&contents) {
                apps.push(DesktopApp {
                    id,
                    name: p.name,
                    exec: p.exec,
                    icon: p.icon,
                });
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "desktop") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|x| x.to_string()).collect())
    }

    #[test]
    fn exec_drops_field_codes() {
        assert_eq!(
            exec_argv("chromium-browser --incognito %U"),
            s(&["chromium-browser", "--incognito"])
        );
    }

    #[test]
    fn exec_handles_quotes() {
        assert_eq!(
            exec_argv(r#""/opt/My App/run" --title "a \"b\"" %f"#),
            s(&["/opt/My App/run", "--title", r#"a "b""#])
        );
    }

    #[test]
    fn exec_percent_escape() {
        assert_eq!(exec_argv("echo 100%%"), s(&["echo", "100%"]));
    }

    #[test]
    fn exec_rejects_bad_input() {
        assert_eq!(exec_argv(r#"run "unterminated"#), None);
        assert_eq!(exec_argv("%U"), None);
    }

    const GOOD: &str = "[Desktop Entry]\nType=Application\nName=Foo\nExec=foo %U\nIcon=foo\n";

    #[test]
    fn parse_accepts_application() {
        let p = parse(GOOD).unwrap();
        assert_eq!(p.name, "Foo");
        assert_eq!(p.exec, vec!["foo"]);
        assert_eq!(p.icon.as_deref(), Some("foo"));
    }

    #[test]
    fn parse_rejects_filtered_and_malformed() {
        for extra in ["NoDisplay=true", "Hidden=true", "Terminal=true"] {
            assert!(parse(&format!("{GOOD}{extra}\n")).is_none(), "{extra}");
        }
        assert!(parse(&GOOD.replace("Application", "Link")).is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=Foo\n").is_none());
    }

    #[test]
    fn parse_ignores_other_groups_and_localised_names() {
        let text = format!("{GOOD}Name[de]=Fu\n[Desktop Action x]\nName=Other\nExec=other\n");
        let p = parse(&text).unwrap();
        assert_eq!(p.name, "Foo");
        assert_eq!(p.exec, vec!["foo"]);
    }

    #[test]
    fn data_dirs_defaults() {
        let dirs = data_dirs(|k| match k {
            "XDG_DATA_DIRS" => Some(String::new()),
            "HOME" => Some("/h".into()),
            _ => None,
        });
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/h/.local/share"),
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ]
        );
    }

    fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("app-drawer-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, contents) in files {
            let p = root.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, contents).unwrap();
        }
        root
    }

    #[test]
    fn scan_user_hidden_shadows_system() {
        let root = tree(
            "shadow",
            &[
                (
                    "a/applications/foo.desktop",
                    &format!("{GOOD}Hidden=true\n"),
                ),
                ("b/applications/foo.desktop", GOOD),
            ],
        );
        let apps = scan(&[root.join("a"), root.join("b")]);
        fs::remove_dir_all(&root).unwrap();
        assert!(apps.is_empty());
    }

    #[test]
    fn scan_sorts_and_ids_subdirs() {
        let root = tree(
            "sort",
            &[
                (
                    "b/applications/kde/zed.desktop",
                    "[Desktop Entry]\nType=Application\nName=Zed\nExec=zed\n",
                ),
                (
                    "b/applications/alpha.desktop",
                    "[Desktop Entry]\nType=Application\nName=alpha\nExec=alpha\n",
                ),
            ],
        );
        let apps = scan(&[root.join("b")]);
        fs::remove_dir_all(&root).unwrap();
        let ids: Vec<_> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["alpha.desktop", "kde-zed.desktop"]);
    }
}
