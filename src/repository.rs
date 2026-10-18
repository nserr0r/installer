use crate::config::check_repo_id;
use crate::error::Error;
use crate::i18n::tr;
use crate::i18n::tt;

/// Где на установщике лежат `.repo`-файлы, которые читает только `dnf` во время установки.
pub const STAGE_DIR: &str = "/run/moonshine/repos.d";

pub const MIN_PRIORITY: u32 = 1;
pub const MAX_PRIORITY: u32 = 99;

const MAX_TEXT: usize = 2048;
const KEYS: [&str; 10] = [
    "id",
    "name",
    "baseurl",
    "metalink",
    "mirrorlist",
    "priority",
    "gpgcheck",
    "gpgkey",
    "exclude",
    "includepkgs",
];

/// Откуда `dnf` берёт метаданные репозитория.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Baseurl(String),
    Metalink(String),
    Mirrorlist(String),
}

/// Репозиторий, описанный в конфиге; он нужен и при установке, и остаётся в `/etc/yum.repos.d` системы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub id: String,
    pub name: String,
    pub origin: Origin,
    /// Как в yum: чем меньше число, тем выше приоритет; без значения действует умолчание dnf (99).
    pub priority: Option<u32>,
    pub gpgcheck: bool,
    pub gpgkey: Option<String>,
    pub exclude: Vec<String>,
    pub includepkgs: Vec<String>,
}

impl Repository {
    /// Разбирает строку `id=…,baseurl=…,priority=…`; запятая разделяет пары, поэтому в значениях её нет.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut pairs: Vec<(&str, &str)> = Vec::new();

        for pair in text.split(',') {
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                invalid(tr!(
                    "key=value expected: {pair}",
                    "нужен вид ключ=значение: {pair}"
                ))
            })?;

            if !KEYS.contains(&key) {
                return Err(invalid(tr!(
                    "unknown key {key} (available: {})",
                    "неизвестный ключ {key} (есть {})",
                    KEYS.join(", ")
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

        let get = |key: &str| {
            pairs
                .iter()
                .find(|(known, _)| *known == key)
                .map(|(_, value)| *value)
        };
        let id = get("id").ok_or_else(|| invalid(tt!("key id is required", "нужен ключ id")))?;

        check_repo_id(id)?;

        let origin = origin(get("baseurl"), get("metalink"), get("mirrorlist"))?;
        let gpgcheck = get("gpgcheck").map_or(Ok(true), flag)?;
        let gpgkey = get("gpgkey")
            .map(|url| {
                check_url("gpgkey", url, &["https://", "http://", "file://"])
                    .map(|()| url.to_string())
            })
            .transpose()?;

        if gpgcheck && gpgkey.is_none() {
            return Err(invalid(tr!(
                "{id}: signature checking is on but gpgkey is not set: set gpgkey or gpgcheck=0",
                "{id}: проверка подписей включена, а gpgkey не указан: задайте gpgkey или gpgcheck=0"
            )));
        }

        let name = get("name").unwrap_or(id);

        if name.is_empty() || name.len() > MAX_TEXT || name.chars().any(char::is_control) {
            return Err(invalid(tr!(
                "{id}: invalid name: {name}",
                "{id}: неверное имя: {name}"
            )));
        }

        Ok(Self {
            id: id.into(),
            name: name.into(),
            origin,
            priority: get("priority").map(priority).transpose()?,
            gpgcheck,
            gpgkey,
            exclude: patterns("exclude", get("exclude"))?,
            includepkgs: patterns("includepkgs", get("includepkgs"))?,
        })
    }

    /// Содержимое `.repo`-файла.
    pub fn file_text(&self) -> String {
        let (key, url) = match &self.origin {
            Origin::Baseurl(url) => ("baseurl", url),
            Origin::Metalink(url) => ("metalink", url),
            Origin::Mirrorlist(url) => ("mirrorlist", url),
        };
        let mut text = format!(
            "[{}]\nname={}\n{key}={url}\nenabled=1\ngpgcheck={}\n",
            self.id,
            self.name,
            u8::from(self.gpgcheck)
        );

        if let Some(gpgkey) = &self.gpgkey {
            text.push_str(&format!("gpgkey={gpgkey}\n"));
        }

        if let Some(priority) = self.priority {
            text.push_str(&format!("priority={priority}\n"));
        }

        for (key, list) in [
            ("exclude", &self.exclude),
            ("includepkgs", &self.includepkgs),
        ] {
            if !list.is_empty() {
                text.push_str(&format!("{key}={}\n", list.join(" ")));
            }
        }

        text
    }
}

/// Приоритет репозитория из хоста: `--repo-priority ID=N`.
pub fn parse_priority(text: &str) -> Result<(String, u32), Error> {
    let (id, value) = text.split_once('=').ok_or_else(|| {
        invalid(tr!(
            "priority: ID=number expected: {text}",
            "приоритет: нужен вид ID=число: {text}"
        ))
    })?;

    check_repo_id(id)?;
    Ok((id.into(), priority(value)?))
}

fn invalid(message: impl std::fmt::Display) -> Error {
    Error::Invalid(tr!("repository: {message}", "репозиторий: {message}"))
}

fn flag(value: &str) -> Result<bool, Error> {
    match value {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        other => Err(invalid(tr!(
            "gpgcheck: 0 or 1 expected: {other}",
            "gpgcheck: нужно 0 или 1: {other}"
        ))),
    }
}

fn priority(value: &str) -> Result<u32, Error> {
    value
        .parse()
        .ok()
        .filter(|number| (MIN_PRIORITY..=MAX_PRIORITY).contains(number))
        .ok_or_else(|| {
            invalid(tr!(
                "priority: a number from {MIN_PRIORITY} to {MAX_PRIORITY}: {value}",
                "приоритет: число от {MIN_PRIORITY} до {MAX_PRIORITY}: {value}"
            ))
        })
}

fn origin(
    baseurl: Option<&str>,
    metalink: Option<&str>,
    mirrorlist: Option<&str>,
) -> Result<Origin, Error> {
    let web = ["https://", "http://"];
    let any = ["https://", "http://", "ftp://", "file://"];

    match (baseurl, metalink, mirrorlist) {
        (Some(url), None, None) => {
            check_url("baseurl", url, &any)?;
            Ok(Origin::Baseurl(url.into()))
        }
        (None, Some(url), None) => {
            check_url("metalink", url, &web)?;
            Ok(Origin::Metalink(url.into()))
        }
        (None, None, Some(url)) => {
            check_url("mirrorlist", url, &web)?;
            Ok(Origin::Mirrorlist(url.into()))
        }
        _ => Err(invalid(tt!(
            "exactly one of baseurl, metalink, mirrorlist is required",
            "нужен ровно один из ключей baseurl, metalink, mirrorlist"
        ))),
    }
}

/// Адрес попадает в `.repo`-файл: схема из списка, без пробелов и управляющих символов; `$releasever` и `$basearch` допустимы.
fn check_url(key: &str, url: &str, schemes: &[&str]) -> Result<(), Error> {
    let located = schemes.iter().find_map(|scheme| {
        url.strip_prefix(scheme)
            .map(|rest| (*scheme == "file://", rest))
    });
    let ok = located.is_some_and(|(local, rest)| {
        !rest.is_empty() && !rest.starts_with('-') && rest.starts_with('/') == local
    }) && url.len() <= MAX_TEXT
        && !url.contains(|c: char| c.is_whitespace() || c.is_control());

    if ok {
        Ok(())
    } else {
        Err(invalid(tr!(
            "{key}: invalid address: {url}",
            "{key}: неверный адрес: {url}"
        )))
    }
}

fn patterns(key: &str, value: Option<&str>) -> Result<Vec<String>, Error> {
    let list: Vec<String> = value
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();

    if list.iter().all(|item| {
        !item.starts_with('-')
            && item
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-+*?:".contains(c))
    }) {
        Ok(list)
    } else {
        Err(invalid(tr!(
            "{key}: invalid package pattern",
            "{key}: неверный шаблон пакета"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "id=rocky-baseos,name=Rocky BaseOS,baseurl=https://dl.rockylinux.org/pub/rocky/$releasever/BaseOS/$basearch/os/,priority=10,gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky";

    #[test]
    fn a_full_definition_is_read() {
        let repo = Repository::parse(FULL).unwrap();

        assert_eq!(repo.id, "rocky-baseos");
        assert_eq!(repo.name, "Rocky BaseOS");
        assert_eq!(repo.priority, Some(10));
        assert!(repo.gpgcheck);
        assert!(matches!(repo.origin, Origin::Baseurl(ref url) if url.contains("$releasever")));
    }

    #[test]
    fn the_repo_file_has_every_given_field_and_nothing_else() {
        let repo = Repository::parse(FULL).unwrap();

        assert_eq!(
            repo.file_text(),
            "[rocky-baseos]\nname=Rocky BaseOS\nbaseurl=https://dl.rockylinux.org/pub/rocky/$releasever/BaseOS/$basearch/os/\nenabled=1\ngpgcheck=1\ngpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky\npriority=10\n"
        );

        let bare = Repository::parse("id=local,baseurl=http://mirror/x,gpgcheck=0").unwrap();

        assert_eq!(bare.name, "local");
        assert_eq!(
            bare.file_text(),
            "[local]\nname=local\nbaseurl=http://mirror/x\nenabled=1\ngpgcheck=0\n"
        );
    }

    #[test]
    fn exclude_and_includepkgs_are_space_separated_lists() {
        let repo = Repository::parse(
            "id=r,metalink=https://m/x,gpgcheck=0,exclude=kernel* grub2*,includepkgs=vim",
        )
        .unwrap();

        assert_eq!(repo.exclude, ["kernel*", "grub2*"]);
        assert!(repo.file_text().contains("exclude=kernel* grub2*\n"));
        assert!(repo.file_text().contains("metalink=https://m/x\n"));
        assert!(repo.file_text().ends_with("includepkgs=vim\n"));
    }

    #[test]
    fn exactly_one_origin_and_a_key_for_signatures_are_required() {
        for bad in [
            "id=r,gpgcheck=0",
            "id=r,baseurl=http://a,metalink=http://b,gpgcheck=0",
            "id=r,baseurl=http://a",
            "baseurl=http://a,gpgcheck=0",
        ] {
            assert!(Repository::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn priorities_are_limited_to_the_yum_range() {
        for (value, ok) in [
            ("1", true),
            ("99", true),
            ("0", false),
            ("100", false),
            ("x", false),
        ] {
            let text = format!("id=r,baseurl=http://a,gpgcheck=0,priority={value}");

            assert_eq!(Repository::parse(&text).is_ok(), ok, "{value}");
        }

        assert_eq!(parse_priority("evillin=5").unwrap(), ("evillin".into(), 5));
        assert!(parse_priority("evillin").is_err());
        assert!(parse_priority("evillin=0").is_err());
        assert!(parse_priority("bad id=5").is_err());
    }

    #[test]
    fn unsafe_values_cannot_reach_the_repo_file() {
        for bad in [
            "id=r,baseurl=http://a b,gpgcheck=0",
            "id=r,baseurl=ftp:/a,gpgcheck=0",
            "id=r,baseurl=-http://a,gpgcheck=0",
            "id=r,baseurl=http://a,gpgcheck=0,name=x\ny",
            "id=r,baseurl=http://a,gpgcheck=maybe",
            "id=r,baseurl=http://a,gpgcheck=0,exclude=-rf",
            "id=r,baseurl=http://a,gpgcheck=0,color=red",
            "id=r,id=s,baseurl=http://a,gpgcheck=0",
            "id=r/../x,baseurl=http://a,gpgcheck=0",
            "id=r,metalink=file:///a,gpgcheck=0",
            "id=r,baseurl=http://a,gpgkey=ftp://k",
        ] {
            assert!(Repository::parse(bad).is_err(), "{bad:?}");
        }
    }
}
