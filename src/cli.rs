use std::io::Write;
use std::io::stdin;
use std::io::stdout;
use std::path::Path;

use crate::check;
use crate::config;
use crate::config::Config;
use crate::config::Ids;
use crate::config::Secret;
use crate::config::User;
use crate::disks;
use crate::error::Error;
use crate::exec;
use crate::i18n;
use crate::i18n::Lang;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::manifest;
use crate::options;
use crate::options::Options;
use crate::plan;
use crate::secrets;
use crate::storage;
use crate::tui;

pub fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    i18n::init(None);
    i18n::init(explicit_lang(&args)?);

    if without_lang(&args).is_empty() {
        return tui::run();
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", options::usage());
        return Ok(());
    }

    match args.split_first() {
        Some((command, rest)) if command == "check" => check(rest.to_vec()),
        _ => install(args),
    }
}

fn without_lang(args: &[String]) -> Vec<&String> {
    let mut rest = Vec::new();
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        if arg == "--lang" {
            iter.next();
        } else {
            rest.push(arg);
        }
    }

    rest
}

/// Язык из последнего `--lang`: решается до разбора остальных аргументов, ошибки разбора уже должны быть на нём.
fn explicit_lang(args: &[String]) -> Result<Option<Lang>, Error> {
    let Some(index) = args.iter().rposition(|arg| arg == "--lang") else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| Error::Invalid(tr!("--lang needs a value", "у --lang нет значения")))?;

    Lang::parse(value).map(Some)
}

/// `check ЦЕЛЬ [--plan]`: разобрать и проверить конфигурацию, ничего не меняя в системе.
fn check(args: Vec<String>) -> Result<(), Error> {
    let (args, show_plan) = check_arguments(args)?;
    let options = Options::parse(expand(args)?)?;
    let config = read_config(&options, Mode::Check)?;

    config.validate()?;

    for line in check::report(&config, &options) {
        println!("{line}");
    }

    if show_plan {
        println!("\n{}", tt!("Installation plan:", "План установки:"));

        for step in plan::installation(&config, &Ids::random(&config.layout), exec::installed) {
            println!("  {step}");
        }
    }

    Ok(())
}

/// Раскрывает `--config`; `lang` из файла действует, если язык не задан в командной строке.
fn expand(args: Vec<String>) -> Result<Vec<String>, Error> {
    let expanded = manifest::expand(args)?;

    if let Some(lang) = explicit_lang(&expanded)? {
        i18n::init(Some(lang));
    }

    Ok(expanded)
}

/// Первый аргумент без `--` — это `--config`; `--plan` забирается отдельно.
fn check_arguments(args: Vec<String>) -> Result<(Vec<String>, bool), Error> {
    let mut show_plan = false;
    let mut rest = Vec::new();

    for (index, arg) in args.into_iter().enumerate() {
        match arg.as_str() {
            "--plan" => show_plan = true,
            _ if index == 0 && !arg.starts_with("--") => {
                rest.push("--config".to_string());
                rest.push(arg);
            }
            _ => rest.push(arg),
        }
    }

    if rest.is_empty() {
        return Err(Error::Invalid(tr!(
            "check: give a file, https://… or git+URL#[branch:]path.toml\n{}",
            "check: укажите файл, https://… или git+URL#[ветка:]путь.toml\n{}",
            options::usage()
        )));
    }

    Ok((rest, show_plan))
}

fn install(args: Vec<String>) -> Result<(), Error> {
    let options = Options::parse(expand(args)?)?;
    let config = read_config(&options, Mode::Install)?;

    config.validate()?;

    let steps = plan::installation(&config, &Ids::random(&config.layout), exec::installed);

    disks::check_blocks(&config.layout.disks())?;
    config.layout.check_fit(config.firmware, disks::size_of)?;

    let missing = storage::missing(&config.layout, exec::installed);

    if !missing.is_empty() {
        println!(
            "{}",
            tr!(
                "These packages will be added to the installer: {}",
                "На установщик будут доставлены пакеты: {}",
                missing.join(" ")
            )
        );
    }

    if !options.assume_yes && !confirmed(&config) {
        return Err(Error::Invalid(tt!("cancelled", "отменено").into()));
    }

    exec::execute(
        &steps,
        Path::new(plan::TARGET),
        &mut exec::System,
        |index, step| println!("[{}/{}] {step}", index + 1, steps.len()),
    )
}

/// Установка спрашивает пароли и читает файл фразы; проверка подставляет заглушки и ничего не читает.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Install,
    Check,
}

fn read_config(options: &Options, mode: Mode) -> Result<Config, Error> {
    let ask = |prompt: &str| {
        rpassword::prompt_password(prompt)
            .map(Secret::new)
            .map_err(|error| Error::Invalid(tr!("input: {error}", "ввод: {error}")))
    };
    let mut users = Vec::new();

    for spec in &options.users {
        let (password, hashed) = match (&spec.hash, mode) {
            (Some(hash), _) => (Secret::new(hash.as_str()), true),
            (None, Mode::Check) => (Secret::new("-"), false),
            (None, Mode::Install) => (
                ask(&tr!(
                    "Password of user {}: ",
                    "Пароль пользователя {}: ",
                    spec.name
                ))?,
                false,
            ),
        };

        users.push(User {
            name: spec.name.clone(),
            password,
            hashed,
        });
    }

    let passphrase = match (
        options.layout.needs_passphrase(),
        mode,
        &options.passphrase_file,
    ) {
        (false, _, _) => None,
        (true, Mode::Check, _) => Some(Secret::new("-")),
        (true, Mode::Install, Some(path)) => Some(secrets::read_passphrase(path)?),
        (true, Mode::Install, None) => {
            Some(ask(tt!("Disk passphrase: ", "Парольная фраза дисков: "))?)
        }
    };

    Ok(Config {
        layout: options.layout.clone(),
        firmware: options.firmware,
        source: options.source.clone(),
        arch: options.arch,
        cpu: options.cpu,
        hostname: options.hostname.clone(),
        users,
        passphrase,
        keys: config::generate_keys(&options.layout)?,
        uboot: options.uboot.clone(),
        timezone: options.timezone.clone(),
        initrd_network: options.initrd_network.clone(),
        system_network: options.system_network.clone(),
        backend: options.backend,
        ssh_keys: options.ssh_keys.clone(),
        kernel_args: options.kernel_args.clone(),
        services: options.services.clone(),
    })
}

fn confirmed(config: &Config) -> bool {
    let disks = config.layout.disks().join(", ");

    print!(
        "{}",
        tr!(
            "All data on {disks} will be erased. Type YES: ",
            "Все данные на {disks} будут стёрты. Введите YES: "
        )
    );
    let _ = stdout().flush();

    let mut answer = String::new();

    stdin().read_line(&mut answer).is_ok() && answer.trim() == "YES"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_takes_the_location_first_and_the_plan_flag_anywhere() {
        let strings = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            check_arguments(strings(&["moon.toml", "--plan"])).unwrap(),
            (strings(&["--config", "moon.toml"]), true)
        );
        assert_eq!(
            check_arguments(strings(&["--config", "a.toml"])).unwrap(),
            (strings(&["--config", "a.toml"]), false)
        );
        assert!(check_arguments(Vec::new()).is_err());
        assert!(check_arguments(strings(&["--plan"])).is_err());
    }

    #[test]
    fn the_language_flag_alone_leaves_the_wizard_and_the_last_one_wins() {
        let strings = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert!(without_lang(&strings(&["--lang", "en"])).is_empty());
        assert_eq!(
            without_lang(&strings(&["--lang", "en", "--yes"])),
            ["--yes"]
        );
        assert_eq!(
            explicit_lang(&strings(&["--lang", "ru", "--yes", "--lang", "en"])).unwrap(),
            Some(Lang::En)
        );
        assert_eq!(explicit_lang(&strings(&["--yes"])).unwrap(), None);
        assert!(explicit_lang(&strings(&["--lang"])).is_err());
        assert!(explicit_lang(&strings(&["--lang", "de"])).is_err());
    }
}
