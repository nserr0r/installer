use std::fs;
use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use toml::Table;
use toml::Value;

use crate::error::Error;
use crate::i18n::tr;
use crate::i18n::tt;

/// Где лежит файл установки.
#[derive(Debug, PartialEq, Eq)]
pub enum Location {
    File(PathBuf),
    Http(String),
    Git {
        url: String,
        reference: String,
        path: String,
    },
}

const MAX_CONFIG_SIZE: u64 = 1 << 20;
const GIT_SCHEMES: [&str; 4] = ["https://", "http://", "ssh://", "git://"];

fn is_plain(text: &str, extra: &str) -> bool {
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

/// Адрес для `curl`/`git`: известная схема, без пробелов и управляющих символов,
/// имя хоста не начинается с «-» (иначе его примут за параметр).
fn check_url(url: &str, schemes: &[&str]) -> Result<(), Error> {
    let rest = schemes.iter().find_map(|scheme| url.strip_prefix(scheme));
    let ok = rest.is_some_and(|rest| !rest.is_empty() && !rest.starts_with(['-', '/']))
        && url.len() <= 2048
        && !url.contains(|c: char| c.is_whitespace() || c.is_control());

    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "invalid address: {url}",
            "неверный адрес: {url}"
        )))
    }
}

fn check_reference(reference: &str) -> Result<(), Error> {
    if !reference.is_empty()
        && reference.len() <= 255
        && !reference.starts_with('-')
        && !reference.contains("..")
        && is_plain(reference, "._/-@^~")
    {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "git: invalid branch, tag or commit: {reference}",
            "git: неверная ветка, тег или коммит: {reference}"
        )))
    }
}

impl Location {
    /// `путь`, `http(s)://…` или `git+URL#[ветка-тег-или-коммит:]путь/в/репозитории.toml`.
    pub fn parse(text: &str) -> Result<Self, Error> {
        if let Some(rest) = text.strip_prefix("git+") {
            let (url, fragment) = rest.split_once('#').ok_or_else(|| {
                Error::Invalid(tr!(
                    "git: a file path is required after #: {text}",
                    "git: после # нужен путь к файлу: {text}"
                ))
            })?;
            let (reference, path) = fragment.split_once(':').unwrap_or(("HEAD", fragment));

            check_url(url, &GIT_SCHEMES)?;
            check_reference(reference)?;
            check_repository_path(path)?;

            return Ok(Self::Git {
                url: url.into(),
                reference: reference.into(),
                path: path.into(),
            });
        }

        if text.starts_with("http://") || text.starts_with("https://") {
            check_url(text, &["https://", "http://"])?;

            return Ok(Self::Http(text.into()));
        }

        if text.is_empty() {
            return Err(Error::Invalid(
                tt!("no installation file given", "не указан файл установки").into(),
            ));
        }

        Ok(Self::File(PathBuf::from(text)))
    }

    pub fn read(&self) -> Result<String, Error> {
        match self {
            Self::File(path) => read_limited(path),
            Self::Http(url) => output(
                Command::new("curl").args([
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--netrc-optional",
                    "--proto",
                    "=http,https",
                    "--proto-redir",
                    "=http,https",
                    "--max-filesize",
                    "1048576",
                    "--max-time",
                    "60",
                    "--url",
                    url,
                ]),
                url,
            ),
            Self::Git {
                url,
                reference,
                path,
            } => read_from_git(url, reference, path),
        }
    }
}

fn check_repository_path(path: &str) -> Result<(), Error> {
    let ok = !path.is_empty()
        && path.len() <= 255
        && is_plain(path, "/._-+@")
        && !path.starts_with('/')
        && path.split('/').all(|part| !part.is_empty() && part != "..");

    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(tr!(
            "git: the path inside the repository must be relative and without \"..\": {path}",
            "git: путь внутри репозитория должен быть относительным без «..»: {path}"
        )))
    }
}

fn output(command: &mut Command, what: &str) -> Result<String, Error> {
    let result = command
        .output()
        .map_err(|error| Error::Invalid(format!("{what}: {error}")))?;

    if !result.status.success() {
        return Err(Error::Invalid(format!(
            "{what}: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn read_from_git(url: &str, reference: &str, path: &str) -> Result<String, Error> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    let directory =
        std::env::temp_dir().join(format!("moonshine-config-{}-{nanos}", std::process::id()));

    DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|error| Error::Invalid(format!("{}: {error}", directory.display())))?;

    let result = fetch_file(&directory, url, reference, path);

    let _ = fs::remove_dir_all(&directory);
    result
}

fn fetch_file(directory: &Path, url: &str, reference: &str, path: &str) -> Result<String, Error> {
    let repository = directory.to_string_lossy();

    for args in [
        vec!["init", "--quiet", &repository],
        vec![
            "-C",
            &repository,
            "fetch",
            "--quiet",
            "--depth",
            "1",
            url,
            reference,
        ],
        vec![
            "-c",
            "core.symlinks=false",
            "-C",
            &repository,
            "checkout",
            "--quiet",
            "FETCH_HEAD",
        ],
    ] {
        output(
            Command::new("git")
                .env("GIT_ALLOW_PROTOCOL", "https:http:ssh:git")
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(&args),
            url,
        )?;
    }

    let file = directory.join(path);
    let real = fs::canonicalize(&file)
        .map_err(|error| Error::Invalid(format!("{url}: {path}: {error}")))?;
    let root =
        fs::canonicalize(directory).map_err(|error| Error::Invalid(format!("{url}: {error}")))?;

    if !real.starts_with(&root) {
        return Err(Error::Invalid(tr!(
            "{url}: {path}: the link leads outside the repository",
            "{url}: {path}: ссылка выводит за пределы репозитория"
        )));
    }

    read_limited(&real)
}

fn read_limited(path: &Path) -> Result<String, Error> {
    let invalid = |error: std::io::Error| Error::Invalid(format!("{}: {error}", path.display()));
    let size = fs::metadata(path).map_err(invalid)?.len();

    if size > MAX_CONFIG_SIZE {
        return Err(Error::Invalid(tr!(
            "{}: the file is larger than {MAX_CONFIG_SIZE} bytes",
            "{}: файл больше {MAX_CONFIG_SIZE} байт",
            path.display()
        )));
    }

    fs::read_to_string(path).map_err(invalid)
}

/// Превращает файл TOML в аргументы командной строки: ключ `a_b` — флаг `--a-b`,
/// массив повторяет флаг, `true` ставит его без значения, а таблица `[initrd]` даёт префикс `--initrd-`.
pub fn arguments(text: &str) -> Result<Vec<String>, Error> {
    let table: Table = text.parse().map_err(|error| {
        Error::Invalid(tr!("installation file: {error}", "файл установки: {error}"))
    })?;
    let mut args = Vec::new();

    for (key, value) in &table {
        match value {
            Value::Array(users) if key == "users" => {
                for user in users {
                    args.extend(["--user".to_string(), user_spec(user)?]);
                }
            }
            Value::Array(items) if SPEC_TABLES.contains(&key.as_str()) => {
                for item in items {
                    args.extend([format!("--{key}"), layout_spec(key, item)?]);
                }
            }
            Value::Table(inner) => {
                for (name, value) in inner {
                    push(&mut args, &format!("{key}_{name}"), value)?;
                }
            }
            _ => push(&mut args, key, value)?,
        }
    }

    Ok(args)
}

const SPEC_TABLES: [&str; 4] = ["partition", "raid", "volume", "repository"];

/// Блок `[[partition]]`, `[[raid]]`, `[[volume]]` или `[[repository]]` превращается в строку `ключ=значение,ключ=значение`.
fn layout_spec(kind: &str, item: &Value) -> Result<String, Error> {
    let broken = || {
        Error::Invalid(tr!(
            "{kind}: tables with strings and numbers are required (repository also takes true and false)",
            "{kind}: нужны таблицы со строками и числами (в repository ещё true и false)"
        ))
    };
    let table = item.as_table().ok_or_else(broken)?;
    let mut pairs = Vec::new();

    for (key, value) in table {
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Integer(number) => number.to_string(),
            Value::Boolean(flag) if kind == "repository" => u8::from(*flag).to_string(),
            _ => return Err(broken()),
        };

        if key.contains([',', '=']) || text.contains(',') {
            return Err(Error::Invalid(tr!(
                "{kind}: keys and values cannot contain \",\" and \"=\": {key}",
                "{kind}: в ключах и значениях нельзя «,» и «=»: {key}"
            )));
        }

        pairs.push(format!("{key}={text}"));
    }

    Ok(pairs.join(","))
}

/// Блок `[[users]]` с `name` и необязательным `hash` превращается в `имя[:хеш]`.
fn user_spec(user: &Value) -> Result<String, Error> {
    let broken = || {
        Error::Invalid(
            tt!(
                "users: tables with the keys name and hash are required",
                "users: нужны таблицы с ключами name и hash"
            )
            .into(),
        )
    };
    let table = user.as_table().ok_or_else(broken)?;

    if table.keys().any(|key| key != "name" && key != "hash") {
        return Err(broken());
    }

    let name = table
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(broken)?;

    match table.get("hash") {
        None => Ok(name.to_string()),
        Some(hash) => Ok(format!("{name}:{}", hash.as_str().ok_or_else(broken)?)),
    }
}

fn push(args: &mut Vec<String>, key: &str, value: &Value) -> Result<(), Error> {
    let flag = format!("--{}", key.replace('_', "-"));

    match value {
        Value::Boolean(true) => args.push(flag),
        Value::Boolean(false) => {}
        Value::String(text) => args.extend([flag, text.clone()]),
        Value::Integer(number) => args.extend([flag, number.to_string()]),
        Value::Array(items) => {
            for item in items {
                push(args, key, item)?;
            }
        }
        _ => {
            return Err(Error::Invalid(tr!(
                "installation file: {key}: a string, number, boolean or array is expected",
                "файл установки: {key}: ожидается строка, число, логическое значение или массив"
            )));
        }
    }

    Ok(())
}

/// Раскрывает `--config <где>` в аргументы из файла; остальные аргументы идут после них и важнее.
pub fn expand(args: Vec<String>) -> Result<Vec<String>, Error> {
    let Some(position) = args.iter().position(|arg| arg == "--config") else {
        return Ok(args);
    };
    let location = args.get(position + 1).ok_or_else(|| {
        Error::Invalid(tt!("--config needs a value", "у --config нет значения").into())
    })?;
    let mut expanded = arguments(&Location::parse(location)?.read()?)?;

    expanded.extend(
        args.iter()
            .enumerate()
            .filter(|(index, _)| *index != position && *index != position + 1)
            .map(|(_, arg)| arg.clone()),
    );
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::Repository;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn locations_are_told_apart() {
        assert_eq!(
            Location::parse("/etc/a.toml").unwrap(),
            Location::File("/etc/a.toml".into())
        );
        assert_eq!(
            Location::parse("https://h/a.toml").unwrap(),
            Location::Http("https://h/a.toml".into())
        );
        assert!(Location::parse("").is_err());
    }

    #[test]
    fn a_git_location_has_a_default_reference_and_an_optional_one() {
        assert_eq!(
            Location::parse("git+https://h/r.git#m/a.toml").unwrap(),
            Location::Git {
                url: "https://h/r.git".into(),
                reference: "HEAD".into(),
                path: "m/a.toml".into()
            }
        );
        assert_eq!(
            Location::parse("git+ssh://git@h/r.git#v1:a.toml").unwrap(),
            Location::Git {
                url: "ssh://git@h/r.git".into(),
                reference: "v1".into(),
                path: "a.toml".into()
            }
        );
    }

    #[test]
    fn dangerous_git_addresses_and_references_are_refused() {
        for url in [
            "ext::sh -c reboot",
            "ext::sh%20-c%20reboot",
            "file:///etc",
            "--upload-pack=reboot",
            "ssh://-oProxyCommand=reboot/x",
            "https://",
            "https:///x",
            "https://h/a b",
            "fd::3",
            "https://h/\nx",
        ] {
            assert!(
                Location::parse(&format!("git+{url}#a.toml")).is_err(),
                "{url:?}"
            );
        }

        for reference in ["-x", "--upload-pack=x", "a..b", "a b", "a;b", "a$(id)"] {
            assert!(
                Location::parse(&format!("git+https://h/r.git#{reference}:a.toml")).is_err(),
                "{reference:?}"
            );
        }

        assert!(Location::parse("git+https://h/r.git#refs/tags/v1.0:a.toml").is_ok());
        assert!(Location::parse("git+ssh://git@h/r.git#0123abcd:a.toml").is_ok());
        assert!(Location::parse("git+https://h/r.git#a b.toml").is_err());
    }

    #[test]
    fn web_addresses_must_be_plain_http_urls() {
        assert!(Location::parse("https://evillin.online/a.toml").is_ok());

        for bad in [
            "https://",
            "https:///x",
            "http://h/a b",
            "https://-h/x",
            "https://h/\nx",
        ] {
            assert!(Location::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_oversized_config_file_is_refused() {
        let path = std::env::temp_dir().join(format!("moonshine-big-{}.toml", std::process::id()));

        fs::write(&path, vec![b'#'; (MAX_CONFIG_SIZE + 1) as usize]).unwrap();

        assert!(Location::File(path.clone()).read().is_err());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_config_cannot_smuggle_in_other_commands() {
        for text in [
            "config = \"/etc/passwd\"\n",
            "help = true\n",
            "disk = \"/dev/sda; reboot\"\n",
        ] {
            let args = arguments(text).unwrap();

            assert!(crate::options::Options::parse(args).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_git_path_cannot_leave_the_repository() {
        for bad in [
            "../x", "/etc/x", "a//b", "a/../b", "a b", "a;b", "a\nb", "$HOME/x",
        ] {
            assert!(
                Location::parse(&format!("git+https://h/r#{bad}")).is_err(),
                "{bad}"
            );
        }
        assert!(Location::parse("git+https://h/r").is_err());
    }

    #[test]
    fn services_come_from_an_array() {
        let args = arguments("service = [\"sshd\", \"chronyd\"]\n").unwrap();

        assert_eq!(
            args,
            strings(&["--service", "sshd", "--service", "chronyd"])
        );
    }

    #[test]
    fn kernel_arguments_come_from_an_array() {
        let args = arguments("kernel_arg = [\"quiet\", \"console=ttyS0,115200\"]\n").unwrap();

        assert_eq!(
            args,
            strings(&[
                "--kernel-arg",
                "quiet",
                "--kernel-arg",
                "console=ttyS0,115200"
            ])
        );
    }

    #[test]
    fn keys_become_flags_arrays_repeat_and_tables_add_a_prefix() {
        let args = arguments(
            "disk = \"/dev/sda\"\nrepo = [\"a\", \"b\"]\nkeyfile = true\nno_encryption = false\n\
             uboot_offset = 64\n[initrd]\nnetwork = \"dhcp\"\n",
        )
        .unwrap();

        assert_eq!(
            args,
            strings(&[
                "--disk",
                "/dev/sda",
                "--initrd-network",
                "dhcp",
                "--keyfile",
                "--repo",
                "a",
                "--repo",
                "b",
                "--uboot-offset",
                "64",
            ])
        );
    }

    #[test]
    fn users_blocks_become_user_flags_with_an_optional_hash() {
        let args = arguments(
            "[[users]]\nname = \"alice\"\nhash = \"$6$s$h\"\n\n[[users]]\nname = \"bob\"\n",
        )
        .unwrap();

        assert_eq!(args, strings(&["--user", "alice:$6$s$h", "--user", "bob"]));
    }

    #[test]
    fn a_users_block_without_a_name_or_with_extra_keys_is_an_error() {
        assert!(arguments("[[users]]\nhash = \"x\"\n").is_err());
        assert!(arguments("[[users]]\nname = \"a\"\nshell = \"sh\"\n").is_err());
        assert!(arguments("users = [\"a\"]\n").is_err());
    }

    #[test]
    fn a_broken_file_and_unsupported_values_are_errors() {
        assert!(arguments("disk = ").is_err());
        assert!(arguments("when = 1979-05-27T07:32:00Z").is_err());
        assert!(arguments("x = 1.5").is_err());
    }

    #[test]
    fn other_arguments_follow_the_file_and_without_config_nothing_changes() {
        let path =
            std::env::temp_dir().join(format!("moonshine-manifest-{}.toml", std::process::id()));

        fs::write(&path, "hostname = \"a\"\n").unwrap();

        let args = expand(strings(&["--config", path.to_str().unwrap(), "--dry-run"])).unwrap();

        assert_eq!(args, strings(&["--hostname", "a", "--dry-run"]));
        assert_eq!(expand(strings(&["--yes"])).unwrap(), strings(&["--yes"]));
        assert!(expand(strings(&["--config"])).is_err());
    }

    const STORAGE: &str = r#"
hostname = "moon"
firmware = "uefi"
repo = ["evillin"]

[[users]]
name = "alice"
hash = "$6$salt$hashhash"

[[partition]]
disk = "/dev/sda"
size = "1GiB"
point  = "boot"

[[partition]]
disk = "/dev/sda"
size = "rest"
point  = "raid:md0"

[[partition]]
disk = "/dev/sdb"
size = "whole"
point  = "raid:md0"

[[raid]]
name  = "md0"
level = 1
point   = "lvm:vg"

[[volume]]
vg   = "vg"
name = "root"
size = "rest"
point  = "/"
fs   = "xfs"
"#;

    #[test]
    fn storage_tables_become_layout_flags_in_order() {
        let args = arguments(STORAGE).unwrap();
        let flags: Vec<&String> = args
            .iter()
            .filter(|arg| {
                arg.starts_with("--partition")
                    || arg.starts_with("--raid")
                    || arg.starts_with("--volume")
            })
            .collect();

        assert_eq!(
            flags,
            [
                "--partition",
                "--partition",
                "--partition",
                "--raid",
                "--volume"
            ]
        );
        assert!(args.contains(&"disk=/dev/sda,point=boot,size=1GiB".to_string()));
        assert!(args.contains(&"level=1,name=md0,point=lvm:vg".to_string()));
    }

    #[test]
    fn a_storage_manifest_makes_a_valid_layout() {
        let options = crate::options::Options::parse(arguments(STORAGE).unwrap()).unwrap();

        assert!(options.layout.validate(options.firmware).is_ok());
        assert_eq!(options.layout.partitions.len(), 3);
        assert_eq!(options.layout.raids.len(), 1);
        assert_eq!(options.layout.volumes.len(), 1);
        assert_eq!(options.layout.root_device(), "/dev/vg/root");
    }

    #[test]
    fn encryption_keys_of_a_partition_table_reach_the_layout() {
        let text = r#"
hostname = "moon"
firmware = "uefi"
repo = ["evillin"]

[[users]]
name = "alice"
hash = "$6$salt$hashhash"

[[partition]]
disk = "/dev/sda"
size = "1GiB"
point  = "boot"

[[partition]]
disk   = "/dev/sda"
size   = "rest"
point    = "/"
crypt  = "key"
header = "detached"

[[partition]]
disk   = "/dev/sdb"
size   = "whole"
point    = "/mnt/data"
crypt  = "key"
header = "detached"
"#;
        let options = crate::options::Options::parse(arguments(text).unwrap()).unwrap();

        assert!(options.layout.validate(options.firmware).is_ok());
        assert_eq!(options.layout.crypts().len(), 2);
        assert_eq!(options.layout.early(), ["/dev/sda2"]);
    }

    #[test]
    fn repository_tables_become_repository_flags_and_take_booleans() {
        let text = r#"
hostname = "moon"
repo_priority = ["evillin=5"]
bare = true
package = ["bash"]
repo = ["evillin"]

[[users]]
name = "alice"

[[repository]]
id       = "rocky"
baseurl  = "https://m/$releasever"
priority = 10
gpgcheck = false
"#;
        let args = arguments(text).unwrap();

        assert!(args.contains(&"--bare".to_string()));
        assert!(args.contains(&"evillin=5".to_string()));
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--repository"
                && pair[1] == "baseurl=https://m/$releasever,gpgcheck=0,id=rocky,priority=10"
        }));
        assert!(
            Repository::parse("baseurl=https://m/$releasever,gpgcheck=0,id=rocky,priority=10")
                .is_ok()
        );
    }

    #[test]
    fn storage_tables_refuse_smuggled_keys_and_odd_values() {
        for text in [
            "[[partition]]\ndisk = \"/dev/sda\"\nsize = \"1GiB,point=/\"\npoint = \"swap\"\n",
            "[[partition]]\ndisk = \"/dev/sda\"\nsize = \"1GiB\"\nuse = [\"swap\"]\n",
            "[[partition]]\n\"a,b\" = \"1\"\n",
            "[[partition]]\n\"a=b\" = \"1\"\n",
            "[[raid]]\nname = \"md0\"\nlevel = true\n",
            "partition = [\"x\"]\n",
        ] {
            assert!(arguments(text).is_err(), "{text:?}");
        }

        let unknown = arguments("[[partition]]\ndisk = \"/dev/sda\"\nsize = \"1GiB\"\npoint = \"swap\"\ncolor = \"red\"\n").unwrap();

        assert!(crate::options::Options::parse(unknown).is_err());
    }
}
