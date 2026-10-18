use std::fmt;

use crate::config::Firmware;
use crate::config::Unlock;
use crate::config::check_disk;
use crate::config::is_safe_path;
use crate::error::Error;
use crate::i18n::tr;
use crate::i18n::tt;

const MEBIBYTE: u64 = 1024 * 1024;
const MAX_SIZE: u64 = 1024 * 1024 * 1024;
const MAX_SPEC: usize = 512;
const MAX_NAME: usize = 32;
const MAX_ITEMS: usize = 128;
/// Каталоги, которые нельзя выносить на отдельную файловую систему: это виртуальные файловые системы, ссылки на `/usr` или каталог с самим `fstab`. Сам `/usr` вынести можно: его монтирует initramfs.
const RESERVED_MOUNTS: [&str; 9] = [
    "/dev", "/proc", "/sys", "/run", "/etc", "/bin", "/sbin", "/lib", "/lib64",
];

pub const BOOT_SIZE: u64 = 512;
const BIOS_BOOT_SIZE: u64 = 4;
pub const BOOT_MIN: u64 = 256;
pub const ROOT_MIN: u64 = 512;
const TABLE_RESERVE: u64 = 2;
const METADATA_RESERVE: u64 = 4;
pub const UBOOT_GAP: u64 = 16;

/// Служебная часть LUKS2 в начале устройства: на неё меньше места, если заголовок не вынесен.
const LUKS_RESERVE: u64 = 16;
/// Имя раскрытого корня в `/dev/mapper`; его знают `crypttab` и initramfs.
pub const ROOT_MAPPER: &str = "cryptroot";
const MAX_CRYPT_NAME: usize = 48;

pub const DEFAULT_GROUP: &str = "moonshine";
pub const DEFAULT_RAID: &str = "md0";
pub const ROOT_VOLUME: &str = "root";

fn invalid(text: impl Into<String>) -> Error {
    Error::Invalid(text.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fs {
    Ext4,
    Xfs,
    Btrfs,
}

impl Fs {
    const ALL: [Fs; 3] = [Fs::Ext4, Fs::Xfs, Fs::Btrfs];

    /// Следующая файловая система по кругу: так её меняют в редакторе одной клавишей.
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|fs| *fs == self).unwrap_or(0);

        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Fs::Ext4 => "ext4",
            Fs::Xfs => "xfs",
            Fs::Btrfs => "btrfs",
        }
    }

    fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "ext4" => Ok(Fs::Ext4),
            "xfs" => Ok(Fs::Xfs),
            "btrfs" => Ok(Fs::Btrfs),
            other => Err(invalid(tr!(
                "unknown file system: {other} (available: ext4, xfs and btrfs)",
                "неизвестная файловая система: {other} (есть ext4, xfs и btrfs)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Raid0,
    Raid1,
    Raid5,
    Raid6,
    Raid10,
}

impl Level {
    pub fn number(self) -> &'static str {
        match self {
            Level::Raid0 => "0",
            Level::Raid1 => "1",
            Level::Raid5 => "5",
            Level::Raid6 => "6",
            Level::Raid10 => "10",
        }
    }

    fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "0" => Ok(Level::Raid0),
            "1" => Ok(Level::Raid1),
            "5" => Ok(Level::Raid5),
            "6" => Ok(Level::Raid6),
            "10" => Ok(Level::Raid10),
            other => Err(invalid(tr!(
                "unknown RAID level: {other} (available: 0, 1, 5, 6, 10)",
                "неизвестный уровень RAID: {other} (есть 0, 1, 5, 6, 10)"
            ))),
        }
    }

    pub fn minimum(self) -> usize {
        match self {
            Level::Raid0 | Level::Raid1 => 2,
            Level::Raid5 => 3,
            Level::Raid6 | Level::Raid10 => 4,
        }
    }

    /// Полезный объём массива из участников заданных размеров.
    fn capacity(self, members: &[u64]) -> u64 {
        let smallest = members.iter().copied().min().unwrap_or(0);
        let count = members.len() as u64;

        match self {
            Level::Raid0 => members.iter().sum(),
            Level::Raid1 => smallest,
            Level::Raid5 => smallest * count.saturating_sub(1),
            Level::Raid6 => smallest * count.saturating_sub(2),
            Level::Raid10 => smallest * count / 2,
        }
    }
}

/// Размер в мебибайтах, «всё оставшееся» или «весь диск без таблицы разделов».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Mib(u64),
    Rest,
    Whole,
}

impl Size {
    fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "rest" => return Ok(Size::Rest),
            "whole" => return Ok(Size::Whole),
            _ => {}
        }

        let digits = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        let (number, unit) = text.split_at(digits);
        let factor = match unit {
            "MiB" => 1,
            "GiB" => 1024,
            "TiB" => 1024 * 1024,
            _ => {
                return Err(invalid(tr!(
                    "size: a number with MiB, GiB or TiB, or rest, or whole is required: {text}",
                    "размер: нужно число с MiB, GiB или TiB, либо rest, либо whole: {text}"
                )));
            }
        };
        let mib = number
            .parse::<u64>()
            .ok()
            .and_then(|number| number.checked_mul(factor))
            .filter(|mib| (1..=MAX_SIZE).contains(mib))
            .ok_or_else(|| invalid(tr!("invalid size: {text}", "неверный размер: {text}")))?;

        Ok(Size::Mib(mib))
    }
}

impl fmt::Display for Size {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Size::Rest => "rest".to_string(),
            Size::Whole => "whole".to_string(),
            Size::Mib(mib) if mib % (1024 * 1024) == 0 => format!("{}TiB", mib >> 20),
            Size::Mib(mib) if mib % 1024 == 0 => format!("{}GiB", mib >> 10),
            Size::Mib(mib) => format!("{mib}MiB"),
        };

        formatter.pad(&text)
    }
}

const MAX_TUNING_ITEMS: usize = 16;
const MAX_TUNING_LEN: usize = 64;
/// Ключи `mkfs` задаёт установщик сам: UUID и принудительное создание.
const FIXED_MKFS_FLAGS: [&str; 5] = ["-U", "-F", "-f", "--uuid", "--force"];

/// Настройки файловой системы: дополнительные аргументы `mkfs` и параметры монтирования для `fstab`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tuning {
    pub mkfs: Vec<String>,
    pub options: Vec<String>,
}

impl Tuning {
    /// В строке разметки запятая внутри значения пишется как «;».
    fn parse(mkfs: Option<&str>, options: Option<&str>) -> Result<Self, Error> {
        let mkfs: Vec<String> = mkfs
            .map(|text| text.replace(';', ","))
            .iter()
            .flat_map(|text| {
                text.split_whitespace()
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        let options: Vec<String> = options
            .iter()
            .flat_map(|text| text.split([';', ',']).map(String::from).collect::<Vec<_>>())
            .collect();

        let bad_mkfs = |token: &String| {
            token.len() > MAX_TUNING_LEN
                || token.starts_with('/')
                || FIXED_MKFS_FLAGS.contains(&token.as_str())
                || !token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_=,.+:@%".contains(c))
        };
        let bad_option = |option: &String| {
            option.is_empty()
                || option.len() > MAX_TUNING_LEN
                || !option
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_=.:@/+-".contains(c))
        };

        if mkfs.len() > MAX_TUNING_ITEMS || mkfs.iter().any(bad_mkfs) {
            return Err(invalid(tr!(
                "mkfs: up to {MAX_TUNING_ITEMS} arguments of letters, digits and \"-_=,.+:@%\", without {} and paths",
                "mkfs: до {MAX_TUNING_ITEMS} аргументов из букв, цифр и «-_=,.+:@%», без {} и путей",
                FIXED_MKFS_FLAGS.join(" ")
            )));
        }

        if options.len() > MAX_TUNING_ITEMS || options.iter().any(bad_option) {
            return Err(invalid(tr!(
                "options: up to {MAX_TUNING_ITEMS} mount options separated by \";\" of letters, digits and \"_=.:@/+-\"",
                "options: до {MAX_TUNING_ITEMS} параметров монтирования через «;» из букв, цифр и «_=.:@/+-»"
            )));
        }

        Ok(Self { mkfs, options })
    }

    pub fn is_default(&self) -> bool {
        self.mkfs.is_empty() && self.options.is_empty()
    }

    /// Параметры монтирования для четвёртого поля `fstab`.
    pub fn fstab_options(&self) -> String {
        if self.options.is_empty() {
            "defaults".into()
        } else {
            self.options.join(",")
        }
    }

    fn suffix(&self) -> String {
        let mut text = String::new();

        if !self.mkfs.is_empty() {
            text.push_str(&format!(",mkfs={}", self.mkfs.join(" ").replace(',', ";")));
        }

        if !self.options.is_empty() {
            text.push_str(&format!(",options={}", self.options.join(";")));
        }

        text
    }
}

/// Что лежит на разделе, массиве или томе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Use {
    BiosBoot,
    Boot,
    /// Пустой раздел размером с загрузочные на диске загрузки: члены массива получаются равными.
    Reserve,
    Swap,
    Pool,
    Lvm(String),
    Raid(String),
    Mount {
        path: String,
        fs: Fs,
        tuning: Tuning,
    },
}

impl Use {
    fn parse(text: &str, fs: Option<Fs>, tuning: Tuning) -> Result<Self, Error> {
        let tuning_is_default = tuning.is_default();
        let usage = match text {
            "biosboot" => Use::BiosBoot,
            "boot" => Use::Boot,
            "reserve" => Use::Reserve,
            "swap" => Use::Swap,
            "pool" => Use::Pool,
            _ => {
                if let Some(name) = text.strip_prefix("lvm:") {
                    Use::Lvm(name.into())
                } else if let Some(name) = text.strip_prefix("raid:") {
                    Use::Raid(name.into())
                } else if text.starts_with('/') {
                    Use::Mount {
                        path: text.into(),
                        fs: fs.unwrap_or(Fs::Ext4),
                        tuning,
                    }
                } else {
                    return Err(invalid(tr!(
                        "point: biosboot, boot, reserve, swap, pool, lvm:GROUP, raid:ARRAY or a mount point: {text}",
                        "point: biosboot, boot, reserve, swap, pool, lvm:ГРУППА, raid:МАССИВ или точка монтирования: {text}"
                    )));
                }
            }
        };

        if (fs.is_some() || !tuning_is_default) && !matches!(usage, Use::Mount { .. }) {
            return Err(invalid(tt!(
                "fs, mkfs and options are given only for a mount point",
                "fs, mkfs и options указывают только у точки монтирования"
            )));
        }

        Ok(usage)
    }

    pub fn mount_path(&self) -> Option<&str> {
        match self {
            Use::Mount { path, .. } => Some(path),
            Use::Boot => Some("/boot"),
            _ => None,
        }
    }
}

impl fmt::Display for Use {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Use::BiosBoot => write!(formatter, "biosboot"),
            Use::Boot => write!(formatter, "boot"),
            Use::Reserve => write!(formatter, "reserve"),
            Use::Swap => write!(formatter, "swap"),
            Use::Pool => write!(formatter, "pool"),
            Use::Lvm(name) => write!(formatter, "lvm:{name}"),
            Use::Raid(name) => write!(formatter, "raid:{name}"),
            Use::Mount { path, .. } => write!(formatter, "{path}"),
        }
    }
}

fn pool_note(volume: &Volume) -> String {
    volume.pool.as_ref().map_or_else(String::new, |pool| {
        tr!("  [thin in {pool}]", "  [тонкий в {pool}]")
    })
}

/// Файловая система, если она не ext4 по умолчанию.
fn fs_note(usage: &Use) -> String {
    match usage {
        Use::Mount { fs, .. } if *fs != Fs::Ext4 => format!(" ({})", fs.name()),
        _ => String::new(),
    }
}

pub fn crypt_note(crypt: Option<Crypt>) -> String {
    crypt
        .map(|crypt| format!("  [{}]", crypt.label()))
        .unwrap_or_default()
}

/// Ключи точки монтирования после `point=`: `fs`, `mkfs`, `options`.
fn mount_suffix(usage: &Use) -> String {
    match usage {
        Use::Mount { fs, tuning, .. } => {
            let fs = match fs {
                Fs::Ext4 => String::new(),
                other => format!(",fs={}", other.name()),
            };

            format!("{fs}{}", tuning.suffix())
        }
        _ => String::new(),
    }
}

/// Шифрование LUKS2 на устройстве: как оно открывается и где лежит заголовок.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crypt {
    /// `Typed` — парольная фраза, `Keyfile` — случайный ключ-файл в `/etc/luks`.
    pub unlock: Unlock,
    /// Заголовок лежит отдельным файлом в `/etc/luks`, а не на самом устройстве.
    pub detached: bool,
}

impl Crypt {
    pub fn label(self) -> String {
        let unlock = match self.unlock {
            Unlock::Typed => tt!("password", "пароль"),
            Unlock::Keyfile => tt!("key file", "ключ-файл"),
        };
        let header = if self.detached {
            tt!(", detached header", ", заголовок отдельно")
        } else {
            ""
        };

        format!("LUKS: {unlock}{header}")
    }

    fn suffix(crypt: Option<Crypt>) -> String {
        match crypt {
            None => String::new(),
            Some(crypt) => format!(
                ",crypt={}{}",
                match crypt.unlock {
                    Unlock::Typed => "pass",
                    Unlock::Keyfile => "key",
                },
                if crypt.detached {
                    ",header=detached"
                } else {
                    ""
                }
            ),
        }
    }

    fn overhead(crypt: Option<Crypt>) -> u64 {
        match crypt {
            Some(crypt) if !crypt.detached => LUKS_RESERVE,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub disk: String,
    pub size: Size,
    pub usage: Use,
    pub crypt: Option<Crypt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raid {
    pub name: String,
    pub level: Level,
    pub usage: Use,
    pub crypt: Option<Crypt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub group: String,
    pub name: String,
    pub size: Size,
    pub usage: Use,
    pub crypt: Option<Crypt>,
    /// Пул, в котором лежит тонкий том; у обычных томов и самих пулов его нет.
    pub pool: Option<String>,
}

struct Pairs<'a>(Vec<(&'a str, &'a str)>);

impl<'a> Pairs<'a> {
    fn parse(text: &'a str, allowed: &[&str]) -> Result<Self, Error> {
        if text.len() > MAX_SPEC {
            return Err(invalid(tt!(
                "the description is too long",
                "описание слишком длинное"
            )));
        }

        let mut pairs: Vec<(&str, &str)> = Vec::new();

        for item in text.split(',') {
            let (key, value) = item
                .split_once('=')
                .filter(|(_, value)| !value.is_empty())
                .ok_or_else(|| {
                    invalid(tr!(
                        "key=value expected: {item}",
                        "нужен вид ключ=значение: {item}"
                    ))
                })?;
            if !allowed.contains(&key) {
                return Err(invalid(tr!(
                    "unknown key {key} (available: {})",
                    "неизвестный ключ {key} (есть {})",
                    allowed.join(", ")
                )));
            }

            if pairs.iter().any(|(known, _)| *known == key) {
                return Err(invalid(tr!(
                    "key {key} is given twice",
                    "ключ {key} указан дважды"
                )));
            }

            pairs.push((key, value));
        }

        Ok(Self(pairs))
    }

    fn get(&self, key: &str) -> Option<&'a str> {
        self.0
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, value)| *value)
    }

    fn need(&self, key: &str) -> Result<&'a str, Error> {
        self.get(key)
            .ok_or_else(|| invalid(tr!("key {key} is missing", "не указан ключ {key}")))
    }

    fn usage(&self) -> Result<Use, Error> {
        let fs = self.get("fs").map(Fs::parse).transpose()?;
        let tuning = Tuning::parse(self.get("mkfs"), self.get("options"))?;

        Use::parse(self.need("point")?, fs, tuning)
    }

    fn crypt(&self) -> Result<Option<Crypt>, Error> {
        let unlock = match self.get("crypt") {
            None if self.get("header").is_some() => {
                return Err(invalid(tt!(
                    "header is given only together with crypt",
                    "header указывают только вместе с crypt"
                )));
            }
            None => return Ok(None),
            Some("pass") => Unlock::Typed,
            Some("key") => Unlock::Keyfile,
            Some(other) => {
                return Err(invalid(tr!(
                    "crypt: pass or key, not {other}",
                    "crypt: pass или key, а не {other}"
                )));
            }
        };
        let detached = match self.get("header") {
            None | Some("attached") => false,
            Some("detached") => true,
            Some(other) => {
                return Err(invalid(tr!(
                    "header: attached or detached, not {other}",
                    "header: attached или detached, а не {other}"
                )));
            }
        };

        Ok(Some(Crypt { unlock, detached }))
    }
}

impl Partition {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let pairs = Pairs::parse(
            text,
            &[
                "disk", "size", "point", "fs", "mkfs", "options", "crypt", "header",
            ],
        )?;

        Ok(Self {
            disk: pairs.need("disk")?.into(),
            size: Size::parse(pairs.need("size")?)?,
            usage: pairs.usage()?,
            crypt: pairs.crypt()?,
        })
    }
}

impl fmt::Display for Partition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "disk={},size={},point={}{}{}",
            self.disk,
            self.size,
            self.usage,
            mount_suffix(&self.usage),
            Crypt::suffix(self.crypt)
        )
    }
}

impl Raid {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let pairs = Pairs::parse(
            text,
            &[
                "name", "level", "point", "fs", "mkfs", "options", "crypt", "header",
            ],
        )?;

        Ok(Self {
            name: pairs.need("name")?.into(),
            level: Level::parse(pairs.need("level")?)?,
            usage: pairs.usage()?,
            crypt: pairs.crypt()?,
        })
    }
}

impl fmt::Display for Raid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "name={},level={},point={}{}{}",
            self.name,
            self.level.number(),
            self.usage,
            mount_suffix(&self.usage),
            Crypt::suffix(self.crypt)
        )
    }
}

impl Volume {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let pairs = Pairs::parse(
            text,
            &[
                "vg", "name", "size", "point", "fs", "mkfs", "options", "crypt", "header", "pool",
            ],
        )?;
        let volume = Self {
            group: pairs.need("vg")?.into(),
            name: pairs.need("name")?.into(),
            size: Size::parse(pairs.need("size")?)?,
            usage: pairs.usage()?,
            crypt: pairs.crypt()?,
            pool: pairs.get("pool").map(Into::into),
        };

        volume.check_thin()?;
        Ok(volume)
    }
}

impl Volume {
    fn check_thin(&self) -> Result<(), Error> {
        let name = &self.name;

        if self.usage == Use::Pool && (self.crypt.is_some() || self.pool.is_some()) {
            return Err(invalid(tr!(
                "pool {name}: the thin volumes of a pool are encrypted, not the pool itself; a pool does not live inside another pool",
                "пул {name}: шифруют тонкие тома пула, а не сам пул; пул не лежит в другом пуле"
            )));
        }

        if self.pool.is_some() && self.usage == Use::Swap {
            return Err(invalid(tr!(
                "volume {name}: swap cannot be thin",
                "том {name}: swap не бывает тонким"
            )));
        }

        if self.pool.is_some() && self.size == Size::Rest {
            return Err(invalid(tr!(
                "thin volume {name}: an exact size is required, rest does not fit",
                "тонкий том {name}: нужен точный размер, rest не подходит"
            )));
        }

        Ok(())
    }

    pub fn is_pool(&self) -> bool {
        self.usage == Use::Pool
    }
}

impl fmt::Display for Volume {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "vg={},name={},size={},point={}{}{}",
            self.group,
            self.name,
            self.size,
            self.usage,
            mount_suffix(&self.usage),
            Crypt::suffix(self.crypt)
        )?;

        match &self.pool {
            Some(pool) => write!(formatter, ",pool={pool}"),
            None => Ok(()),
        }
    }
}

/// Устройство, на которое что-то кладётся: раздел, массив или логический том.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub path: String,
    pub usage: Use,
    pub crypt: Option<Crypt>,
}

/// Разметка хранилища: разделы → RAID → LVM → файловые системы. `/boot` (он же ESP) и biosboot лежат только на разделах.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub partitions: Vec<Partition>,
    pub raids: Vec<Raid>,
    pub volumes: Vec<Volume>,
}

pub fn partition_path(disk: &str, number: usize) -> String {
    if disk.starts_with("/dev/disk/by-") {
        return format!("{disk}-part{number}");
    }

    let separator = if disk.ends_with(|c: char| c.is_ascii_digit()) {
        "p"
    } else {
        ""
    };

    format!("{disk}{separator}{number}")
}

pub fn raid_path(name: &str) -> String {
    format!("/dev/md/{name}")
}

pub fn volume_path(group: &str, name: &str) -> String {
    format!("/dev/{group}/{name}")
}

fn check_name(name: &str, what: &str) -> Result<(), Error> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.+-".contains(c));

    if ok {
        Ok(())
    } else {
        Err(invalid(tr!(
            "{what}: up to {MAX_NAME} characters of letters, digits and \"_.+-\", not starting with \"-\" or \".\": {name}",
            "{what}: до {MAX_NAME} символов из букв, цифр и «_.+-», не с «-» и не с «.»: {name}"
        )))
    }
}

fn check_mount_path(path: &str) -> Result<(), Error> {
    if path == "/boot" || path == "/boot/efi" {
        return Err(invalid(tr!(
            "{path} is set through point=boot",
            "{path} задаётся через point=boot"
        )));
    }

    if RESERVED_MOUNTS
        .iter()
        .any(|reserved| path == *reserved || path.starts_with(&format!("{reserved}/")))
    {
        return Err(invalid(tr!(
            "{path} cannot be moved to a separate file system",
            "{path} нельзя выносить на отдельную файловую систему"
        )));
    }

    let ok = is_safe_path(path) && (path == "/" || (!path.ends_with('/') && !path.contains("//")));

    if ok {
        Ok(())
    } else {
        Err(invalid(tr!(
            "invalid mount point: {path}",
            "неверная точка монтирования: {path}"
        )))
    }
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|known| known == value) {
        list.push(value.into());
    }
}

impl Layout {
    pub fn simple(disk: &str, boot_disk: &str, firmware: Firmware) -> Self {
        let mut partitions = boot_partitions(boot_disk, firmware);
        let root = Use::Mount {
            path: "/".into(),
            fs: Fs::Ext4,
            tuning: Tuning::default(),
        };

        if disk == boot_disk {
            partitions.push(Partition {
                disk: disk.into(),
                size: Size::Rest,
                usage: root,
                crypt: None,
            });
        } else {
            partitions.push(Partition {
                disk: disk.into(),
                size: Size::Whole,
                usage: root,
                crypt: None,
            });
        }

        Self {
            partitions,
            ..Self::default()
        }
    }

    /// Меняет размер `/boot` (в MiB) у готовой разметки.
    pub fn with_boot_size(mut self, boot: u64) -> Self {
        let bios = if self
            .partitions
            .iter()
            .any(|part| part.usage == Use::BiosBoot)
        {
            BIOS_BOOT_SIZE
        } else {
            0
        };

        for part in &mut self.partitions {
            match part.usage {
                Use::Boot => part.size = Size::Mib(boot),
                Use::Reserve => part.size = Size::Mib(bios + boot),
                _ => {}
            }
        }

        self
    }

    /// Диски данных объединяются в одну группу LVM, при `raid` — через массив; загрузка лежит на `boot_disk`.
    pub fn pooled(
        disks: &[String],
        boot_disk: &str,
        raid: Option<Level>,
        firmware: Firmware,
    ) -> Self {
        let usage = match raid {
            Some(_) => Use::Raid(DEFAULT_RAID.into()),
            None => Use::Lvm(DEFAULT_GROUP.into()),
        };
        let mut order = vec![boot_disk.to_string()];

        disks
            .iter()
            .filter(|disk| disk.as_str() != boot_disk)
            .for_each(|disk| order.push(disk.clone()));

        let mut partitions = Vec::new();

        for disk in &order {
            if disk == boot_disk {
                partitions.extend(boot_partitions(disk, firmware));
            }

            if raid.is_some() && disk != boot_disk && disks.iter().any(|known| known == boot_disk) {
                partitions.push(Partition {
                    disk: disk.clone(),
                    size: Size::Mib(boot_area(firmware)),
                    usage: Use::Reserve,
                    crypt: None,
                });
            }

            if disks.contains(disk) {
                partitions.push(Partition {
                    disk: disk.clone(),
                    size: Size::Rest,
                    usage: usage.clone(),
                    crypt: None,
                });
            }
        }

        Self {
            partitions,
            raids: raid
                .map(|level| Raid {
                    name: DEFAULT_RAID.into(),
                    level,
                    usage: Use::Lvm(DEFAULT_GROUP.into()),
                    crypt: None,
                })
                .into_iter()
                .collect(),
            volumes: vec![Volume {
                group: DEFAULT_GROUP.into(),
                name: ROOT_VOLUME.into(),
                size: Size::Rest,
                usage: Use::Mount {
                    path: "/".into(),
                    fs: Fs::Ext4,
                    tuning: Tuning::default(),
                },
                crypt: None,
                pool: None,
            }],
        }
    }

    /// Шифрование того, что смонтировано в `/`: так схемы «один диск» и «LVM» получают зашифрованный корень.
    pub fn with_root_crypt(mut self, crypt: Option<Crypt>) -> Self {
        let root = |usage: &Use| usage.mount_path() == Some("/");

        self.partitions
            .iter_mut()
            .filter(|part| root(&part.usage))
            .for_each(|part| part.crypt = crypt);
        self.raids
            .iter_mut()
            .filter(|raid| root(&raid.usage))
            .for_each(|raid| raid.crypt = crypt);
        self.volumes
            .iter_mut()
            .filter(|volume| root(&volume.usage))
            .for_each(|volume| volume.crypt = crypt);
        self
    }

    /// Строки вида `partition disk=…`, `raid name=…`, `volume vg=…`: так разметку правят в мастере.
    pub fn rows(&self) -> Vec<String> {
        let partitions = self
            .partitions
            .iter()
            .map(|part| format!("partition {part}"));
        let raids = self.raids.iter().map(|raid| format!("raid {raid}"));
        let volumes = self.volumes.iter().map(|volume| format!("volume {volume}"));

        partitions.chain(raids).chain(volumes).collect()
    }

    pub fn from_rows(rows: &[String]) -> Result<Self, Error> {
        let mut layout = Self::default();

        for row in rows {
            let (kind, spec) = row.trim().split_once(' ').ok_or_else(|| {
                invalid(tr!(
                    "layout line: partition|raid|volume and a description: {row}",
                    "строка разметки: partition|raid|volume и описание: {row}"
                ))
            })?;

            match kind {
                "partition" => layout.partitions.push(Partition::parse(spec.trim())?),
                "raid" => layout.raids.push(Raid::parse(spec.trim())?),
                "volume" => layout.volumes.push(Volume::parse(spec.trim())?),
                other => {
                    return Err(invalid(tr!(
                        "a layout line starts with partition, raid or volume: {other}",
                        "строка разметки начинается с partition, raid или volume: {other}"
                    )));
                }
            }
        }

        Ok(layout)
    }

    pub fn disks(&self) -> Vec<String> {
        let mut disks = Vec::new();

        self.partitions
            .iter()
            .for_each(|part| push_unique(&mut disks, &part.disk));
        disks
    }

    pub fn groups(&self) -> Vec<String> {
        let mut groups = Vec::new();

        for usage in self
            .partitions
            .iter()
            .map(|part| &part.usage)
            .chain(self.raids.iter().map(|raid| &raid.usage))
        {
            if let Use::Lvm(name) = usage {
                push_unique(&mut groups, name);
            }
        }

        groups
    }

    pub fn partition_device(&self, index: usize) -> String {
        let part = &self.partitions[index];

        if part.size == Size::Whole {
            return part.disk.clone();
        }

        let number = self.partitions[..=index]
            .iter()
            .filter(|other| other.disk == part.disk && other.size != Size::Whole)
            .count();

        partition_path(&part.disk, number)
    }

    pub fn devices(&self) -> Vec<Device> {
        let partitions = self
            .partitions
            .iter()
            .enumerate()
            .map(|(index, part)| Device {
                path: self.partition_device(index),
                usage: part.usage.clone(),
                crypt: part.crypt,
            });
        let raids = self.raids.iter().map(|raid| Device {
            path: raid_path(&raid.name),
            usage: raid.usage.clone(),
            crypt: raid.crypt,
        });
        let volumes = self
            .volumes
            .iter()
            .filter(|volume| !volume.is_pool())
            .map(|volume| Device {
                path: volume_path(&volume.group, &volume.name),
                usage: volume.usage.clone(),
                crypt: volume.crypt,
            });

        partitions.chain(raids).chain(volumes).collect()
    }

    /// Зашифрованные устройства в порядке создания: разделы, массивы, тома.
    pub fn crypts(&self) -> Vec<Device> {
        self.devices()
            .into_iter()
            .filter(|device| device.crypt.is_some())
            .collect()
    }

    pub fn has_crypt(&self) -> bool {
        self.partitions.iter().any(|part| part.crypt.is_some())
            || self.raids.iter().any(|raid| raid.crypt.is_some())
            || self.volumes.iter().any(|volume| volume.crypt.is_some())
    }

    /// Есть ли устройство, которое открывается общей парольной фразой.
    pub fn needs_passphrase(&self) -> bool {
        self.crypts().iter().any(|device| {
            matches!(
                device.crypt,
                Some(Crypt {
                    unlock: Unlock::Typed,
                    ..
                })
            )
        })
    }

    pub fn crypt_of(&self, path: &str) -> Option<Crypt> {
        self.devices()
            .into_iter()
            .find(|device| device.path == path)
            .and_then(|device| device.crypt)
    }

    /// Короткое имя зашифрованного устройства: по нему называются файлы в `/etc/luks`.
    pub fn crypt_name(&self, path: &str) -> String {
        if path == self.root_device() {
            return "root".into();
        }

        let name = path.strip_prefix("/dev/").unwrap_or(path);
        let name = if name.starts_with("disk/") {
            name.rsplit('/').next().unwrap_or(name)
        } else {
            name
        };

        name.chars()
            .map(|symbol| match symbol {
                '/' => '-',
                symbol if symbol.is_ascii_alphanumeric() || "_.+-".contains(symbol) => symbol,
                _ => '_',
            })
            .take(MAX_CRYPT_NAME)
            .collect()
    }

    /// Имя раскрытого устройства в `/dev/mapper`.
    pub fn mapper(&self, path: &str) -> String {
        if path == self.root_device() {
            ROOT_MAPPER.into()
        } else {
            format!("crypt-{}", self.crypt_name(path))
        }
    }

    /// Устройство, на котором лежит то, что выше по стеку: раскрытое LUKS или само устройство.
    pub fn inner(&self, path: &str) -> String {
        match self.crypt_of(path) {
            Some(_) => format!("/dev/mapper/{}", self.mapper(path)),
            None => path.into(),
        }
    }

    /// Устройство и всё, на чём оно стоит: участники массива, физические тома группы.
    pub fn under(&self, path: &str) -> Vec<String> {
        let mut found = Vec::new();

        self.collect_under(path, &mut found);
        found
    }

    fn collect_under(&self, path: &str, found: &mut Vec<String>) {
        if found.iter().any(|known| known == path) {
            return;
        }

        found.push(path.into());

        if let Some(raid) = self.raids.iter().find(|raid| raid_path(&raid.name) == path) {
            for member in self.members(&raid.name) {
                self.collect_under(&member, found);
            }
        }

        if let Some(volume) = self
            .volumes
            .iter()
            .find(|volume| volume_path(&volume.group, &volume.name) == path)
        {
            for physical in self.volumes_of(&volume.group) {
                self.collect_under(&physical, found);
            }
        }
    }

    /// Зашифрованные устройства, без которых не смонтировать корень и `/usr`: их открывает initramfs.
    pub fn early(&self) -> Vec<String> {
        let mut needed = self.under(&self.root_device());

        for device in self.devices() {
            if device.usage.mount_path() == Some("/usr") {
                needed.extend(self.under(&device.path));
            }
        }

        self.crypts()
            .into_iter()
            .map(|device| device.path)
            .filter(|path| needed.contains(path))
            .collect()
    }

    /// Лежит ли устройство на зашифрованном слое.
    pub fn protected(&self, path: &str) -> bool {
        self.under(path)
            .iter()
            .any(|known| self.crypt_of(known).is_some())
    }

    /// Устройства, на которых создаётся файловая система или раздел подкачки.
    pub fn filesystems(&self) -> Vec<Device> {
        self.devices()
            .into_iter()
            .filter(|device| matches!(device.usage, Use::Boot | Use::Swap | Use::Mount { .. }))
            .collect()
    }

    pub fn members(&self, raid: &str) -> Vec<String> {
        self.partitions
            .iter()
            .enumerate()
            .filter(|(_, part)| matches!(&part.usage, Use::Raid(name) if name == raid))
            .map(|(index, _)| self.partition_device(index))
            .collect()
    }

    /// Физические тома группы: разделы и массивы с `lvm:ГРУППА`.
    pub fn volumes_of(&self, group: &str) -> Vec<String> {
        self.devices()
            .into_iter()
            .filter(|device| matches!(&device.usage, Use::Lvm(name) if name == group))
            .map(|device| device.path)
            .collect()
    }

    pub fn boot_disk(&self) -> &str {
        self.partitions
            .iter()
            .find(|part| part.usage == Use::Boot)
            .map_or("", |part| part.disk.as_str())
    }

    pub fn root_device(&self) -> String {
        self.devices()
            .into_iter()
            .find(|device| device.usage.mount_path() == Some("/"))
            .map_or_else(String::new, |device| device.path)
    }

    /// Физические диски под устройством: через тома, массивы и разделы.
    pub fn disks_under(&self, path: &str) -> Vec<String> {
        let mut disks = Vec::new();

        self.collect_disks(path, &mut disks);
        disks
    }

    fn collect_disks(&self, path: &str, disks: &mut Vec<String>) {
        if let Some(index) = (0..self.partitions.len()).find(|&i| self.partition_device(i) == path)
        {
            push_unique(disks, &self.partitions[index].disk);
        }

        if let Some(raid) = self.raids.iter().find(|raid| raid_path(&raid.name) == path) {
            for member in self.members(&raid.name) {
                self.collect_disks(&member, disks);
            }
        }

        if let Some(volume) = self
            .volumes
            .iter()
            .find(|volume| volume_path(&volume.group, &volume.name) == path)
        {
            for physical in self.volumes_of(&volume.group) {
                self.collect_disks(&physical, disks);
            }
        }
    }

    /// Диск загрузки не входит в число дисков корня: без него система не стартует.
    pub fn separate_boot(&self) -> bool {
        !self
            .disks_under(&self.root_device())
            .iter()
            .any(|disk| disk == self.boot_disk())
    }

    pub fn needs_raid(&self) -> bool {
        !self.raids.is_empty()
    }

    pub fn needs_lvm(&self) -> bool {
        !self.groups().is_empty()
    }

    pub fn needs_thin(&self) -> bool {
        self.volumes.iter().any(Volume::is_pool)
    }

    pub fn uses_fs(&self, wanted: Fs) -> bool {
        self.devices()
            .iter()
            .any(|device| matches!(device.usage, Use::Mount { fs, .. } if fs == wanted))
    }

    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();

        for (index, part) in self.partitions.iter().enumerate() {
            lines.push(format!(
                "{:<20} {:>8}  {}{}{}",
                self.partition_device(index),
                part.size,
                part.usage,
                fs_note(&part.usage),
                crypt_note(part.crypt)
            ));
        }

        for raid in &self.raids {
            lines.push(format!(
                "{:<20} RAID{:<3}  {}{}{}  ← {}",
                raid_path(&raid.name),
                raid.level.number(),
                raid.usage,
                fs_note(&raid.usage),
                crypt_note(raid.crypt),
                self.members(&raid.name).join(" ")
            ));
        }

        for volume in &self.volumes {
            lines.push(format!(
                "{:<20} {:>8}  {}{}{}{}",
                volume_path(&volume.group, &volume.name),
                volume.size,
                volume.usage,
                fs_note(&volume.usage),
                pool_note(volume),
                crypt_note(volume.crypt)
            ));
        }

        lines
    }

    pub fn validate(&self, firmware: Firmware) -> Result<(), Error> {
        if self.partitions.is_empty() {
            return Err(invalid(tt!(
                "the layout has no partitions",
                "в разметке нет ни одного раздела"
            )));
        }

        if self.partitions.len() > MAX_ITEMS
            || self.raids.len() > MAX_ITEMS
            || self.volumes.len() > MAX_ITEMS
        {
            return Err(invalid(tr!(
                "the layout has more than {MAX_ITEMS} items",
                "в разметке больше {MAX_ITEMS} элементов"
            )));
        }

        self.check_names()?;
        self.check_minimums()?;
        self.check_tables()?;
        self.check_carriers()?;
        self.check_mounts()?;
        self.check_boot(firmware)?;
        self.check_raids()?;
        self.check_groups()?;
        self.check_crypt()
    }

    fn check_crypt(&self) -> Result<(), Error> {
        let root_protected = self.protected(&self.root_device());
        let early = self.early();
        let mut names: Vec<String> = Vec::new();

        for device in self.crypts() {
            let Some(crypt) = device.crypt else {
                continue;
            };

            if matches!(device.usage, Use::Boot | Use::BiosBoot | Use::Reserve) {
                return Err(invalid(tr!(
                    "{}: /boot and biosboot are not encrypted: the bootloader reads them",
                    "{}: /boot и biosboot не шифруются: их читает загрузчик",
                    device.path
                )));
            }

            let name = self.crypt_name(&device.path);

            if names.contains(&name) {
                return Err(invalid(tr!(
                    "{}: the name {name} is already used by another encrypted device",
                    "{}: имя {name} уже занято другим зашифрованным устройством",
                    device.path
                )));
            }

            names.push(name);

            if early.contains(&device.path) {
                if crypt.unlock == Unlock::Keyfile && !crypt.detached {
                    return Err(invalid(tr!(
                        "{}: a key file in the initramfs needs a detached header (header=detached)",
                        "{}: ключ-файл в initramfs требует отдельного заголовка (header=detached)",
                        device.path
                    )));
                }

                if self.separate_boot() && !crypt.detached {
                    return Err(invalid(tr!(
                        "{}: with a separate boot disk the LUKS header must be detached (header=detached)",
                        "{}: при отдельном диске загрузки заголовок LUKS должен быть отдельным (header=detached)",
                        device.path
                    )));
                }
            } else if crypt.unlock == Unlock::Keyfile && !root_protected {
                return Err(invalid(tr!(
                    "{}: the key file lives in /etc and the root is not encrypted — the key would be exposed",
                    "{}: ключ-файл лежит в /etc, а корень не зашифрован — ключ был бы открыт",
                    device.path
                )));
            }
        }

        Ok(())
    }

    fn check_names(&self) -> Result<(), Error> {
        for part in &self.partitions {
            check_disk(&part.disk)?;
            Self::check_usage(&part.usage)?;
        }

        for raid in &self.raids {
            check_name(&raid.name, tt!("array name", "имя массива"))?;
            Self::check_usage(&raid.usage)?;
        }

        for volume in &self.volumes {
            check_name(&volume.group, tt!("group name", "имя группы"))?;
            check_name(&volume.name, tt!("volume name", "имя тома"))?;
            Self::check_usage(&volume.usage)?;
        }

        Ok(())
    }

    fn check_usage(usage: &Use) -> Result<(), Error> {
        match usage {
            Use::Lvm(name) => check_name(name, tt!("group name", "имя группы")),
            Use::Raid(name) => check_name(name, tt!("array name", "имя массива")),
            Use::Mount { path, .. } => check_mount_path(path),
            _ => Ok(()),
        }
    }

    fn check_minimums(&self) -> Result<(), Error> {
        for part in &self.partitions {
            if part.usage == Use::Boot && matches!(part.size, Size::Mib(mib) if mib < BOOT_MIN) {
                return Err(invalid(tr!(
                    "boot: size {} is below the allowed {BOOT_MIN}MiB",
                    "boot: размер {} меньше допустимых {BOOT_MIN}MiB",
                    part.size
                )));
            }
        }

        Ok(())
    }

    fn check_tables(&self) -> Result<(), Error> {
        for disk in self.disks() {
            let sizes: Vec<Size> = self
                .partitions
                .iter()
                .filter(|part| part.disk == disk)
                .map(|part| part.size)
                .collect();

            if sizes.contains(&Size::Whole) && sizes.len() > 1 {
                return Err(invalid(tr!(
                    "{disk}: whole takes the entire disk, no other partitions can be on it",
                    "{disk}: whole занимает диск целиком, других разделов на нём быть не может"
                )));
            }

            let rests = sizes.iter().filter(|size| **size == Size::Rest).count();

            if rests > 1 || (rests == 1 && sizes.last() != Some(&Size::Rest)) {
                return Err(invalid(tr!(
                    "{disk}: rest can be used once and only by the last partition",
                    "{disk}: rest может быть один и только у последнего раздела"
                )));
            }
        }

        Ok(())
    }

    fn check_carriers(&self) -> Result<(), Error> {
        for part in &self.partitions {
            if part.usage == Use::Pool {
                return Err(invalid(tr!(
                    "{}: point=pool exists only for an LVM volume",
                    "{}: point=pool бывает только у тома LVM",
                    part.disk
                )));
            }

            let fixed = matches!(part.usage, Use::BiosBoot | Use::Boot | Use::Reserve);

            if fixed && part.size == Size::Whole {
                return Err(invalid(tr!(
                    "{}: {} cannot take a disk without a partition table",
                    "{}: {} не может занимать диск без таблицы разделов",
                    part.disk,
                    part.usage
                )));
            }
        }

        for raid in &self.raids {
            if matches!(
                raid.usage,
                Use::BiosBoot | Use::Boot | Use::Reserve | Use::Raid(_) | Use::Pool
            ) {
                return Err(invalid(tr!(
                    "array {}: point={} is not allowed",
                    "массив {}: point={} недопустим",
                    raid.name,
                    raid.usage
                )));
            }
        }

        for volume in &self.volumes {
            let wrong_usage = matches!(
                volume.usage,
                Use::BiosBoot | Use::Boot | Use::Reserve | Use::Raid(_) | Use::Lvm(_)
            );

            if wrong_usage || volume.size == Size::Whole {
                return Err(invalid(tr!(
                    "volume {}/{}: a size and either swap or a mount point are required",
                    "том {}/{}: нужен размер и либо swap, либо точка монтирования",
                    volume.group,
                    volume.name
                )));
            }
        }

        Ok(())
    }

    fn check_mounts(&self) -> Result<(), Error> {
        let mut paths: Vec<String> = Vec::new();

        for device in self.devices() {
            if let Some(path) = device.usage.mount_path() {
                if paths.iter().any(|known| known == path) {
                    return Err(invalid(tr!(
                        "mount point {path} is given twice",
                        "точка монтирования {path} указана дважды"
                    )));
                }

                paths.push(path.into());
            }
        }

        if !paths.iter().any(|path| path == "/") {
            return Err(invalid(tt!(
                "no root is given: point=/",
                "не указан корень: point=/"
            )));
        }

        Ok(())
    }

    fn check_boot(&self, firmware: Firmware) -> Result<(), Error> {
        let count = |usage: Use| {
            self.partitions
                .iter()
                .filter(|part| part.usage == usage)
                .count()
        };

        if count(Use::Boot) != 1 {
            return Err(invalid(tt!(
                "exactly one point=boot partition is required: /boot lives on a plain partition, not on LVM or RAID",
                "нужен ровно один раздел point=boot: /boot лежит на обычном разделе, не на LVM и не на RAID"
            )));
        }

        let boot_disk = self.boot_disk();
        let on_boot_disk = |usage: Use| {
            self.partitions
                .iter()
                .any(|part| part.usage == usage && part.disk == boot_disk)
        };

        let bios = usize::from(firmware.has_bios());

        if count(Use::BiosBoot) != bios || (bios == 1 && !on_boot_disk(Use::BiosBoot)) {
            return Err(invalid(if bios == 1 {
                tt!(
                    "BIOS and hybrid need exactly one point=biosboot partition on the disk with /boot",
                    "для BIOS и гибрида нужен ровно один раздел point=biosboot на диске с /boot"
                )
            } else {
                tt!(
                    "point=biosboot is needed only for BIOS and hybrid",
                    "point=biosboot нужен только для BIOS и гибрида"
                )
            }));
        }

        let first = self.partitions.iter().find(|part| part.disk == boot_disk);

        if firmware == Firmware::Uboot && first.is_none_or(|part| part.usage != Use::Boot) {
            return Err(invalid(tt!(
                "for U-Boot /boot must be the first partition of its disk",
                "для U-Boot /boot должен быть первым разделом своего диска"
            )));
        }

        Ok(())
    }

    fn check_raids(&self) -> Result<(), Error> {
        for (index, raid) in self.raids.iter().enumerate() {
            if self.raids[..index]
                .iter()
                .any(|other| other.name == raid.name)
            {
                return Err(invalid(tr!(
                    "array {} is given twice",
                    "массив {} указан дважды",
                    raid.name
                )));
            }

            let members = self.members(&raid.name);

            if members.len() < raid.level.minimum() {
                return Err(invalid(tr!(
                    "array {} (RAID{}) needs at least {} partitions, {} given",
                    "массиву {} (RAID{}) нужно не меньше {} разделов, указано {}",
                    raid.name,
                    raid.level.number(),
                    raid.level.minimum(),
                    members.len()
                )));
            }

            let disks = self.disks_under(&raid_path(&raid.name));

            if disks.len() != members.len() {
                return Err(invalid(tr!(
                    "members of array {} must be on different disks",
                    "участники массива {} должны лежать на разных дисках",
                    raid.name
                )));
            }
        }

        for part in &self.partitions {
            if let Use::Raid(name) = &part.usage
                && !self.raids.iter().any(|raid| &raid.name == name)
            {
                return Err(invalid(tr!(
                    "array {name} is not described",
                    "нет описания массива {name}"
                )));
            }
        }

        Ok(())
    }

    fn check_groups(&self) -> Result<(), Error> {
        let groups = self.groups();

        for (index, volume) in self.volumes.iter().enumerate() {
            if !groups.contains(&volume.group) {
                return Err(invalid(tr!(
                    "volume {}: group {} has no physical volumes (point=lvm:{})",
                    "том {}: нет физических томов группы {} (point=lvm:{})",
                    volume.name,
                    volume.group,
                    volume.group
                )));
            }

            let same_group = |other: &&Volume| other.group == volume.group;

            if self.volumes[..index]
                .iter()
                .filter(same_group)
                .any(|other| other.name == volume.name)
            {
                return Err(invalid(tr!(
                    "volume {}/{} is given twice",
                    "том {}/{} указан дважды",
                    volume.group,
                    volume.name
                )));
            }

            self.check_pool_of(volume)?;

            if volume.size == Size::Rest
                && self.volumes[..index]
                    .iter()
                    .filter(same_group)
                    .any(|other| other.size == Size::Rest)
            {
                return Err(invalid(tr!(
                    "in group {} only one volume can have the size rest",
                    "в группе {} размер rest может быть только у одного тома",
                    volume.group
                )));
            }
        }

        for group in &groups {
            if !self.volumes.iter().any(|volume| &volume.group == group) {
                return Err(invalid(tr!(
                    "group {group} has no volumes",
                    "в группе {group} нет ни одного тома"
                )));
            }
        }

        for pool in self.volumes.iter().filter(|volume| volume.is_pool()) {
            if !self.thin_of(pool).any(|_| true) {
                return Err(invalid(tr!(
                    "pool {}/{}: it has no thin volumes (pool={})",
                    "пул {}/{}: в нём нет тонких томов (pool={})",
                    pool.group,
                    pool.name,
                    pool.name
                )));
            }
        }

        Ok(())
    }

    fn thin_of<'a>(&'a self, pool: &'a Volume) -> impl Iterator<Item = &'a Volume> {
        self.volumes.iter().filter(move |volume| {
            volume.group == pool.group && volume.pool.as_ref() == Some(&pool.name)
        })
    }

    fn check_pool_of(&self, volume: &Volume) -> Result<(), Error> {
        let Some(name) = &volume.pool else {
            return Ok(());
        };
        let found = self
            .volumes
            .iter()
            .any(|other| other.group == volume.group && other.name == *name && other.is_pool());

        if found {
            Ok(())
        } else {
            Err(invalid(tr!(
                "volume {}/{}: this group has no pool {name} (point=pool)",
                "том {}/{}: нет пула {name} (point=pool) в этой группе",
                volume.group,
                volume.name
            )))
        }
    }

    /// Пулы, в которых тонкие тома обещают больше места, чем есть: пул и сумма виртуальных размеров, MiB.
    pub fn overcommitted(&self) -> Vec<(String, u64, u64)> {
        self.volumes
            .iter()
            .filter(|pool| pool.is_pool())
            .filter_map(|pool| {
                let Size::Mib(size) = pool.size else {
                    return None;
                };
                let promised: u64 = self
                    .thin_of(pool)
                    .filter_map(|thin| match thin.size {
                        Size::Mib(mib) => Some(mib),
                        _ => None,
                    })
                    .sum();

                (promised > size).then(|| (format!("{}/{}", pool.group, pool.name), size, promised))
            })
            .collect()
    }

    pub fn check_fit(
        &self,
        firmware: Firmware,
        disk_size: impl Fn(&str) -> Option<u64>,
    ) -> Result<(), Error> {
        let sizes = self.partition_sizes(firmware, disk_size)?;
        let sizes: Vec<u64> = self
            .partitions
            .iter()
            .zip(&sizes)
            .map(|(part, size)| size.saturating_sub(Crypt::overhead(part.crypt)))
            .collect();

        for (part, size) in self.partitions.iter().zip(&sizes) {
            let minimum = if part.usage == Use::Boot { BOOT_MIN } else { 0 };

            if *size < minimum {
                return Err(invalid(tr!(
                    "{}: {} needs at least {}MiB, got {size}MiB",
                    "{}: для {} нужно не меньше {}MiB, получается {size}MiB",
                    part.disk,
                    part.usage,
                    minimum
                )));
            }

            Self::check_root_size(&part.usage, *size)?;
        }

        for raid in &self.raids {
            let members = self.sizes_of(
                &sizes,
                |part| matches!(&part.usage, Use::Raid(name) if name == &raid.name),
            );
            let capacity = raid
                .level
                .capacity(&members)
                .saturating_sub(METADATA_RESERVE + Crypt::overhead(raid.crypt));

            if capacity == 0 {
                return Err(invalid(tr!(
                    "array {} turns out empty",
                    "массив {} получается пустым",
                    raid.name
                )));
            }

            Self::check_root_size(&raid.usage, capacity)?;
        }

        for group in self.groups() {
            self.check_group_fit(&group, &sizes)?;
        }

        Ok(())
    }

    fn sizes_of(&self, sizes: &[u64], select: impl Fn(&Partition) -> bool) -> Vec<u64> {
        self.partitions
            .iter()
            .zip(sizes)
            .filter(|(part, _)| select(part))
            .map(|(_, size)| *size)
            .collect()
    }

    fn check_root_size(usage: &Use, size: u64) -> Result<(), Error> {
        if usage.mount_path() == Some("/") && size < ROOT_MIN {
            return Err(invalid(tr!(
                "the root needs at least {ROOT_MIN}MiB, got {size}MiB",
                "для корня нужно не меньше {ROOT_MIN}MiB, получается {size}MiB"
            )));
        }

        Ok(())
    }

    fn check_group_fit(&self, group: &str, sizes: &[u64]) -> Result<(), Error> {
        let mut capacity = 0;

        for (part, size) in self.partitions.iter().zip(sizes) {
            if matches!(&part.usage, Use::Lvm(name) if name == group) {
                capacity += size.saturating_sub(METADATA_RESERVE);
            }
        }

        for raid in &self.raids {
            if matches!(&raid.usage, Use::Lvm(name) if name == group) {
                let members = self.sizes_of(
                    sizes,
                    |part| matches!(&part.usage, Use::Raid(name) if name == &raid.name),
                );

                capacity += raid
                    .level
                    .capacity(&members)
                    .saturating_sub(METADATA_RESERVE * 2 + Crypt::overhead(raid.crypt));
            }
        }

        let volumes: Vec<&Volume> = self
            .volumes
            .iter()
            .filter(|volume| volume.group == group)
            .collect();
        let fixed: u64 = volumes
            .iter()
            .filter(|volume| volume.pool.is_none())
            .filter_map(|volume| match volume.size {
                Size::Mib(mib) => Some(mib),
                _ => None,
            })
            .sum();

        if fixed > capacity {
            return Err(invalid(tr!(
                "group {group}: volumes ask for {fixed}MiB but there is {capacity}MiB",
                "группа {group}: тома просят {fixed}MiB, а места {capacity}MiB"
            )));
        }

        for volume in volumes {
            let size = match volume.size {
                Size::Mib(mib) => mib,
                _ => capacity - fixed,
            };

            if size == 0 && volume.pool.is_none() {
                return Err(invalid(tr!(
                    "volume {}/{}: no space left",
                    "том {}/{}: места не осталось",
                    group,
                    volume.name
                )));
            }

            Self::check_root_size(
                &volume.usage,
                size.saturating_sub(Crypt::overhead(volume.crypt)),
            )?;
        }

        Ok(())
    }

    /// Размеры разделов в мебибайтах с учётом `rest` и `whole`.
    fn partition_sizes(
        &self,
        firmware: Firmware,
        disk_size: impl Fn(&str) -> Option<u64>,
    ) -> Result<Vec<u64>, Error> {
        let mut sizes = vec![0; self.partitions.len()];

        for disk in self.disks() {
            let total = disk_size(&disk)
                .map(|bytes| bytes / MEBIBYTE)
                .ok_or_else(|| {
                    invalid(tr!(
                        "{disk}: failed to get the disk size",
                        "{disk}: не удалось узнать размер диска"
                    ))
                })?;
            let indexes: Vec<usize> = (0..self.partitions.len())
                .filter(|&index| self.partitions[index].disk == disk)
                .collect();
            let gap = if firmware == Firmware::Uboot && disk == self.boot_disk() {
                UBOOT_GAP
            } else {
                0
            };
            let available = total.saturating_sub(TABLE_RESERVE + gap);
            let mut fixed = 0;

            for &index in &indexes {
                match self.partitions[index].size {
                    Size::Mib(mib) => {
                        sizes[index] = mib;
                        fixed += mib;
                    }
                    Size::Whole => sizes[index] = total,
                    Size::Rest => {}
                }
            }

            if fixed > available {
                return Err(invalid(tr!(
                    "{disk}: partitions ask for {fixed}MiB but the disk has {available}MiB",
                    "{disk}: разделы просят {fixed}MiB, а на диске {available}MiB"
                )));
            }

            for &index in &indexes {
                if self.partitions[index].size == Size::Rest {
                    sizes[index] = available - fixed;

                    if sizes[index] == 0 {
                        return Err(invalid(tr!(
                            "{disk}: no space left for rest",
                            "{disk}: для rest места не осталось"
                        )));
                    }
                }
            }
        }

        Ok(sizes)
    }
}

fn boot_area(firmware: Firmware) -> u64 {
    boot_partitions("", firmware)
        .iter()
        .map(|part| match part.size {
            Size::Mib(mib) => mib,
            _ => 0,
        })
        .sum()
}

fn boot_partitions(disk: &str, firmware: Firmware) -> Vec<Partition> {
    let mut partitions = Vec::new();

    if firmware.has_bios() {
        partitions.push(Partition {
            disk: disk.into(),
            size: Size::Mib(BIOS_BOOT_SIZE),
            usage: Use::BiosBoot,
            crypt: None,
        });
    }

    partitions.push(Partition {
        disk: disk.into(),
        size: Size::Mib(BOOT_SIZE),
        usage: Use::Boot,
        crypt: None,
    });
    partitions
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn layout(rows: &[&str]) -> Layout {
        let rows: Vec<String> = rows.iter().map(ToString::to_string).collect();

        Layout::from_rows(&rows).unwrap()
    }

    fn lvm_on_two_disks() -> Layout {
        layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "partition disk=/dev/sdb,size=rest,point=lvm:vg",
            "volume vg=vg,name=root,size=40GiB,point=/",
            "volume vg=vg,name=swap,size=4GiB,point=swap",
            "volume vg=vg,name=home,size=rest,point=/home,fs=xfs",
        ])
    }

    fn raid_layout() -> Layout {
        layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=raid:md0",
            "partition disk=/dev/sdb,size=rest,point=raid:md0",
            "raid name=md0,level=1,point=lvm:vg",
            "volume vg=vg,name=root,size=rest,point=/",
        ])
    }

    fn sizes(disk: &str) -> Option<u64> {
        match disk {
            "/dev/sda" | "/dev/sdb" => Some(100 * GIB),
            "/dev/small" => Some(GIB),
            _ => None,
        }
    }

    #[test]
    fn the_filesystem_cycles_through_all_known_ones() {
        assert_eq!(Fs::Ext4.next(), Fs::Xfs);
        assert_eq!(Fs::Xfs.next(), Fs::Btrfs);
        assert_eq!(Fs::Btrfs.next(), Fs::Ext4);
    }

    #[test]
    fn sizes_are_parsed_and_printed_in_the_largest_exact_unit() {
        assert_eq!(Size::parse("512MiB").unwrap(), Size::Mib(512));
        assert_eq!(Size::parse("2GiB").unwrap(), Size::Mib(2048));
        assert_eq!(Size::parse("1TiB").unwrap(), Size::Mib(1024 * 1024));
        assert_eq!(Size::parse("rest").unwrap(), Size::Rest);
        assert_eq!(Size::parse("whole").unwrap(), Size::Whole);
        assert_eq!(Size::Mib(1536).to_string(), "1536MiB");
        assert_eq!(Size::Mib(2048).to_string(), "2GiB");
        assert_eq!(Size::Mib(1024 * 1024).to_string(), "1TiB");

        for bad in [
            "",
            "0MiB",
            "10",
            "10GB",
            "-1GiB",
            "1.5GiB",
            "99999999999999999999GiB",
            "1 GiB",
            "MiB",
            "1PiB",
        ] {
            assert!(Size::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rows_survive_a_round_trip() {
        for layout in [lvm_on_two_disks(), raid_layout()] {
            assert_eq!(Layout::from_rows(&layout.rows()).unwrap(), layout);
        }
    }

    #[test]
    fn specs_reject_unknown_duplicate_and_malformed_keys() {
        for bad in [
            "disk=/dev/sda,size=1GiB",
            "disk=/dev/sda,size=1GiB,point=boot,point=swap",
            "disk=/dev/sda,size=1GiB,point=boot,color=red",
            "disk=/dev/sda,size=1GiB,point=",
            "disk=/dev/sda,size=1GiB,point=boot,,",
            "disk=/dev/sda size=1GiB point=boot",
            "disk=/dev/sda,size=1GiB,point=boot,fs=xfs",
            "disk=/dev/sda,size=1GiB,point=/,fs=zfs",
            "disk=/dev/sda,size=1GiB,point=nonsense",
            "disk=/dev/sda,size=1GiB,point=lvm",
        ] {
            assert!(Partition::parse(bad).is_err(), "{bad:?}");
        }

        assert!(Raid::parse("name=md0,level=2,point=swap").is_err());
        assert!(Volume::parse("vg=vg,name=a,point=swap").is_err());
        assert!(
            Partition::parse(&format!(
                "disk=/dev/sda,size=1GiB,point={}",
                "x".repeat(600)
            ))
            .is_err()
        );
        assert!(Layout::from_rows(&["disk=/dev/sda".to_string()]).is_err());
        assert!(Layout::from_rows(&["mount disk=/dev/sda".to_string()]).is_err());
    }

    #[test]
    fn the_simple_layouts_are_valid_for_every_firmware() {
        for firmware in [Firmware::Uefi, Firmware::Bios, Firmware::Uboot] {
            let same = Layout::simple("/dev/sda", "/dev/sda", firmware);
            let apart = Layout::simple("/dev/sdb", "/dev/sda", firmware);

            assert!(same.validate(firmware).is_ok(), "{firmware:?}");
            assert!(apart.validate(firmware).is_ok(), "{firmware:?}");
            assert!(!same.separate_boot());
            assert!(apart.separate_boot());
        }
    }

    #[test]
    fn the_simple_layout_numbers_partitions_like_before() {
        let same = Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi);
        let apart = Layout::simple("/dev/sdb", "/dev/sda", Firmware::Uefi);
        let paths = |layout: &Layout| -> Vec<String> {
            layout
                .devices()
                .into_iter()
                .map(|device| device.path)
                .collect()
        };

        assert_eq!(paths(&same), ["/dev/sda1", "/dev/sda2"]);
        assert_eq!(paths(&apart), ["/dev/sda1", "/dev/sdb"]);
        assert_eq!(same.root_device(), "/dev/sda2");
        assert_eq!(apart.root_device(), "/dev/sdb");
        assert_eq!(apart.boot_disk(), "/dev/sda");
    }

    #[test]
    fn partition_names_follow_the_device_kind() {
        assert_eq!(partition_path("/dev/sda", 3), "/dev/sda3");
        assert_eq!(partition_path("/dev/vda", 1), "/dev/vda1");
        assert_eq!(partition_path("/dev/nvme0n1", 3), "/dev/nvme0n1p3");
        assert_eq!(partition_path("/dev/mmcblk0", 2), "/dev/mmcblk0p2");
        assert_eq!(
            partition_path("/dev/disk/by-id/usb-SanDisk_Cruzer-0:0", 2),
            "/dev/disk/by-id/usb-SanDisk_Cruzer-0:0-part2"
        );
        assert_eq!(
            partition_path("/dev/disk/by-path/pci-0000:00:1f.2-ata-1", 1),
            "/dev/disk/by-path/pci-0000:00:1f.2-ata-1-part1"
        );
    }

    #[test]
    fn lvm_and_raid_layouts_are_valid() {
        assert!(lvm_on_two_disks().validate(Firmware::Uefi).is_ok());
        assert!(raid_layout().validate(Firmware::Uefi).is_ok());
    }

    #[test]
    fn pooled_presets_are_valid() {
        let disks = vec!["/dev/sda".to_string(), "/dev/sdb".to_string()];

        for (boot, raid) in [
            ("/dev/sda", None),
            ("/dev/sdb", None),
            ("/dev/sda", Some(Level::Raid1)),
        ] {
            let layout = Layout::pooled(&disks, boot, raid, Firmware::Bios);

            assert!(layout.validate(Firmware::Bios).is_ok(), "{boot} {raid:?}");
            assert_eq!(layout.boot_disk(), boot);
            assert_eq!(layout.root_device(), "/dev/moonshine/root");
        }

        let flash = Layout::pooled(&disks, "/dev/sdc", Some(Level::Raid1), Firmware::Uefi);

        assert!(flash.validate(Firmware::Uefi).is_ok());
        assert!(flash.separate_boot());
        assert_eq!(flash.disks(), ["/dev/sdc", "/dev/sda", "/dev/sdb"]);
    }

    #[test]
    fn raid_members_are_equal_because_the_other_disks_reserve_the_boot_area() {
        let disks = vec!["/dev/sda".to_string(), "/dev/sdb".to_string()];
        let reserved = |layout: &Layout| -> Vec<(String, Size)> {
            layout
                .partitions
                .iter()
                .filter(|part| part.usage == Use::Reserve)
                .map(|part| (part.disk.clone(), part.size))
                .collect()
        };
        let mirrored = Layout::pooled(&disks, "/dev/sda", Some(Level::Raid1), Firmware::Bios);

        assert_eq!(
            reserved(&mirrored),
            [(
                "/dev/sdb".to_string(),
                Size::Mib(BIOS_BOOT_SIZE + BOOT_SIZE)
            )]
        );
        assert!(mirrored.validate(Firmware::Bios).is_ok());

        let uefi = Layout::pooled(&disks, "/dev/sda", Some(Level::Raid1), Firmware::Uefi);

        assert_eq!(
            reserved(&uefi),
            [("/dev/sdb".to_string(), Size::Mib(BOOT_SIZE))]
        );
        assert_eq!(
            reserved(&mirrored.with_boot_size(1024)),
            [("/dev/sdb".to_string(), Size::Mib(BIOS_BOOT_SIZE + 1024))]
        );

        let lvm = Layout::pooled(&disks, "/dev/sda", None, Firmware::Bios);
        let apart = Layout::pooled(&disks, "/dev/sdc", Some(Level::Raid1), Firmware::Bios);

        assert!(reserved(&lvm).is_empty() && reserved(&apart).is_empty());
    }

    #[test]
    fn a_reserve_is_parsed_and_cannot_be_a_mount_or_encrypted() {
        assert_eq!(
            Use::parse("reserve", None, Tuning::default()).unwrap(),
            Use::Reserve
        );
        assert_eq!(Use::Reserve.to_string(), "reserve");
        assert!(Use::Reserve.mount_path().is_none());
    }

    #[test]
    fn disks_under_the_root_follow_volumes_and_arrays() {
        assert_eq!(
            lvm_on_two_disks().disks_under("/dev/vg/root"),
            ["/dev/sda", "/dev/sdb"]
        );
        assert_eq!(
            raid_layout().disks_under("/dev/vg/root"),
            ["/dev/sda", "/dev/sdb"]
        );
        assert_eq!(
            raid_layout().disks_under("/dev/md/md0"),
            ["/dev/sda", "/dev/sdb"]
        );
        assert_eq!(raid_layout().members("md0"), ["/dev/sda2", "/dev/sdb1"]);
        assert_eq!(
            lvm_on_two_disks().volumes_of("vg"),
            ["/dev/sda2", "/dev/sdb1"]
        );
    }

    fn broken(mut rows: Vec<&str>, replace: (usize, &'static str)) -> Layout {
        rows[replace.0] = replace.1;
        layout(&rows)
    }

    #[test]
    fn nonsense_layouts_are_refused() {
        let base = vec![
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/",
        ];
        let cases = [
            broken(
                base.clone(),
                (0, "partition disk=/dev/sda,size=1GiB,point=swap"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=/home"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=512MiB,point=boot"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=whole,point=/"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=/boot"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=/boot/efi"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=/a b"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=/a/"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=/../etc"),
            ),
            broken(base.clone(), (1, "partition disk=sda,size=rest,point=/")),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=lvm:vg"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=raid:md0"),
            ),
            broken(
                base.clone(),
                (1, "partition disk=/dev/sda,size=rest,point=lvm:-x"),
            ),
            broken(
                base.clone(),
                (0, "partition disk=/dev/sda,size=whole,point=boot"),
            ),
            broken(
                base.clone(),
                (0, "partition disk=/dev/sda,size=rest,point=boot"),
            ),
        ];

        for case in cases {
            assert!(case.validate(Firmware::Uefi).is_err(), "{:#?}", case.rows());
        }

        assert!(layout(&base).validate(Firmware::Uefi).is_ok());
    }

    #[test]
    fn the_boot_partitions_must_match_the_firmware() {
        let uefi = Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi);
        let bios = Layout::simple("/dev/sda", "/dev/sda", Firmware::Bios);
        let uboot = Layout::simple("/dev/sda", "/dev/sda", Firmware::Uboot);

        assert!(uefi.validate(Firmware::Bios).is_err());
        assert!(bios.validate(Firmware::Uefi).is_err());
        assert!(bios.validate(Firmware::Uboot).is_err());
        assert!(uboot.validate(Firmware::Uboot).is_ok());

        let late_boot = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=swap",
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/",
        ]);

        assert!(late_boot.validate(Firmware::Uboot).is_err());
    }

    #[test]
    fn boot_cannot_live_in_lvm_or_raid() {
        let no_boot = layout(&[
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=boot,size=1GiB,point=/boot",
            "volume vg=vg,name=root,size=rest,point=/",
        ]);

        assert!(no_boot.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn disks_without_a_table_hold_exactly_one_device() {
        let shared = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sdb,size=whole,point=/",
            "partition disk=/dev/sdb,size=1GiB,point=swap",
        ]);

        assert!(shared.validate(Firmware::Uefi).is_err());

        let rest_first = layout(&[
            "partition disk=/dev/sda,size=rest,point=boot",
            "partition disk=/dev/sda,size=1GiB,point=/",
        ]);

        assert!(rest_first.validate(Firmware::Uefi).is_err());

        let two_rests = layout(&[
            "partition disk=/dev/sda,size=rest,point=boot",
            "partition disk=/dev/sda,size=rest,point=/",
        ]);

        assert!(two_rests.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn raids_need_enough_members_on_different_disks() {
        let one = broken(
            vec![
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=raid:md0",
                "raid name=md0,level=1,point=/",
            ],
            (2, "raid name=md0,level=1,point=/"),
        );

        assert!(one.validate(Firmware::Uefi).is_err());

        let same_disk = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=10GiB,point=raid:md0",
            "partition disk=/dev/sda,size=rest,point=raid:md0",
            "raid name=md0,level=1,point=/",
        ]);

        assert!(same_disk.validate(Firmware::Uefi).is_err());

        let missing = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=raid:nope",
            "partition disk=/dev/sdb,size=whole,point=/",
        ]);

        assert!(missing.validate(Firmware::Uefi).is_err());

        let three = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=raid:md0",
            "partition disk=/dev/sdb,size=whole,point=raid:md0",
            "partition disk=/dev/sdc,size=whole,point=raid:md0",
            "raid name=md0,level=5,point=/",
        ]);
        let four = Layout {
            raids: vec![Raid {
                level: Level::Raid6,
                ..three.raids[0].clone()
            }],
            ..three.clone()
        };

        assert!(three.validate(Firmware::Uefi).is_ok());
        assert!(four.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn groups_and_volumes_must_agree() {
        let rows = [
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
        ];
        let valid = |extra: &[&str]| {
            let all: Vec<&str> = rows.iter().chain(extra).copied().collect();

            layout(&all).validate(Firmware::Uefi)
        };

        assert!(valid(&["volume vg=vg,name=root,size=rest,point=/"]).is_ok());
        assert!(valid(&[]).is_err());
        assert!(valid(&["volume vg=other,name=root,size=rest,point=/"]).is_err());
        assert!(
            valid(&[
                "volume vg=vg,name=root,size=10GiB,point=/",
                "volume vg=vg,name=root,size=rest,point=/home"
            ])
            .is_err()
        );
        assert!(
            valid(&[
                "volume vg=vg,name=root,size=rest,point=/",
                "volume vg=vg,name=home,size=rest,point=/home"
            ])
            .is_err()
        );
        assert!(
            valid(&[
                "volume vg=vg,name=root,size=10GiB,point=/",
                "volume vg=vg,name=other,size=10GiB,point=/"
            ])
            .is_err()
        );
        assert!(valid(&["volume vg=vg,name=root,size=whole,point=/"]).is_err());
        assert!(valid(&["volume vg=vg,name=root,size=10GiB,point=boot"]).is_err());
        assert!(valid(&["volume vg=vg,name=root,size=10GiB,point=lvm:vg"]).is_err());
        assert!(valid(&["volume vg=vg,name=a/b,size=10GiB,point=/"]).is_err());
        assert!(valid(&["volume vg=vg,name=-a,size=10GiB,point=/"]).is_err());
    }

    #[test]
    fn the_boot_size_can_change_within_limits() {
        let layout = Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi).with_boot_size(256);

        assert!(layout.validate(Firmware::Uefi).is_ok());
        assert_eq!(layout.partitions[0].size, Size::Mib(256));
        assert!(layout.with_boot_size(255).validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn the_root_may_be_small_but_not_below_the_minimum() {
        let root = |size: &str| {
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                &format!("partition disk=/dev/sda,size={size},point=/"),
            ])
        };

        assert!(root("512MiB").check_fit(Firmware::Uefi, sizes).is_ok());
        assert!(root("511MiB").check_fit(Firmware::Uefi, sizes).is_err());
    }

    #[test]
    fn too_many_elements_are_refused() {
        let mut huge = Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi);

        for _ in 0..MAX_ITEMS {
            huge.partitions.insert(
                2,
                Partition {
                    disk: "/dev/sda".into(),
                    size: Size::Mib(1),
                    usage: Use::Swap,
                    crypt: None,
                },
            );
        }

        assert!(huge.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn a_layout_without_a_root_is_refused() {
        let layout = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/home",
        ]);

        assert!(layout.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn the_fit_check_uses_real_disk_sizes() {
        assert!(lvm_on_two_disks().check_fit(Firmware::Uefi, sizes).is_ok());
        assert!(raid_layout().check_fit(Firmware::Uefi, sizes).is_ok());
        assert!(
            Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi)
                .check_fit(Firmware::Uefi, sizes)
                .is_ok()
        );
        assert!(
            Layout::simple("/dev/small", "/dev/small", Firmware::Uefi)
                .check_fit(Firmware::Uefi, sizes)
                .is_err()
        );
        assert!(
            Layout::simple("/dev/missing", "/dev/sda", Firmware::Uefi)
                .check_fit(Firmware::Uefi, sizes)
                .is_err()
        );
    }

    #[test]
    fn volumes_that_do_not_fit_the_group_are_refused() {
        let too_big = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=root,size=90GiB,point=/",
            "volume vg=vg,name=home,size=20GiB,point=/home",
        ]);
        let no_room_left = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=root,size=101370MiB,point=/",
            "volume vg=vg,name=home,size=rest,point=/home",
        ]);
        let tiny_root = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=root,size=400MiB,point=/",
        ]);

        assert!(too_big.check_fit(Firmware::Uefi, sizes).is_err());
        assert!(no_room_left.check_fit(Firmware::Uefi, sizes).is_err());
        assert!(tiny_root.check_fit(Firmware::Uefi, sizes).is_err());
    }

    #[test]
    fn the_raid_capacity_follows_the_level() {
        let sizes = [100, 80, 120];

        assert_eq!(Level::Raid0.capacity(&sizes), 300);
        assert_eq!(Level::Raid1.capacity(&sizes), 80);
        assert_eq!(Level::Raid5.capacity(&sizes), 160);
        assert_eq!(Level::Raid6.capacity(&[10, 10, 10, 10]), 20);
        assert_eq!(Level::Raid10.capacity(&[10, 10, 10, 10]), 20);
    }

    #[test]
    fn the_uboot_gap_is_reserved_on_the_boot_disk() {
        let layout = Layout::simple("/dev/sda", "/dev/sda", Firmware::Uboot);
        let exact = |bytes: u64| move |_: &str| Some(bytes);

        assert!(
            layout
                .check_fit(Firmware::Uboot, exact(1042 * MEBIBYTE))
                .is_ok()
        );
        assert!(
            layout
                .check_fit(Firmware::Uboot, exact(1041 * MEBIBYTE))
                .is_err()
        );
    }

    #[test]
    fn describing_the_layout_lists_every_device() {
        let lines = raid_layout().describe();

        assert_eq!(lines.len(), 5);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/dev/md/md0") && line.contains("RAID1"))
        );
        assert!(lines.iter().any(|line| line.contains("/dev/vg/root")));
    }

    #[test]
    fn usr_may_live_apart_but_virtual_and_linked_directories_may_not() {
        let with = |mount: &str| {
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                &format!("partition disk=/dev/sda,size=20GiB,point={mount}"),
                "partition disk=/dev/sda,size=rest,point=/",
            ])
        };

        for good in ["/usr", "/usr/local", "/var", "/opt"] {
            assert!(with(good).validate(Firmware::Uefi).is_ok(), "{good}");
        }

        for bad in [
            "/dev", "/proc", "/sys", "/run", "/etc", "/bin", "/sbin", "/lib", "/lib64", "/sys/x",
        ] {
            assert!(with(bad).validate(Firmware::Uefi).is_err(), "{bad}");
        }
    }

    fn crypt_layout(data: &str) -> Layout {
        layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/,crypt=pass",
            data,
        ])
    }

    #[test]
    fn crypt_and_header_are_read_written_and_survive_a_round_trip() {
        let row = "disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key,header=detached";
        let part = Partition::parse(row).unwrap();

        assert_eq!(
            part.crypt,
            Some(Crypt {
                unlock: Unlock::Keyfile,
                detached: true
            })
        );
        assert_eq!(part.to_string(), row);

        let pass = "disk=/dev/sdb,size=rest,point=/srv,crypt=pass";

        assert_eq!(Partition::parse(pass).unwrap().to_string(), pass);

        let volume = "vg=vg,name=data,size=rest,point=/srv,crypt=key";
        let raid = "name=md0,level=1,point=lvm:vg,crypt=pass,header=detached";

        assert_eq!(Volume::parse(volume).unwrap().to_string(), volume);
        assert_eq!(Raid::parse(raid).unwrap().to_string(), raid);
    }

    #[test]
    fn a_header_without_crypt_and_unknown_crypt_modes_are_refused() {
        for bad in [
            "disk=/dev/sdb,size=rest,point=/srv,header=detached",
            "disk=/dev/sdb,size=rest,point=/srv,crypt=yes",
            "disk=/dev/sdb,size=rest,point=/srv,crypt=pass,header=floating",
        ] {
            assert!(Partition::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn boot_and_biosboot_cannot_be_encrypted() {
        let boot = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot,crypt=pass",
            "partition disk=/dev/sda,size=rest,point=/",
        ]);
        let bios = layout(&[
            "partition disk=/dev/sda,size=4MiB,point=biosboot,crypt=pass",
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/",
        ]);

        assert!(boot.validate(Firmware::Uefi).is_err());
        assert!(bios.validate(Firmware::Bios).is_err());
    }

    #[test]
    fn the_root_and_what_it_stands_on_are_opened_early_data_is_not() {
        let plain = crypt_layout("partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=pass");

        assert_eq!(plain.early(), ["/dev/sda2"]);
        assert_eq!(plain.crypts().len(), 2);
        assert!(plain.protected("/dev/sda2") && !plain.protected("/dev/sda1"));
        assert_eq!(plain.crypt_name("/dev/sda2"), "root");
        assert_eq!(plain.mapper("/dev/sda2"), "cryptroot");
        assert_eq!(plain.mapper("/dev/sdb"), "crypt-sdb");
        assert_eq!(plain.inner("/dev/sda2"), "/dev/mapper/cryptroot");
        assert_eq!(plain.inner("/dev/sda1"), "/dev/sda1");

        let stacked = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=raid:md0,crypt=pass",
            "partition disk=/dev/sdb,size=rest,point=raid:md0,crypt=pass",
            "raid name=md0,level=1,point=lvm:vg,crypt=pass",
            "volume vg=vg,name=root,size=20GiB,point=/",
            "volume vg=vg,name=usr,size=20GiB,point=/usr,crypt=pass",
            "volume vg=vg,name=data,size=rest,point=/srv,crypt=pass",
        ]);

        assert_eq!(
            stacked.early(),
            ["/dev/sda2", "/dev/sdb1", "/dev/md/md0", "/dev/vg/usr"]
        );
        assert!(stacked.validate(Firmware::Uefi).is_ok());
    }

    #[test]
    fn a_key_in_etc_needs_an_encrypted_root_and_an_exposed_key_in_the_initramfs_needs_a_detached_header()
     {
        let data = "partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key";
        let plain_root = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/",
            data,
        ]);

        assert!(plain_root.validate(Firmware::Uefi).is_err());
        assert!(crypt_layout(data).validate(Firmware::Uefi).is_ok());

        let exposed = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/,crypt=key",
        ]);
        let hidden = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/,crypt=key,header=detached",
        ]);

        assert!(exposed.validate(Firmware::Uefi).is_err());
        assert!(hidden.validate(Firmware::Uefi).is_ok());
    }

    #[test]
    fn a_separate_boot_disk_needs_a_detached_header_on_the_early_devices_only() {
        let attached = layout(&[
            "partition disk=/dev/sdc,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=whole,point=/,crypt=pass",
        ]);
        let detached = layout(&[
            "partition disk=/dev/sdc,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=whole,point=/,crypt=pass,header=detached",
            "partition disk=/dev/sdb,size=whole,point=/srv,crypt=pass",
        ]);

        assert!(attached.validate(Firmware::Uefi).is_err());
        assert!(detached.validate(Firmware::Uefi).is_ok());
    }

    #[test]
    fn two_encrypted_devices_cannot_share_a_name() {
        let clash = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=/,crypt=pass",
            "partition disk=/dev/sdb,size=whole,point=/srv,crypt=pass",
            "partition disk=/dev/disk/by-id/sdb,size=whole,point=/opt,crypt=pass",
        ]);

        assert!(clash.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn an_attached_header_takes_space_from_the_encrypted_partition() {
        let sizes = |_: &str| Some(2 * GIB);
        let root = |size: &str, crypt: &str| {
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                &format!("partition disk=/dev/sda,size={size},point=/{crypt}"),
            ])
        };

        assert!(root("512MiB", "").check_fit(Firmware::Uefi, sizes).is_ok());
        assert!(
            root("512MiB", ",crypt=pass")
                .check_fit(Firmware::Uefi, sizes)
                .is_err()
        );
        assert!(
            root("528MiB", ",crypt=pass")
                .check_fit(Firmware::Uefi, sizes)
                .is_ok()
        );
        assert!(
            root("512MiB", ",crypt=pass,header=detached")
                .check_fit(Firmware::Uefi, sizes)
                .is_ok()
        );
    }

    #[test]
    fn with_root_crypt_marks_only_what_is_mounted_at_the_root() {
        let marked = lvm_on_two_disks().with_root_crypt(Some(Crypt {
            unlock: Unlock::Typed,
            detached: false,
        }));

        assert_eq!(marked.crypts().len(), 1);
        assert_eq!(marked.crypts()[0].path, "/dev/vg/root");
        assert!(marked.with_root_crypt(None).crypts().is_empty());
    }

    #[test]
    fn use_key_is_rejected() {
        assert!(Partition::parse("disk=/dev/sda,size=rest,use=/home").is_err());
        let partition = Partition::parse("disk=/dev/sda,size=rest,point=/home").unwrap();
        assert_eq!(partition.to_string(), "disk=/dev/sda,size=rest,point=/home");
    }

    #[test]
    fn mkfs_and_mount_options_are_read_written_and_round_trip() {
        let row = "disk=/dev/sda,size=rest,point=/data,mkfs=-E lazy_itable_init=0;discard,options=noatime;discard";
        let part = Partition::parse(row).unwrap();
        let Use::Mount { tuning, .. } = &part.usage else {
            panic!("не точка монтирования");
        };

        assert_eq!(tuning.mkfs, ["-E", "lazy_itable_init=0,discard"]);
        assert_eq!(tuning.options, ["noatime", "discard"]);
        assert_eq!(tuning.fstab_options(), "noatime,discard");
        assert_eq!(part.to_string(), row);
        assert_eq!(Tuning::default().fstab_options(), "defaults");
    }

    #[test]
    fn mkfs_and_options_only_make_sense_on_a_mount_point_and_must_be_safe() {
        for bad in [
            "disk=/dev/sda,size=rest,point=swap,mkfs=-m 1",
            "disk=/dev/sda,size=rest,point=lvm:vg,options=noatime",
            "disk=/dev/sda,size=rest,point=/d,mkfs=-U 1234",
            "disk=/dev/sda,size=rest,point=/d,mkfs=-F",
            "disk=/dev/sda,size=rest,point=/d,mkfs=/dev/sdb",
            "disk=/dev/sda,size=rest,point=/d,mkfs=$(reboot)",
            "disk=/dev/sda,size=rest,point=/d,options=a b",
            "disk=/dev/sda,size=rest,point=/d,options=a;;b",
        ] {
            assert!(Partition::parse(bad).is_err(), "{bad}");
        }
    }

    fn thin() -> Layout {
        layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=pool,size=100GiB,point=pool",
            "volume vg=vg,name=root,size=50GiB,point=/,pool=pool",
            "volume vg=vg,name=home,size=80GiB,point=/home,pool=pool",
        ])
    }

    #[test]
    fn thin_volumes_round_trip_and_the_pool_is_not_a_device() {
        let layout = thin();

        assert_eq!(Layout::from_rows(&layout.rows()).unwrap(), layout);
        assert!(layout.rows()[3].ends_with(",pool=pool"));
        assert!(layout.validate(Firmware::Uefi).is_ok());
        assert!(layout.needs_thin());
        assert!(
            layout
                .devices()
                .iter()
                .all(|device| device.usage != Use::Pool)
        );
        assert_eq!(
            layout.overcommitted(),
            [("vg/pool".to_string(), 102400, 133120)]
        );
    }

    #[test]
    fn thin_volumes_need_a_pool_of_their_group_and_a_pool_needs_volumes() {
        let without_pool = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=root,size=50GiB,point=/,pool=nope",
        ]);
        let empty_pool = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=lvm:vg",
            "volume vg=vg,name=pool,size=10GiB,point=pool",
            "volume vg=vg,name=root,size=10GiB,point=/",
        ]);

        assert!(without_pool.validate(Firmware::Uefi).is_err());
        assert!(empty_pool.validate(Firmware::Uefi).is_err());
    }

    #[test]
    fn thin_and_pool_specs_reject_nonsense() {
        for bad in [
            "vg=vg,name=a,size=1GiB,point=pool,crypt=pass",
            "vg=vg,name=a,size=1GiB,point=pool,pool=b",
            "vg=vg,name=a,size=1GiB,point=swap,pool=b",
            "vg=vg,name=a,size=rest,point=/,pool=b",
        ] {
            assert!(Volume::parse(bad).is_err(), "{bad}");
        }

        assert!(Partition::parse("disk=/dev/sda,size=rest,point=pool").is_ok());
        assert!(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=pool",
            ])
            .validate(Firmware::Uefi)
            .is_err()
        );
    }

    #[test]
    fn thin_volumes_do_not_take_space_from_the_group() {
        let sizes = |disk: &str| (disk == "/dev/sda").then_some(300 * GIB);

        assert!(thin().check_fit(Firmware::Uefi, sizes).is_ok());
    }
}
