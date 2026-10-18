use std::fs;
use std::io::Read;
use std::path::Path;

use crate::disks;
use crate::disks::Disk;
use crate::hardware::Hardware;
use crate::repos;
use crate::repos::Repo;

const REPOS: &str = "/etc/yum.repos.d";
const ZONEINFO: &str = "/usr/share/zoneinfo";
const ZONE_SKIPPED: [&str; 3] = ["posix", "right", "SystemV"];

/// Всё, из чего пользователь выбирает значения в мастере.
#[derive(Debug, Default)]
pub struct Sources {
    pub disks: Vec<Disk>,
    pub timezones: Vec<String>,
    pub interfaces: Vec<String>,
    pub repos: Vec<Repo>,
    pub hardware: Hardware,
}

impl Sources {
    pub fn detect() -> Self {
        Self {
            disks: disks::list(),
            timezones: timezones(Path::new(ZONEINFO)),
            interfaces: interfaces(Path::new("/sys/class/net")),
            repos: repos::list(Path::new(REPOS)),
            hardware: Hardware::detect(),
        }
    }
}

/// Имена часовых поясов: файлы TZif в базе зон, без `posix` и `right`.
pub fn timezones(root: &Path) -> Vec<String> {
    let mut zones = Vec::new();

    collect_zones(root, "", &mut zones);
    zones.sort();
    zones
}

fn collect_zones(directory: &Path, prefix: &str, zones: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };

        if metadata.is_dir() {
            if !(prefix.is_empty() && ZONE_SKIPPED.contains(&name.as_str())) {
                collect_zones(&path, &format!("{prefix}{name}/"), zones);
            }
        } else if is_zone_file(&path) {
            zones.push(format!("{prefix}{name}"));
        }
    }
}

fn is_zone_file(path: &Path) -> bool {
    let mut magic = [0; 4];

    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && &magic == b"TZif"
}

/// Сетевые интерфейсы из `/sys/class/net`, кроме `lo`.
pub fn interfaces(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "lo")
        .collect();

    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("moonshine-sources-{name}-{}", std::process::id()));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn timezones_are_tzif_files_without_posix_and_junk() {
        let root = scratch("zones");

        fs::create_dir_all(root.join("Europe")).unwrap();
        fs::create_dir_all(root.join("posix/Europe")).unwrap();
        fs::write(root.join("Europe/Moscow"), b"TZif2....").unwrap();
        fs::write(root.join("UTC"), b"TZif2....").unwrap();
        fs::write(root.join("posix/Europe/Moscow"), b"TZif2....").unwrap();
        fs::write(root.join("zone.tab"), b"# table").unwrap();
        fs::write(root.join("tiny"), b"T").unwrap();

        assert_eq!(timezones(&root), ["Europe/Moscow", "UTC"]);
        assert!(timezones(&root.join("missing")).is_empty());
    }

    #[test]
    fn interfaces_skip_the_loopback() {
        let root = scratch("net");

        for name in ["lo", "eth0", "enp3s0"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }

        assert_eq!(interfaces(&root), ["enp3s0", "eth0"]);
    }
}
