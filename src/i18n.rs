/// Строка на текущем языке: `tr!("English {name}", "Русский {name}")`; аргументы как в `format!`.
macro_rules! tr {
    ($en:literal, $ru:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
            $crate::i18n::Lang::Ru => format!($ru $(, $arg)*),
        }
    };
}

/// То же без подстановок и без выделения памяти: годится там, где нужен `&'static str`.
macro_rules! tt {
    ($en:expr, $ru:expr) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::En => $en,
            $crate::i18n::Lang::Ru => $ru,
        }
    };
}

use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    static FORCED: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(Some(Lang::Ru)) };
}

impl Lang {
    pub fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "en" => Ok(Self::En),
            "ru" => Ok(Self::Ru),
            other => Err(Error::Invalid(tr!(
                "unknown language: {other} (en or ru)",
                "неизвестный язык: {other} (en или ru)"
            ))),
        }
    }

    /// Первая непустая из `LC_ALL`, `LC_MESSAGES`, `LANG` решает: `ru…` — русский, остальное — английский.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Self {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| env(name))
            .find(|value| !value.is_empty());

        match locale {
            Some(locale) if locale.starts_with("ru") => Self::Ru,
            _ => Self::En,
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::En => 1,
            Self::Ru => 2,
        }
    }
}

/// Язык явно задан флагом, `MOONSHINE_LANG` или параметром ядра; иначе определяется по окружению.
pub fn init(explicit: Option<Lang>) {
    let lang = explicit.unwrap_or_else(|| {
        std::env::var("MOONSHINE_LANG")
            .ok()
            .and_then(|text| Lang::parse(&text).ok())
            .unwrap_or_else(|| Lang::detect(|name| std::env::var(name).ok()))
    });

    CURRENT.store(lang.code(), Ordering::Relaxed);
}

#[cfg(not(test))]
pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        2 => Lang::Ru,
        _ => Lang::En,
    }
}

/// В тестах по умолчанию русский: на нём написаны проверки текстов; `with` включает другой язык.
#[cfg(test)]
pub fn lang() -> Lang {
    FORCED.with(std::cell::Cell::get).unwrap_or(Lang::Ru)
}

#[cfg(test)]
pub fn with<T>(lang: Lang, body: impl FnOnce() -> T) -> T {
    let before = FORCED.with(|forced| forced.replace(Some(lang)));
    let result = body();

    FORCED.with(|forced| forced.set(before));
    result
}

pub(crate) use tr;
pub(crate) use tt;

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::layout::Partition;
    use crate::layout::Raid;
    use crate::layout::Volume;
    use crate::options::Options;
    use crate::testing;

    pub fn cyrillic(text: &str) -> bool {
        text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
    }

    fn arguments(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn russian_is_chosen_only_by_a_russian_locale() {
        assert_eq!(Lang::detect(env(&[("LANG", "ru_RU.UTF-8")])), Lang::Ru);
        assert_eq!(Lang::detect(env(&[("LC_ALL", "ru_UA.UTF-8")])), Lang::Ru);
        assert_eq!(Lang::detect(env(&[("LANG", "en_US.UTF-8")])), Lang::En);
        assert_eq!(Lang::detect(env(&[("LANG", "C")])), Lang::En);
        assert_eq!(Lang::detect(env(&[])), Lang::En);
    }

    #[test]
    fn the_first_non_empty_variable_wins() {
        let pairs = [("LC_ALL", ""), ("LC_MESSAGES", "en_US"), ("LANG", "ru_RU")];

        assert_eq!(Lang::detect(env(&pairs)), Lang::En);
        assert_eq!(
            Lang::detect(env(&[("LC_ALL", "en_US"), ("LANG", "ru_RU")])),
            Lang::En
        );
    }

    #[test]
    fn codes_parse_and_anything_else_is_refused() {
        assert_eq!(Lang::parse("en").unwrap(), Lang::En);
        assert_eq!(Lang::parse("ru").unwrap(), Lang::Ru);
        assert!(Lang::parse("de").is_err());
    }

    #[test]
    fn the_macros_follow_the_language_and_substitute_arguments() {
        let name = "sda";

        assert_eq!(
            with(Lang::En, || tr!("disk {name}", "диск {name}")),
            "disk sda"
        );
        assert_eq!(tr!("disk {}", "диск {}", name), "диск sda");
        assert_eq!(with(Lang::En, || tt!("yes", "да")), "yes");
        assert_eq!(tt!("yes", "да"), "да");
    }

    #[test]
    fn english_usage_and_parse_errors_have_no_russian() {
        with(Lang::En, || {
            let mut texts = vec![crate::options::usage()];

            for line in [
                "--bogus",
                "--disk",
                "--disk /dev/sda --hostname h",
                "--disk /dev/sda --hostname h --user u --firmware bios --arch aarch64",
                "--disk /dev/sda --hostname h --user u --initrd-network manual",
                "--lang xx",
            ] {
                texts.extend(Options::parse(arguments(line)).err().map(|e| e.to_string()));
            }

            texts.extend(
                Partition::parse("disk=/dev/sda")
                    .err()
                    .map(|e| e.to_string()),
            );
            texts.extend(Raid::parse("name=md0,level=7").err().map(|e| e.to_string()));
            texts.extend(
                Volume::parse("vg=a,name=b,size=1x")
                    .err()
                    .map(|e| e.to_string()),
            );

            for text in texts {
                assert!(!cyrillic(&text), "{text}");
            }
        });
    }

    #[test]
    fn english_plan_steps_have_no_russian() {
        with(Lang::En, || {
            let config = testing::config();
            let steps = crate::plan::installation(&config, &testing::ids(), |_| false);

            assert!(
                steps
                    .iter()
                    .any(|step| step.to_string().starts_with("write "))
            );
            assert!(
                steps
                    .iter()
                    .any(|step| step.to_string().starts_with("release disks"))
            );

            for step in steps {
                assert!(!cyrillic(&step.to_string()), "{step}");
            }
        });
    }

    #[test]
    fn config_errors_are_translated() {
        with(Lang::En, || {
            let mut config = testing::config();

            config.hostname = "-bad".into();

            let error = config.validate().unwrap_err().to_string();

            assert_eq!(error, "invalid hostname: -bad");
        });
        assert_eq!(
            Lang::parse("xx").unwrap_err().to_string(),
            "неизвестный язык: xx (en или ru)"
        );
    }
}
