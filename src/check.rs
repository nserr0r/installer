use crate::config::Arch;
use crate::config::Config;
use crate::config::Cpu;
use crate::config::Firmware;
use crate::config::ROOT;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::network::Network;
use crate::options::Options;
use crate::secrets;

/// Замечания, которые не мешают установке, но о которых лучше знать заранее.
pub fn warnings(config: &Config, options: &Options) -> Vec<String> {
    let mut notes = Vec::new();

    if !config.users.iter().any(|user| user.name == ROOT) {
        notes.push(
            tt!(
                "root is not listed: it will be locked, log in as a wheel user",
                "root не указан: он будет заблокирован, входить нужно пользователем из wheel"
            )
            .into(),
        );
    }

    for spec in options.users.iter().filter(|spec| spec.hash.is_none()) {
        notes.push(tr!(
            "{} has no hash: the password will be asked on the console, an unattended install needs a hash (openssl passwd -6)",
            "у {} нет хеша: пароль спросят на консоли, для установки без присмотра нужен хеш (openssl passwd -6)",
            spec.name
        ));
    }

    if config.layout.needs_passphrase() {
        match &options.passphrase_file {
            Some(path) => notes.extend(secrets::problem(path)),
            None => notes.push(
                tt!(
                    "the disk passphrase will be asked on the console: for an unattended install use passphrase_file or crypt=key",
                    "парольную фразу дисков спросят на консоли: для установки без присмотра используйте passphrase_file или crypt=key"
                )
                .into(),
            ),
        }
    }

    for (pool, size, promised) in config.layout.overcommitted() {
        notes.push(tr!(
            "pool {pool}: thin volumes are promised {promised}MiB with {size}MiB in the pool, writes will stop when it fills up",
            "пул {pool}: тонким томам обещано {promised}MiB при {size}MiB в пуле, при заполнении запись остановится"
        ));
    }

    if config.system_network == Network::None {
        notes.push(
            tt!(
                "the network of the installed system is not configured",
                "сеть в установленной системе не настроена"
            )
            .into(),
        );
    }

    notes
}

pub fn report(config: &Config, options: &Options) -> Vec<String> {
    let mut lines = vec![
        tt!("The configuration is valid.", "Конфигурация корректна.").to_string(),
        String::new(),
        tr!(
            "Platform:       {} · {} · {}",
            "Платформа:      {} · {} · {}",
            firmware(config.firmware),
            arch(config.arch),
            cpu(config.cpu)
        ),
        tr!("Hostname:       {}", "Имя машины:     {}", config.hostname),
        tr!("Time zone:      {}", "Часовой пояс:   {}", config.timezone),
        tr!("Users:          {}", "Пользователи:   {}", names(config)),
        tr!(
            "Kernel options: {}",
            "Параметры ядра: {}",
            listed(&config.kernel_args)
        ),
        tr!(
            "Services:       {}",
            "Службы:         {}",
            listed(&config.services)
        ),
        String::new(),
        tt!(
            "Layout (disks and their sizes are not read during a check):",
            "Разметка (диски и их размеры при проверке не читаются):"
        )
        .into(),
    ];

    lines.extend(
        config
            .layout
            .describe()
            .into_iter()
            .map(|line| format!("  {line}")),
    );
    lines.push(String::new());
    lines.extend(sources(config));
    lines.push(tr!(
        "Network in initramfs: {}",
        "Сеть в initramfs: {}",
        network(&config.initrd_network)
    ));
    lines.push(tr!(
        "Network in system:    {}",
        "Сеть в системе:   {}",
        network(&config.system_network)
    ));

    let notes = warnings(config, options);

    if !notes.is_empty() {
        lines.push(String::new());
        lines.push(tt!("Notes:", "Замечания:").into());
        lines.extend(notes.into_iter().map(|note| format!("  ! {note}")));
    }

    lines
}

fn listed(items: &[String]) -> String {
    if items.is_empty() {
        tt!("none", "нет").to_string()
    } else {
        items.join(" ")
    }
}

fn names(config: &Config) -> String {
    config
        .users
        .iter()
        .map(|user| user.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn sources(config: &Config) -> Vec<String> {
    let source = &config.source;
    let mut lines = vec![tt!("Package sources:", "Источники пакетов:").to_string()];

    for id in &source.repos {
        let priority = source
            .priorities
            .iter()
            .find(|(known, _)| known == id)
            .map_or(String::new(), |(_, priority)| {
                tr!(" (priority {priority})", " (приоритет {priority})")
            });

        lines.push(tr!(
            "  host repository {id}{priority}",
            "  репозиторий хоста {id}{priority}"
        ));
    }

    for repository in &source.repositories {
        let priority = repository.priority.map_or(String::new(), |priority| {
            tr!(" (priority {priority})", " (приоритет {priority})")
        });

        lines.push(tr!(
            "  config repository {}{priority}",
            "  репозиторий из конфига {}{priority}",
            repository.id
        ));
    }

    for path in &source.local {
        lines.push(tr!(
            "  local directory {path}",
            "  локальный каталог {path}"
        ));
    }

    let set = if source.bare {
        tt!(
            "only the listed packages (--bare)",
            "только указанные пакеты (--bare)"
        )
    } else {
        tt!(
            "built-in set + listed packages",
            "встроенный набор + указанные пакеты"
        )
    };

    lines.push(tr!(
        "  packages: {set}, listed {}",
        "  пакеты: {set}, указано {}",
        source.packages.len()
    ));
    lines
}

fn firmware(firmware: Firmware) -> &'static str {
    match firmware {
        Firmware::Uefi => "UEFI",
        Firmware::Bios => "BIOS",
        Firmware::Hybrid => "BIOS+UEFI",
        Firmware::Uboot => "U-Boot",
    }
}

fn arch(arch: Arch) -> &'static str {
    match arch {
        Arch::X86_64 => "x86_64",
        Arch::Aarch64 => "aarch64",
    }
}

fn cpu(cpu: Cpu) -> &'static str {
    match cpu {
        Cpu::Intel => "Intel",
        Cpu::Amd => "AMD",
        Cpu::Other => tt!("other CPU", "другой процессор"),
    }
}

fn network(network: &Network) -> String {
    match network {
        Network::None => tt!("none", "нет").into(),
        Network::Dhcp { interface } => format!("dhcp{}", on(interface.as_deref())),
        Network::Manual(manual) => tr!(
            "{}/{} via {}{}",
            "{}/{} через {}{}",
            manual.address,
            manual.prefix,
            manual.gateway,
            on(Some(&manual.interface))
        ),
    }
}

fn on(interface: Option<&str>) -> String {
    interface.map_or(String::new(), |name| tr!(" on {name}", " на {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;
    use crate::config::User;
    use crate::layout::Layout;
    use crate::testing::config;

    fn options(line: &str) -> Options {
        Options::parse(line.split_whitespace().map(String::from)).unwrap()
    }

    fn user(name: &str) -> User {
        User {
            name: name.into(),
            password: Secret::new("$6$s$h"),
            hashed: true,
        }
    }

    #[test]
    fn a_complete_unattended_setup_has_no_warnings() {
        let options = options(
            "--disk /dev/sda --hostname h --user u:$6$salt$abcdefghij --repo r --system-network dhcp --no-encryption",
        );
        let config = Config {
            users: vec![user("root"), user("u")],
            system_network: Network::Dhcp { interface: None },
            layout: options.layout.clone(),
            ..config()
        };

        assert!(warnings(&config, &options).is_empty());
    }

    #[test]
    fn missing_root_hashes_the_phrase_and_the_network_are_pointed_out() {
        let options = options("--disk /dev/sda --hostname h --user u --repo r");
        let notes = warnings(&config(), &options).join("\n");

        for expected in [
            "root не указан",
            "у u нет хеша",
            "парольную фразу",
            "сеть в установленной",
        ] {
            assert!(notes.contains(expected), "нет «{expected}»:\n{notes}");
        }
    }

    #[test]
    fn an_unreadable_passphrase_file_is_a_warning_not_an_error() {
        let options = options(
            "--disk /dev/sda --hostname h --user u --repo r --passphrase-file /nonexistent/phrase",
        );
        let notes = warnings(&config(), &options).join("\n");

        assert!(notes.contains("/nonexistent/phrase"), "{notes}");
        assert!(
            !notes.contains("спросят на консоли: для установки"),
            "{notes}"
        );
    }

    #[test]
    fn the_report_lists_kernel_parameters() {
        let options = options("--disk /dev/sda --hostname moon --user u --repo r");
        let mut config = Config {
            layout: options.layout.clone(),
            source: options.source.clone(),
            ..config()
        };

        assert!(
            report(&config, &options)
                .join("\n")
                .contains("Параметры ядра: нет")
        );

        config.kernel_args = vec!["quiet".into(), "nomodeset".into()];
        config.services = vec!["sshd".into(), "chronyd".into()];

        assert!(
            report(&config, &options)
                .join("\n")
                .contains("Параметры ядра: quiet nomodeset")
        );
        assert!(
            report(&config, &options)
                .join("\n")
                .contains("Службы:         sshd chronyd")
        );
    }

    #[test]
    fn the_report_names_platform_layout_sources_and_networks() {
        let options = options(
            "--disk /dev/sda --hostname moon --user u --repo evillin --repo-priority evillin=5 --repository id=c,baseurl=https://rpm.evillin.online/c/,priority=7,gpgcheck=0 --package vim",
        );
        let config = Config {
            hostname: "moon".into(),
            layout: options.layout.clone(),
            source: options.source.clone(),
            ..config()
        };
        let text = report(&config, &options).join("\n");

        for expected in [
            "Конфигурация корректна.",
            "UEFI · x86_64",
            "moon",
            "/dev/sda1",
            "репозиторий хоста evillin (приоритет 5)",
            "репозиторий из конфига c (приоритет 7)",
            "встроенный набор + указанные пакеты, указано 1",
            "Сеть в initramfs: нет",
        ] {
            assert!(text.contains(expected), "нет «{expected}»:\n{text}");
        }
    }

    #[test]
    fn promising_thin_volumes_more_than_the_pool_holds_is_pointed_out() {
        let options = options("--disk /dev/sda --hostname h --user u --repo r");
        let config = Config {
            layout: Layout::from_rows(&[
                "partition disk=/dev/sda,size=1GiB,point=boot".to_string(),
                "partition disk=/dev/sda,size=rest,point=lvm:vg".to_string(),
                "volume vg=vg,name=pool,size=10GiB,point=pool".to_string(),
                "volume vg=vg,name=root,size=20GiB,point=/,pool=pool".to_string(),
            ])
            .unwrap(),
            ..config()
        };

        assert!(
            warnings(&config, &options)
                .join("\n")
                .contains("пул vg/pool")
        );
    }

    #[test]
    fn the_english_report_has_no_russian() {
        crate::i18n::with(crate::i18n::Lang::En, || {
            let options = options("--disk /dev/sda --hostname moon --user u --repo evillin");
            let config = Config {
                layout: options.layout.clone(),
                ..config()
            };
            let report = super::report(&config, &options).join("\n");

            assert!(report.contains("Platform:"), "{report}");
            assert!(report.contains("Notes:"), "{report}");
            assert!(!crate::i18n::tests::cyrillic(&report), "{report}");
        });
    }
}
