use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::process::Command;

use crate::error::Error;
use crate::i18n::tr;
use crate::i18n::tt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    pub size: u64,
    pub model: String,
    pub removable: bool,
    pub medium: Medium,
    /// Постоянный путь из `/dev/disk/by-id`, а если его нет — обычный.
    pub stable: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    Nvme,
    Ssd,
    Hdd,
    Unknown,
}

impl Medium {
    fn name(self) -> &'static str {
        match self {
            Medium::Nvme => "NVMe",
            Medium::Ssd => "SSD",
            Medium::Hdd => "HDD",
            Medium::Unknown => "?",
        }
    }

    fn detect(transport: Option<&str>, rotational: Option<&str>) -> Self {
        match (transport, rotational) {
            (Some("nvme"), _) => Medium::Nvme,
            (_, Some("0")) => Medium::Ssd,
            (_, Some("1")) => Medium::Hdd,
            _ => Medium::Unknown,
        }
    }
}

impl Disk {
    pub fn label(&self) -> String {
        let model = if self.model.is_empty() {
            tt!("unnamed", "без названия")
        } else {
            &self.model
        };
        let removable = if self.removable {
            tt!("  (removable)", "  (съёмный)")
        } else {
            ""
        };

        format!(
            "{:<14} {:<5} {:>9}  {model}{removable}",
            self.path,
            self.medium.name(),
            human(self.size)
        )
    }
}

pub fn human(bytes: u64) -> String {
    let units = tt!(
        ["B", "KiB", "MiB", "GiB", "TiB"],
        ["Б", "КиБ", "МиБ", "ГиБ", "ТиБ"]
    );

    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", units[0])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// Размер блочного устройства в байтах по `/sys/class/block`; ссылки вроде `/dev/disk/by-id/…` раскрываются.
pub fn size_of(path: &str) -> Option<u64> {
    let real = std::fs::canonicalize(path).ok()?;
    let name = real.file_name()?.to_str()?;
    let sectors: u64 = std::fs::read_to_string(format!("/sys/class/block/{name}/size"))
        .ok()?
        .trim()
        .parse()
        .ok()?;

    sectors.checked_mul(512)
}

/// Стирать можно только блочное устройство; один диск нельзя указать под двумя именами.
pub fn check_blocks(disks: &[String]) -> Result<(), Error> {
    let mut real = Vec::new();

    for disk in disks {
        let is_block = std::fs::metadata(disk).is_ok_and(|meta| meta.file_type().is_block_device());

        if !is_block {
            return Err(Error::Invalid(tr!(
                "{disk}: not a block device",
                "{disk}: это не блочное устройство"
            )));
        }

        real.push(std::fs::canonicalize(disk).unwrap_or_else(|_| disk.into()));
    }

    match first_duplicate(&real) {
        Some(path) => Err(Error::Invalid(tr!(
            "{}: the same disk is given under different names",
            "{}: один и тот же диск указан под разными именами",
            path.display()
        ))),
        None => Ok(()),
    }
}

fn first_duplicate(paths: &[PathBuf]) -> Option<&PathBuf> {
    paths
        .iter()
        .enumerate()
        .find(|(index, path)| paths[..*index].contains(path))
        .map(|(_, path)| path)
}

/// Диски системы: вывод `lsblk` без разделов, зеркал и загрузочного носителя только для чтения.
pub fn list() -> Vec<Disk> {
    Command::new("lsblk")
        .args([
            "--nodeps",
            "--noheadings",
            "--bytes",
            "--paths",
            "--pairs",
            "--output",
            "NAME,SIZE,MODEL,TYPE,RM,RO,ROTA,TRAN",
        ])
        .output()
        .map(|output| parse(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
        .into_iter()
        .map(|mut disk| {
            if let Some(path) = stable_path(&disk.path) {
                disk.stable = path;
            }

            disk
        })
        .collect()
}

/// Ссылка в `/dev/disk/by-id`, указывающая на сам диск, а не на его раздел.
fn stable_path(disk: &str) -> Option<String> {
    let target = std::fs::canonicalize(disk).ok()?;
    let mut links: Vec<String> = std::fs::read_dir("/dev/disk/by-id")
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            !path.to_string_lossy().contains("-part")
                && std::fs::canonicalize(path).is_ok_and(|real| real == target)
        })
        .map(|path| path.to_string_lossy().into_owned())
        .collect();

    links.sort();
    links.into_iter().next()
}

pub fn parse(output: &str) -> Vec<Disk> {
    output
        .lines()
        .filter_map(|line| {
            let pairs = pairs(line);
            let get = |key: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.as_str())
            };

            if get("TYPE")? != "disk" || get("RO")? == "1" {
                return None;
            }

            Some(Disk {
                path: get("NAME")?.to_string(),
                size: get("SIZE")?.parse().ok()?,
                model: get("MODEL").unwrap_or_default().trim().to_string(),
                removable: get("RM") == Some("1"),
                medium: Medium::detect(get("TRAN"), get("ROTA")),
                stable: get("NAME")?.to_string(),
            })
        })
        .collect()
}

/// `KEY="значение"` через пробел; в значениях `\xNN` — байт в шестнадцатеричном виде.
fn pairs(line: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut rest = line.trim();

    while let Some((key, after)) = rest.split_once("=\"") {
        let Some(end) = closing_quote(after) else {
            break;
        };

        result.push((key.trim().to_string(), unescape(&after[..end])));
        rest = after[end + 1..].trim_start();
    }

    result
}

fn closing_quote(text: &str) -> Option<usize> {
    let mut escaped = false;

    for (index, symbol) in text.char_indices() {
        match symbol {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(index),
            _ => escaped = false,
        }
    }

    None
}

fn unescape(text: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(symbol) = chars.next() {
        if symbol == '\\' && chars.peek() == Some(&'x') {
            let hex: String = chars.clone().skip(1).take(2).collect();

            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                bytes.push(byte);
                chars.nth(2);
                continue;
            }
        }

        let mut buffer = [0; 4];

        bytes.extend_from_slice(symbol.encode_utf8(&mut buffer).as_bytes());
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"NAME="/dev/sda" SIZE="512110190592" MODEL="Samsung\x20SSD\x20870" TYPE="disk" RM="0" RO="0" ROTA="0" TRAN="sata"
NAME="/dev/sdb" SIZE="16008609792" MODEL="" TYPE="disk" RM="1" RO="0" ROTA="1" TRAN="usb"
NAME="/dev/sr0" SIZE="1073741312" MODEL="QEMU DVD-ROM" TYPE="rom" RM="1" RO="0"
NAME="/dev/loop0" SIZE="2000000000" MODEL="" TYPE="loop" RM="0" RO="1"
NAME="/dev/nvme0n1" SIZE="1000204886016" MODEL="WD Black" TYPE="disk" RM="0" RO="0" ROTA="0" TRAN="nvme"
"#;

    #[test]
    fn only_writable_disks_are_listed_with_their_models() {
        let disks = parse(SAMPLE);

        assert_eq!(
            disks
                .iter()
                .map(|disk| disk.path.as_str())
                .collect::<Vec<_>>(),
            ["/dev/sda", "/dev/sdb", "/dev/nvme0n1"]
        );
        assert_eq!(disks[0].model, "Samsung SSD 870");
        assert!(disks[1].removable && !disks[0].removable);
        assert_eq!(disks[0].size, 512_110_190_592);
    }

    #[test]
    fn the_medium_comes_from_the_transport_and_rotation() {
        let disks = parse(SAMPLE);

        assert_eq!(
            disks.iter().map(|disk| disk.medium).collect::<Vec<_>>(),
            [Medium::Ssd, Medium::Hdd, Medium::Nvme]
        );
        assert!(disks[0].label().contains("SSD") && disks[1].label().contains("HDD"));
        assert_eq!(Medium::detect(None, None), Medium::Unknown);
    }

    #[test]
    fn broken_lines_are_skipped() {
        assert!(parse("").is_empty());
        assert!(parse("garbage\nNAME=\"/dev/sda\" SIZE=\"x\" TYPE=\"disk\" RO=\"0\"\n").is_empty());
    }

    #[test]
    fn sizes_are_human_readable_and_the_label_has_the_model() {
        assert_eq!(human(512), "512 Б");
        assert_eq!(human(1536), "1.5 КиБ");
        assert_eq!(human(512_110_190_592), "476.9 ГиБ");

        let label = parse(SAMPLE)[0].label();

        assert!(
            label.contains("/dev/sda")
                && label.contains("476.9 ГиБ")
                && label.contains("Samsung SSD 870"),
            "{label}"
        );
        assert!(
            parse(SAMPLE)[1].label().contains("без названия")
                && parse(SAMPLE)[1].label().contains("съёмный")
        );
    }

    #[test]
    fn hex_escapes_become_characters() {
        assert_eq!(unescape(r"a\x20b\x22c"), "a b\"c");
        assert_eq!(unescape(r"\xZZ"), r"\xZZ");
        assert_eq!(unescape(r"\xd0\xb4\xd0\xb8\xd1\x81\xd0\xba"), "диск");
    }

    #[test]
    fn only_block_devices_may_be_erased() {
        for bad in ["/dev/null", "/dev/zero", "/etc", "/nonexistent/disk"] {
            assert!(check_blocks(&[bad.to_string()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_same_disk_under_two_names_is_found() {
        let paths: Vec<PathBuf> = ["/dev/sda", "/dev/sdb", "/dev/sda"]
            .iter()
            .map(Into::into)
            .collect();

        assert_eq!(first_duplicate(&paths), Some(&"/dev/sda".into()));
        assert_eq!(first_duplicate(&paths[..2]), None);
    }
}
