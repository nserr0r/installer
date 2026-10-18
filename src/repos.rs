use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub file: String,
}

impl Repo {
    pub fn label(&self) -> String {
        format!("{:<30} {}", self.id, self.name)
    }
}

/// Репозитории из всех `*.repo` каталога, по именам файлов.
pub fn list(directory: &Path) -> Vec<Repo> {
    let mut files: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "repo")
        })
        .collect();

    files.sort();
    files
        .iter()
        .filter_map(|path| {
            let text = fs::read_to_string(path).ok()?;

            Some(parse(&path.to_string_lossy(), &text))
        })
        .flatten()
        .collect()
}

pub fn parse(file: &str, text: &str) -> Vec<Repo> {
    let mut repos: Vec<Repo> = Vec::new();

    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if let Some(id) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            repos.push(Repo {
                id: id.trim().to_string(),
                name: id.trim().to_string(),
                enabled: true,
                file: file.to_string(),
            });
            continue;
        }

        let (Some(repo), Some((key, value))) = (repos.last_mut(), line.split_once('=')) else {
            continue;
        };
        let value = value.trim();

        match key.trim() {
            "name" => repo.name = value.to_string(),
            "enabled" => {
                repo.enabled = !matches!(value.to_lowercase().as_str(), "0" | "false" | "no");
            }
            _ => {}
        }
    }

    repos
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# комментарий\n\
        [baseos]\n\
        name=Base OS $releasever\n\
        baseurl=https://mirror.example/baseos\n\
        enabled=1\n\
        \n\
        [baseos-debug]\n\
        name = Base OS Debug\n\
        enabled = 0\n\
        ; ещё комментарий\n\
        [appstream]\n\
        name=AppStream\n\
        metalink=https://mirror.example/meta\n";

    #[test]
    fn sections_names_and_the_enabled_flag_are_read() {
        let repos = parse("/etc/yum.repos.d/a.repo", SAMPLE);

        assert_eq!(
            repos
                .iter()
                .map(|repo| repo.id.as_str())
                .collect::<Vec<_>>(),
            ["baseos", "baseos-debug", "appstream"]
        );
        assert_eq!(repos[0].name, "Base OS $releasever");
        assert_eq!(repos[1].name, "Base OS Debug");
        assert_eq!(
            repos.iter().map(|repo| repo.enabled).collect::<Vec<_>>(),
            [true, false, true],
            "без enabled репозиторий включён"
        );
        assert!(
            repos
                .iter()
                .all(|repo| repo.file == "/etc/yum.repos.d/a.repo")
        );
    }

    #[test]
    fn keys_outside_sections_and_other_keys_are_ignored() {
        let repos = parse("f", "name=orphan\n[x]\nbaseurl=http://a\nexclude=foo\n");

        assert_eq!(repos.len(), 1);
        assert_eq!((repos[0].name.as_str(), repos[0].enabled), ("x", true));
        assert!(parse("f", "").is_empty());
    }

    #[test]
    fn false_spellings_disable_a_repository() {
        for value in ["0", "false", "No", "FALSE"] {
            assert!(
                !parse("f", &format!("[x]\nenabled={value}\n"))[0].enabled,
                "{value}"
            );
        }

        assert!(parse("f", "[x]\nenabled=true\n")[0].enabled);
    }

    #[test]
    fn only_repo_files_are_listed_in_name_order() {
        let root = std::env::temp_dir().join(format!("moonshine-repos-{}", std::process::id()));

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("b.repo"), "[second]\n").unwrap();
        fs::write(root.join("a.repo"), "[first]\n").unwrap();
        fs::write(root.join("notes.txt"), "[ignored]\n").unwrap();

        let repos = list(&root);

        assert_eq!(
            repos
                .iter()
                .map(|repo| repo.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(repos[0].file.ends_with("a.repo"));
        assert!(list(&root.join("missing")).is_empty());
    }

    #[test]
    fn the_label_has_the_id_and_the_name() {
        let repo = &parse("f", "[baseos]\nname=Base\n")[0];

        assert!(repo.label().starts_with("baseos") && repo.label().ends_with("Base"));
    }
}
