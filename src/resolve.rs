use crate::i18n::tr;
use std::fs;
use std::process::Command;

/// Временный корень для расчёта в памяти (`/run`): туда dnf складывает кэш метаданных, ничего не устанавливая.
pub const PREVIEW_ROOT: &str = "/run/moonshine/resolve";

const ARCHES: [&str; 10] = [
    "noarch", "x86_64", "i686", "i386", "aarch64", "armv7hl", "ppc64le", "s390x", "riscv64", "src",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub arch: String,
    pub version: String,
    pub repo: String,
}

/// Устанавливаемые пакеты из таблицы транзакции `dnf install --assumeno`, по алфавиту.
pub fn parse(output: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    let mut collecting = false;
    let mut pending: Option<String> = None;

    for line in output.lines() {
        if line.starts_with("Transaction Summary") {
            break;
        }

        if !line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            collecting = line.starts_with("Installing") && line.trim_end().ends_with(':');
            pending = None;
            continue;
        }

        if !collecting {
            continue;
        }

        let columns: Vec<&str> = line.split_whitespace().collect();

        match columns.as_slice() {
            [name, arch, version, repo, ..] if ARCHES.contains(arch) => {
                packages.push(package(name, arch, version, repo));
                pending = None;
            }
            [arch, version, repo, ..] if ARCHES.contains(arch) && pending.is_some() => {
                let name = pending.take().unwrap_or_default();

                packages.push(package(&name, arch, version, repo));
            }
            [name] => pending = Some((*name).to_string()),
            _ => pending = None,
        }
    }

    packages.sort_by(|left, right| {
        (left.name.as_str(), left.arch.as_str()).cmp(&(right.name.as_str(), right.arch.as_str()))
    });
    packages
}

fn package(name: &str, arch: &str, version: &str, repo: &str) -> Package {
    Package {
        name: name.into(),
        arch: arch.into(),
        version: version.into(),
        repo: repo.into(),
    }
}

/// Запускает `dnf` с готовыми аргументами (`--assumeno`) и разбирает таблицу.
pub fn run(args: &[String]) -> Result<Vec<Package>, String> {
    fs::create_dir_all(PREVIEW_ROOT).map_err(|error| format!("{PREVIEW_ROOT}: {error}"))?;

    let output = Command::new("dnf")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| {
            tr!(
                "failed to start dnf: {error}",
                "не удалось запустить dnf: {error}"
            )
        });
    let _ = fs::remove_dir_all(PREVIEW_ROOT);
    let output = output?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let packages = parse(&text);

    if packages.is_empty() {
        let tail: Vec<&str> = text.lines().rev().take(12).collect();

        return Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }

    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DNF4: &str = "\
Last metadata expiration check: 0:01:00 ago.
Dependencies resolved.
================================================================================
 Package                 Architecture  Version            Repository      Size
================================================================================
Installing:
 kernel                  x86_64        6.8.5-301.fc40     updates         11 k
 bash                    x86_64        5.2.26-3.fc40      fedora         1.8 M
Installing group/module packages:
 vim-minimal             x86_64        2:9.1-1.fc40       updates        700 k
Installing dependencies:
 kernel-core             x86_64        6.8.5-301.fc40     updates         26 M
 glibc-common            x86_64        2.39-1.fc40        fedora         400 k
 tzdata                  noarch        2024a-1.fc40       fedora         716 k
 a-very-long-package-name-that-wraps-in-the-table
                         noarch        1.0-1.fc40         fedora          12 k
Installing weak dependencies:
 dracut-config-rescue    x86_64        102-1.fc40         fedora          10 k
Installing Groups:
 Core

Transaction Summary
================================================================================
Install  8 Packages

Total download size: 60 M
Installed size: 200 M
Operation aborted.
";

    const DNF5: &str = "\
Updating and loading repositories:
Repositories loaded.
Package          Arch   Version             Repository      Size
Installing:
 kernel          x86_64 6.8.5-301.fc40      updates     11.0 KiB
Installing dependencies:
 zlib-ng-compat  x86_64 2.1.6-3.fc40        fedora      64.0 KiB
Installing weak dependencies:
 efibootmgr      x86_64 18-6.fc40           fedora      40.0 KiB

Transaction Summary:
 Installing:         3 packages
";

    fn names(packages: &[Package]) -> Vec<&str> {
        packages
            .iter()
            .map(|package| package.name.as_str())
            .collect()
    }

    #[test]
    fn dnf4_tables_are_read_sorted_with_all_sections() {
        let packages = parse(DNF4);

        assert_eq!(
            names(&packages),
            [
                "a-very-long-package-name-that-wraps-in-the-table",
                "bash",
                "dracut-config-rescue",
                "glibc-common",
                "kernel",
                "kernel-core",
                "tzdata",
                "vim-minimal"
            ]
        );
        assert_eq!(packages[1].version, "5.2.26-3.fc40");
        assert_eq!(
            (packages[6].arch.as_str(), packages[6].repo.as_str()),
            ("noarch", "fedora")
        );
        assert_eq!(packages[7].version, "2:9.1-1.fc40");
    }

    #[test]
    fn dnf5_tables_are_read_too() {
        let packages = parse(DNF5);

        assert_eq!(names(&packages), ["efibootmgr", "kernel", "zlib-ng-compat"]);
        assert_eq!(packages[1].repo, "updates");
    }

    #[test]
    fn group_names_headers_and_summaries_are_not_packages() {
        let packages = parse(DNF4);

        assert!(!names(&packages).contains(&"Core"));
        assert!(!names(&packages).contains(&"Package"));
        assert!(parse("Nothing to do.\n").is_empty());
        assert!(parse("").is_empty());
    }

    #[test]
    fn upgrades_and_removals_are_not_counted() {
        let output =
            "Upgrading:\n bash x86_64 5.2-1 updates 1 M\nRemoving:\n vim x86_64 9 @System 1 M\n";

        assert!(parse(output).is_empty());
    }
}
