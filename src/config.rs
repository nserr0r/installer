use std::fmt;

use uuid::Uuid;

use crate::error::Error;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::layout::Crypt;
use crate::layout::Layout;
use crate::network::Backend;
use crate::network::Network;
use crate::repository::Repository;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Firmware {
    Uefi,
    Bios,
    /// BIOS и UEFI одновременно: диск грузится на любой прошивке x86_64.
    Hybrid,
    Uboot,
}

impl Firmware {
    pub fn has_bios(self) -> bool {
        matches!(self, Self::Bios | Self::Hybrid)
    }

    pub fn has_efi(self) -> bool {
        matches!(self, Self::Uefi | Self::Hybrid)
    }
}

/// Как открывается зашифрованное устройство.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Unlock {
    /// Парольная фраза вводится вручную.
    #[default]
    Typed,
    /// Случайный ключ-файл в `/etc/luks`; для корня он ещё и в initramfs на диске загрузки.
    Keyfile,
}

/// Секторов (по 512 байт) в начале диска загрузки, которые U-Boot оставляет под сырой загрузчик.
pub const UBOOT_RESERVED_SECTORS: u64 = 32768;

/// Случайный ключ для ключ-файла: 32 байта из `/dev/urandom` в шестнадцатеричном виде.
pub fn random_key() -> Result<Secret, Error> {
    use std::io::Read;

    let mut bytes = [0u8; 32];

    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|error| {
            Error::Invalid(tr!(
                "failed to get a random key: {error}",
                "не удалось получить случайный ключ: {error}"
            ))
        })?;

    Ok(Secret::new(
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    ))
}

/// Сырой образ U-Boot, который записывается на диск загрузки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uboot {
    pub image: String,
    pub offset: u64,
}

/// Что ставится: репозитории хоста, локальные каталоги с пакетами, дополнительные пакеты,
/// версия выпуска и файлы `.repo`, которые переносятся в систему.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    pub repos: Vec<String>,
    pub local: Vec<String>,
    pub packages: Vec<String>,
    /// Параметры dnf вида `ключ=значение`; `releasever=44` задаёт версию выпуска.
    pub options: Vec<String>,
    pub copy: Vec<String>,
    /// Репозитории, описанные в конфиге: попадают в установку и остаются в `/etc/yum.repos.d` системы.
    pub repositories: Vec<Repository>,
    /// Приоритеты репозиториев хоста из `repos`; действуют только во время установки.
    pub priorities: Vec<(String, u32)>,
    /// Без встроенного набора пакетов (`@core`, ядро, dracut, загрузчик): всё задаётся в `packages`.
    pub bare: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Arch {
    #[default]
    X86_64,
    Aarch64,
}

/// Производитель процессора: от него зависит микрокод.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Cpu {
    Intel,
    Amd,
    #[default]
    Other,
}

#[derive(Debug, Clone)]
pub struct User {
    pub name: String,
    pub password: Secret,
    /// `password` — готовый хеш (`$6$…`), а не открытый пароль.
    pub hashed: bool,
}

pub const ROOT: &str = "root";

/// Случайные ключи для устройств, которые открываются ключ-файлом.
pub fn generate_keys(layout: &Layout) -> Result<Vec<(String, Secret)>, Error> {
    layout
        .crypts()
        .into_iter()
        .filter(|device| {
            matches!(
                device.crypt,
                Some(Crypt {
                    unlock: Unlock::Keyfile,
                    ..
                })
            )
        })
        .map(|device| Ok((device.path, random_key()?)))
        .collect()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub layout: Layout,
    pub firmware: Firmware,
    pub source: Source,
    pub arch: Arch,
    pub cpu: Cpu,
    pub hostname: String,
    /// Пользователи, кроме `root`, получают группу `wheel`; если `root` не указан, он блокируется.
    pub users: Vec<User>,
    /// Одна парольная фраза на все устройства с `crypt=pass`.
    pub passphrase: Option<Secret>,
    /// Случайные ключи устройств с `crypt=key`: путь устройства и ключ.
    pub keys: Vec<(String, Secret)>,
    pub uboot: Option<Uboot>,
    pub timezone: String,
    pub initrd_network: Network,
    pub system_network: Network,
    pub backend: Backend,
    pub ssh_keys: Vec<String>,
    /// Дополнительные параметры ядра: дописываются к `GRUB_CMDLINE_LINUX` после служебных.
    pub kernel_args: Vec<String>,
    /// Службы systemd, которые включаются в установленной системе (`systemctl enable`).
    pub services: Vec<String>,
}

fn name_chars(text: &str, extra: &[char]) -> bool {
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

const MAX_TEXT: usize = 255;

/// Абсолютный путь из безопасных символов, без `..`: он попадает в аргументы команд и файлы.
pub(crate) fn is_safe_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= MAX_TEXT
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+@:~".contains(c))
        && path.split('/').all(|part| part != "..")
}

pub fn check_disk(disk: &str) -> Result<(), Error> {
    let name = disk.strip_prefix("/dev/").unwrap_or_default();

    if !name.is_empty() && !name.ends_with('/') && is_safe_path(disk) {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "invalid device: {disk}",
            "неверное устройство: {disk}"
        )))
    }
}

pub fn check_hostname(hostname: &str) -> Result<(), Error> {
    if hostname.is_empty()
        || hostname.len() > 63
        || !name_chars(hostname, &['-'])
        || hostname.starts_with('-')
    {
        return Err(Error::Invalid(tr!(
            "invalid hostname: {hostname}",
            "неверное имя машины: {hostname}"
        )));
    }

    Ok(())
}

pub fn check_user(user: &str) -> Result<(), Error> {
    let start = user
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_');

    if !start
        || user.len() > 32
        || !user
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(Error::Invalid(tr!(
            "invalid user name: {user}",
            "неверное имя пользователя: {user}"
        )));
    }

    Ok(())
}

pub fn check_password_hash(hash: &str) -> Result<(), Error> {
    let ok = hash.starts_with('$')
        && hash.len() > 8
        && hash.len() <= 512
        && !hash.contains(|c: char| c.is_whitespace() || c.is_control() || c == ':');

    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(
            tt!(
                "the password hash must look like $id$salt$hash without spaces and colons",
                "хеш пароля должен иметь вид $id$соль$хеш без пробелов и двоеточий"
            )
            .into(),
        ))
    }
}

pub fn check_password(password: &str) -> Result<(), Error> {
    if password.is_empty() || password.len() > 1024 || password.contains(char::is_control) {
        return Err(Error::Invalid(
            tt!(
                "the password is empty, too long or contains control characters",
                "пароль пуст, слишком длинный или содержит управляющие символы"
            )
            .into(),
        ));
    }

    Ok(())
}

pub fn check_passphrase(phrase: &str) -> Result<(), Error> {
    if phrase.is_empty() || phrase.len() > 1024 || phrase.contains(char::is_control) {
        return Err(Error::Invalid(
            tt!(
                "the disk passphrase is empty, too long or contains control characters",
                "парольная фраза диска пуста, слишком длинна или содержит управляющие символы"
            )
            .into(),
        ));
    }

    Ok(())
}

pub fn check_timezone(timezone: &str) -> Result<(), Error> {
    let ok = !timezone.is_empty()
        && !timezone.starts_with('/')
        && timezone
            .split('/')
            .all(|part| !part.is_empty() && part != ".." && name_chars(part, &['_', '-', '+']));

    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "invalid time zone: {timezone}",
            "неверный часовой пояс: {timezone}"
        )))
    }
}

const SSH_KEY_TYPES: [&str; 6] = [
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
];

/// Ключ SSH: тип, данные base64 и необязательный комментарий. Параметры (`command=…`) не допускаются.
pub fn check_ssh_key(key: &str) -> Result<(), Error> {
    let mut parts = key.trim().splitn(3, ' ');
    let kind = parts.next().unwrap_or_default();
    let data = parts.next().unwrap_or_default();
    let comment = parts.next().unwrap_or_default();
    let ok = SSH_KEY_TYPES.contains(&kind)
        && !data.is_empty()
        && data
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        && !comment.contains(char::is_control)
        && key.len() <= 8192;

    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(
            tt!("SSH key: a single line like \"ssh-ed25519 AAAA… comment\" without options is required", "ключ SSH: нужна одна строка вида «ssh-ed25519 AAAA… комментарий» без параметров")
                .into(),
        ))
    }
}

pub fn check_repo_id(id: &str) -> Result<(), Error> {
    if !id.is_empty() && id.len() <= 128 && name_chars(id, &['.', '_', '-', ':', '+']) {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "invalid repository id: {id}",
            "неверный идентификатор репозитория: {id}"
        )))
    }
}

pub fn check_local_repo(path: &str) -> Result<(), Error> {
    if is_safe_path(path) {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "local repository: an absolute directory path of letters, digits and \"/._-+@:~\" is required: {path}",
            "локальный репозиторий: нужен абсолютный путь каталога из букв, цифр и «/._-+@:~»: {path}"
        )))
    }
}

pub fn check_package(spec: &str) -> Result<(), Error> {
    let first = spec.chars().next();

    if first.is_some_and(|c| c != '-')
        && spec.len() <= MAX_TEXT
        && name_chars(spec, &['.', '_', '-', '+', '@', ':', '*', '^'])
    {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "invalid package name: {spec}",
            "неверное имя пакета: {spec}"
        )))
    }
}

/// Разбирает параметр dnf `ключ=значение`; значение может быть пустым.
pub fn split_option(option: &str) -> Result<(&str, &str), Error> {
    let invalid = || {
        Error::Invalid(tr!(
            "dnf option: key=value expected: {option}",
            "параметр dnf: нужен вид ключ=значение: {option}"
        ))
    };
    let (key, value) = option.split_once('=').ok_or_else(invalid)?;

    if key.is_empty()
        || option.len() > MAX_TEXT
        || !name_chars(key, &['_', '.', '-'])
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+:/,@~*=%?".contains(c))
    {
        return Err(invalid());
    }

    Ok((key, value))
}

/// Параметр ядра попадает в `/etc/default/grub`, который читает оболочка, поэтому кавычки, `$`, `\` и обратные апострофы запрещены.
pub fn check_kernel_arg(arg: &str) -> Result<(), Error> {
    if !arg.is_empty()
        && arg.len() <= MAX_TEXT
        && arg
            .chars()
            .all(|c| c.is_ascii_graphic() && !"\"'`$\\".contains(c))
    {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "kernel parameter: printable ASCII without spaces, quotes, $, ` and \\ is required: {arg}",
            "параметр ядра: нужны печатные ASCII-символы без пробелов, кавычек, $, ` и \\: {arg}"
        )))
    }
}

/// Имя юнита systemd: буквы, цифры и `:-_.@`; не с `-` и не с `.`, чтобы его не приняли за параметр или скрытый файл.
pub fn check_service(name: &str) -> Result<(), Error> {
    if !name.is_empty()
        && name.len() <= MAX_TEXT
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":-_.@".contains(c))
    {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "service: a unit name of letters, digits and \":-_.@\" is required, not starting with - or .: {name}",
            "служба: нужно имя юнита из букв, цифр и \":-_.@\", не начинающееся с - и .: {name}"
        )))
    }
}

pub fn check_option(option: &str) -> Result<(), Error> {
    split_option(option).map(|_| ())
}

pub fn check_repo_file(path: &str) -> Result<(), Error> {
    if is_safe_path(path) && path.ends_with(".repo") {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "repository file: an absolute .repo path of letters, digits and \"/._-+@:~\" is required: {path}",
            "файл репозитория: нужен абсолютный путь к .repo из букв, цифр и «/._-+@:~»: {path}"
        )))
    }
}

impl Source {
    pub fn validate(&self) -> Result<(), Error> {
        if self.repos.is_empty() && self.local.is_empty() && self.repositories.is_empty() {
            return Err(Error::Invalid(
                tt!(
                    "no repository is chosen and no local directory is given",
                    "не выбран ни один репозиторий и не указан локальный каталог"
                )
                .into(),
            ));
        }

        if self.bare && self.packages.is_empty() {
            return Err(Error::Invalid(
                tt!(
                    "--bare: the package set is empty, give it with --package",
                    "--bare: набор пакетов пуст, укажите его через --package"
                )
                .into(),
            ));
        }

        self.validate_repositories()?;

        self.repos.iter().try_for_each(|id| check_repo_id(id))?;
        self.local
            .iter()
            .try_for_each(|path| check_local_repo(path))?;
        self.packages
            .iter()
            .try_for_each(|spec| check_package(spec))?;
        self.copy
            .iter()
            .try_for_each(|path| check_repo_file(path))?;
        self.options
            .iter()
            .try_for_each(|option| check_option(option))
    }
}

impl Source {
    fn validate_repositories(&self) -> Result<(), Error> {
        let mut seen: Vec<&str> = self.repos.iter().map(String::as_str).collect();

        for repository in &self.repositories {
            if seen.contains(&repository.id.as_str()) || repository.id.starts_with("moonshine-") {
                return Err(Error::Invalid(tr!(
                    "repository {}: the id is taken or reserved (moonshine-…)",
                    "репозиторий {}: идентификатор уже занят или зарезервирован (moonshine-…)",
                    repository.id
                )));
            }

            seen.push(&repository.id);
        }

        for (index, (id, _)) in self.priorities.iter().enumerate() {
            if !self.repos.contains(id) {
                return Err(Error::Invalid(tr!(
                    "priority is set for {id}, but there is no such host repository among --repo",
                    "приоритет задан для {id}, а такого репозитория хоста нет среди --repo"
                )));
            }

            if self.priorities[..index]
                .iter()
                .any(|(known, _)| known == id)
            {
                return Err(Error::Invalid(tr!(
                    "priority of {id} is given twice",
                    "приоритет {id} указан дважды"
                )));
            }
        }

        Ok(())
    }
}

impl User {
    fn validate(&self) -> Result<(), Error> {
        check_user(&self.name)?;

        if self.hashed {
            check_password_hash(self.password.expose())
        } else {
            check_password(self.password.expose())
        }
    }
}

impl Config {
    fn validate_users(&self) -> Result<(), Error> {
        if self.users.is_empty() {
            return Err(Error::Invalid(
                tt!("no user is given", "не указан ни один пользователь").into(),
            ));
        }

        for (index, user) in self.users.iter().enumerate() {
            user.validate()?;

            if self.users[..index]
                .iter()
                .any(|other| other.name == user.name)
            {
                return Err(Error::Invalid(tr!(
                    "user is given twice: {}",
                    "пользователь указан дважды: {}",
                    user.name
                )));
            }
        }

        Ok(())
    }

    /// У каждого зашифрованного устройства есть чем его открыть.
    fn validate_secrets(&self) -> Result<(), Error> {
        for (_, key) in &self.keys {
            check_passphrase(key.expose())?;
        }

        for device in self.layout.crypts() {
            let Some(crypt) = device.crypt else {
                continue;
            };

            let present = match crypt.unlock {
                Unlock::Typed => self.passphrase.is_some(),
                Unlock::Keyfile => self.keys.iter().any(|(path, _)| *path == device.path),
            };

            if !present {
                return Err(Error::Invalid(tr!(
                    "{}: neither a passphrase nor a key for encryption",
                    "{}: нет ни парольной фразы, ни ключа для шифрования",
                    device.path
                )));
            }
        }

        Ok(())
    }

    /// Чем открывается зашифрованное устройство: общая фраза или его собственный ключ.
    pub fn secret(&self, path: &str) -> Option<&Secret> {
        match self.layout.crypt_of(path)?.unlock {
            Unlock::Typed => self.passphrase.as_ref(),
            Unlock::Keyfile => self
                .keys
                .iter()
                .find(|(known, _)| known == path)
                .map(|(_, key)| key),
        }
    }

    fn validate_platform(&self) -> Result<(), Error> {
        match (self.firmware, self.arch) {
            (Firmware::Bios | Firmware::Hybrid, Arch::Aarch64) => Err(Error::Invalid(
                tt!("BIOS exists only on x86_64", "BIOS бывает только на x86_64").into(),
            )),
            (Firmware::Uboot, Arch::X86_64) => Err(Error::Invalid(
                tt!(
                    "U-Boot is supported only on aarch64",
                    "U-Boot поддерживается только на aarch64"
                )
                .into(),
            )),
            _ => Ok(()),
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        self.source.validate()?;
        self.layout.validate(self.firmware)?;
        check_hostname(&self.hostname)?;
        self.validate_users()?;
        self.validate_platform()?;

        if let Some(phrase) = &self.passphrase {
            check_passphrase(phrase.expose())?;
        }

        self.validate_secrets()?;

        if let Some(uboot) = &self.uboot {
            if !is_safe_path(&uboot.image) {
                return Err(Error::Invalid(tr!(
                    "U-Boot image: an absolute path of letters, digits and \"/._-+@:~\" is required: {}",
                    "образ U-Boot: нужен абсолютный путь из букв, цифр и «/._-+@:~»: {}",
                    uboot.image
                )));
            }

            if uboot.offset >= UBOOT_RESERVED_SECTORS {
                return Err(Error::Invalid(tr!(
                    "the U-Boot offset must be below {UBOOT_RESERVED_SECTORS} sectors",
                    "смещение U-Boot должно быть меньше {UBOOT_RESERVED_SECTORS} секторов"
                )));
            }
        }

        check_timezone(&self.timezone)?;
        self.kernel_args
            .iter()
            .try_for_each(|arg| check_kernel_arg(arg))?;
        self.services
            .iter()
            .try_for_each(|name| check_service(name))?;
        self.ssh_keys
            .iter()
            .try_for_each(|key| check_ssh_key(key))?;

        if !self.ssh_keys.is_empty() && self.initrd_network == Network::None {
            return Err(Error::Invalid(
                tt!(
                    "SSH keys for the initramfs need a network: choose dhcp or manual",
                    "ключи SSH для initramfs требуют сети: выберите dhcp или manual"
                )
                .into(),
            ));
        }

        self.initrd_network.validate()?;
        self.system_network.validate()?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ids {
    /// UUID LUKS и раздела корня: у остальных зашифрованных устройств свои, в `locks` и `devices`.
    pub luks: Uuid,
    pub part: Uuid,
    pub fat: u32,
    root: String,
    devices: Vec<(String, Uuid)>,
    arrays: Vec<(String, Uuid)>,
    locks: Vec<(String, Uuid)>,
}

impl Ids {
    pub fn random(layout: &Layout) -> Self {
        Self::generate(layout, Uuid::new_v4)
    }

    pub fn generate(layout: &Layout, mut next: impl FnMut() -> Uuid) -> Self {
        Self {
            luks: next(),
            part: next(),
            fat: next().as_fields().0,
            root: layout.root_device(),
            devices: layout
                .devices()
                .into_iter()
                .map(|device| (device.path, next()))
                .collect(),
            arrays: layout
                .raids
                .iter()
                .map(|raid| (raid.name.clone(), next()))
                .collect(),
            locks: layout
                .crypts()
                .into_iter()
                .filter(|device| device.path != layout.root_device())
                .map(|device| (device.path, next()))
                .collect(),
        }
    }

    /// UUID заголовка LUKS на устройстве.
    pub fn luks_of(&self, path: &str) -> Uuid {
        if path == self.root {
            self.luks
        } else {
            lookup(&self.locks, path)
        }
    }

    /// PARTUUID раздела: по нему находится устройство без сигнатуры LUKS (заголовок отдельно).
    pub fn partuuid(&self, path: &str) -> Uuid {
        if path == self.root {
            self.part
        } else {
            self.device(path)
        }
    }

    /// UUID файловой системы или раздела подкачки на устройстве.
    pub fn device(&self, path: &str) -> Uuid {
        lookup(&self.devices, path)
    }

    /// UUID массива в формате mdadm: четыре группы по восемь шестнадцатеричных цифр.
    pub fn array(&self, name: &str) -> String {
        let number = lookup(&self.arrays, name).as_u128();

        format!(
            "{:08x}:{:08x}:{:08x}:{:08x}",
            (number >> 96) as u32,
            (number >> 64) as u32,
            (number >> 32) as u32,
            number as u32
        )
    }

    pub fn fat_label(&self) -> String {
        format!("{:04X}-{:04X}", self.fat >> 16, self.fat & 0xFFFF)
    }
}

fn lookup(list: &[(String, Uuid)], key: &str) -> Uuid {
    list.iter()
        .find(|(known, _)| known == key)
        .map_or(Uuid::nil(), |(_, id)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::config;

    #[test]
    fn kernel_parameters_must_be_safe_for_the_shell_file() {
        for good in [
            "quiet",
            "console=ttyS0,115200",
            "rd.luks.allow-discards",
            "a=b:c/d",
        ] {
            assert!(check_kernel_arg(good).is_ok(), "{good}");
        }

        for bad in [
            "", "a b", "a\"b", "a'b", "a$b", "a`b", "a\\b", "а=б", "a\nb",
        ] {
            assert!(check_kernel_arg(bad).is_err(), "{bad:?}");
        }

        let config = Config {
            kernel_args: vec!["bad arg".into()],
            ..config()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn service_names_are_systemd_unit_names() {
        for good in [
            "sshd",
            "chronyd.service",
            "getty@tty1",
            "systemd-resolved",
            "a:b_c",
        ] {
            assert!(check_service(good).is_ok(), "{good}");
        }

        for bad in [
            "", "-now", ".hidden", "a b", "a;b", "a/b", "a\\b", "а", "$(x)",
        ] {
            assert!(check_service(bad).is_err(), "{bad:?}");
        }

        let config = Config {
            services: vec!["a b".into()],
            ..config()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn a_normal_config_is_valid() {
        assert!(config().validate().is_ok());
    }

    fn account(name: &str, password: &str) -> User {
        User {
            name: name.into(),
            password: Secret::new(password),
            hashed: false,
        }
    }

    #[test]
    fn values_that_could_break_commands_or_files_are_refused() {
        let broken = [
            Config {
                layout: Layout::simple("sda", "sda", Firmware::Uefi),
                ..config()
            },
            Config {
                layout: Layout::simple("/dev/sda --force", "/dev/sda --force", Firmware::Uefi),
                ..config()
            },
            Config {
                hostname: "-moon".into(),
                ..config()
            },
            Config {
                hostname: "mo on".into(),
                ..config()
            },
            Config {
                users: vec![account("Alice", "pw")],
                ..config()
            },
            Config {
                users: vec![account("a:b", "pw")],
                ..config()
            },
            Config {
                users: vec![account("alice", "a\nb")],
                ..config()
            },
            Config {
                users: vec![account("alice", "")],
                ..config()
            },
            Config {
                passphrase: Some(Secret::new("")),
                ..config()
            },
            Config {
                timezone: "../etc/passwd".into(),
                ..config()
            },
            Config {
                timezone: "/etc/passwd".into(),
                ..config()
            },
            Config {
                ssh_keys: vec!["ssh-ed25519 AAAA\nssh-rsa BBBB".into()],
                ..config()
            },
        ];

        for config in broken {
            assert!(config.validate().is_err(), "{config:?}");
        }
    }

    #[test]
    fn ssh_keys_need_a_network_in_the_initramfs() {
        let keys = vec!["ssh-ed25519 AAAA".to_string()];

        assert!(
            Config {
                initrd_network: Network::None,
                ssh_keys: keys.clone(),
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                initrd_network: Network::None,
                system_network: Network::Dhcp { interface: None },
                ssh_keys: keys.clone(),
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                initrd_network: Network::Dhcp { interface: None },
                ssh_keys: keys,
                ..config()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn both_networks_are_validated() {
        let bad = Network::Dhcp {
            interface: Some("bad name".into()),
        };

        assert!(
            Config {
                initrd_network: bad.clone(),
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                system_network: bad,
                ..config()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn secrets_are_hidden_from_debug_output() {
        let config = Config {
            users: vec![account("alice", "hunter2")],
            passphrase: Some(Secret::new("swordfish")),
            ..config()
        };
        let text = format!("{config:?}");

        assert!(!text.contains("hunter2") && !text.contains("swordfish"));
    }

    #[test]
    fn devices_and_paths_accept_only_safe_characters() {
        for good in [
            "/dev/sda",
            "/dev/nvme0n1",
            "/dev/disk/by-id/usb-SanDisk_Cruzer-0:0",
            "/dev/disk/by-id/nvme-eui.0025385b",
        ] {
            assert!(check_disk(good).is_ok(), "{good}");
        }

        for bad in [
            "sda",
            "/dev/",
            "/dev/sda/",
            "/dev/../etc/passwd",
            "/etc/passwd",
            "/dev/sda --force",
            "/dev/sda\nx",
            "/dev/sda;reboot",
            "/dev/sda$(id)",
            "/dev/sda#x",
            "-/dev/sda",
        ] {
            assert!(check_disk(bad).is_err(), "{bad:?}");
        }

        for bad in [
            "/srv/../etc",
            "/srv/a b",
            "/srv/a;b",
            "/srv/$HOME",
            "/srv/a\nb",
            "rel",
        ] {
            assert!(check_local_repo(bad).is_err(), "{bad:?}");
            assert!(check_repo_file(&format!("{bad}.repo")).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ssh_keys_have_a_known_type_base64_data_and_no_options() {
        for good in [
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA alice@host",
            "ssh-rsa AAAAB3NzaC1yc2EAAAADAQAB",
            "ecdsa-sha2-nistp256 AAAAE2VjZHNh+/= note with spaces",
        ] {
            assert!(check_ssh_key(good).is_ok(), "{good}");
        }

        for bad in [
            "",
            "ssh-ed25519",
            "command=\"reboot\" ssh-ed25519 AAAA x",
            "from=\"*\",ssh-ed25519 AAAA",
            "ssh-ed25519 AAAA\nssh-ed25519 BBBB",
            "ssh-ed25519 AA;AA",
            "ssh-dss AAAA",
            "ssh-ed25519 AAAA c\u{7}",
        ] {
            assert!(check_ssh_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn passwords_and_passphrases_refuse_control_characters_and_absurd_length() {
        for bad in ["a\nb", "a\0b", "a\rb", "a\tb", ""] {
            assert!(check_password(bad).is_err(), "{bad:?}");
            assert!(check_passphrase(bad).is_err(), "{bad:?}");
        }

        assert!(check_password(&"x".repeat(1025)).is_err());
        assert!(check_passphrase("пароль с пробелами и юникодом").is_ok());
        assert!(check_password_hash(&format!("$6$s${}", "a".repeat(600))).is_err());
        assert!(check_password_hash("$6$salt$ha sh").is_err());
        assert!(check_password_hash("$6$salt:$hash").is_err());
    }

    #[test]
    fn repositories_packages_and_options_are_limited_in_size_and_alphabet() {
        assert!(check_repo_id(&"a".repeat(129)).is_err());
        assert!(check_repo_id("a b").is_err());
        assert!(check_package(&"a".repeat(256)).is_err());
        assert!(check_package("vim;reboot").is_err());
        assert!(check_package("$(id)").is_err());
        assert!(check_package("--installroot=/").is_err());
        assert!(check_option("exclude=kernel*").is_ok());
        assert!(check_option("proxy=http://h:3128").is_ok());

        for bad in [
            "k=$(id)", "k=a b", "k=`id`", "k=a;b", "k=a|b", "k=a\nb", "=v", "k",
        ] {
            assert!(check_option(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn local_repository_paths_are_absolute_without_spaces_or_commas() {
        for good in ["/srv/repo", "/mnt/usb/rpms"] {
            assert!(check_local_repo(good).is_ok(), "{good}");
        }

        for bad in ["repo", "/srv/my repo", "/srv/a,b", ""] {
            assert!(check_local_repo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_package_source_is_validated() {
        let packages =
            |repos: &[&str], names: &[&str], version: Option<&str>, copy: &[&str]| Source {
                repos: repos.iter().map(ToString::to_string).collect(),
                local: Vec::new(),
                packages: names.iter().map(ToString::to_string).collect(),
                options: version
                    .map(|version| format!("releasever={version}"))
                    .into_iter()
                    .collect(),
                copy: copy.iter().map(ToString::to_string).collect(),
                ..Source::default()
            };

        assert!(
            packages(
                &["baseos"],
                &["vim", "@core"],
                Some("44"),
                &["/etc/yum.repos.d/a.repo"]
            )
            .validate()
            .is_ok()
        );
        assert!(packages(&[], &[], None, &[]).validate().is_err());
        assert!(packages(&["a b"], &[], None, &[]).validate().is_err());
        assert!(packages(&["x"], &["-y"], None, &[]).validate().is_err());
        assert!(packages(&["x"], &["a;rm"], None, &[]).validate().is_err());
        assert!(packages(&["x"], &[], Some("4 4"), &[]).validate().is_err());
        assert!(packages(&["x"], &[], None, &["a.repo"]).validate().is_err());
        assert!(
            packages(&["x"], &[], None, &["/etc/passwd"])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn the_fat_label_has_the_fat_format() {
        let mut ids = Ids::generate(&config().layout, Uuid::nil);

        ids.fat = 0xABCD_1234;
        assert_eq!(ids.fat_label(), "ABCD-1234");
    }

    #[test]
    fn array_uuids_use_the_mdadm_format() {
        let layout = Layout::from_rows(&[
            "partition disk=/dev/sda,size=rest,point=raid:md0".to_string(),
            "partition disk=/dev/sdb,size=rest,point=raid:md0".to_string(),
            "raid name=md0,level=1,point=/".to_string(),
        ])
        .unwrap();
        let mut counter = 0;
        let ids = Ids::generate(&layout, || {
            counter += 1;
            Uuid::from_u128(counter << 96 | 0xAB)
        });

        assert_eq!(ids.array("md0").len(), 35);
        assert_eq!(ids.array("md0").matches(':').count(), 3);
        assert!(ids.array("md0").ends_with(":000000ab"));
        assert_ne!(ids.device("/dev/sda1"), ids.device("/dev/sdb1"));
        assert_eq!(ids.device("/dev/none"), Uuid::nil());
    }
}
