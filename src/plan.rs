use std::fmt;

use crate::config::Arch;
use crate::config::Config;
use crate::config::Cpu;
use crate::config::Firmware;
use crate::config::Ids;
use crate::config::ROOT;
use crate::config::Secret;
use crate::config::Source;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::network::Backend;
use crate::repository::STAGE_DIR;
use crate::storage;

pub const TARGET: &str = "/mnt/target";
/// Размер временного файла заголовка LUKS.
pub(crate) const HEADER_SIZE: &str = "16MiB";
pub(crate) const UBOOT_START: &str = "16MiB";
const LUKS_DIR: &str = "/etc/luks";
const KERNEL_CMDLINE: &str = "/etc/kernel/cmdline";
/// Данные пользователя в скрипт не попадают: командная строка ядра читается из файла.
const EXTLINUX_SCRIPT: &str = "version=$(ls /lib/modules | sort -V | tail -n 1)\n\
    mkdir -p /boot/extlinux\n\
    printf 'default moonshine\\ntimeout 30\\nlabel moonshine\\n  kernel /vmlinuz-%s\\n  initrd /initramfs-%s.img\\n  fdtdir /dtb-%s/\\n  append %s\\n' \"$version\" \"$version\" \"$version\" \"$(cat /etc/kernel/cmdline)\" > /boot/extlinux/extlinux.conf\n";
const HOST_REPOS: &str = "/etc/yum.repos.d";
const LOCAL_REPO: &str = "moonshine-local";
const BOOTLOADER_ID: &str = "moonshine";
const GRUB_CONFIG: &str = "/boot/grub2/grub.cfg";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Script(String),
    Secret(Secret),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub input: Option<Input>,
}

impl Command {
    pub fn new<S: Into<String>>(program: &str, args: impl IntoIterator<Item = S>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            input: None,
        }
    }

    pub(crate) fn input(mut self, input: Input) -> Self {
        self.input = Some(input);
        self
    }

    fn in_target<S: Into<String>>(program: &str, args: impl IntoIterator<Item = S>) -> Self {
        let mut command = vec![TARGET.to_string(), program.to_string()];
        command.extend(args.into_iter().map(Into::into));

        Self::new("chroot", command)
    }
}

impl fmt::Display for Command {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.program)?;

        for arg in &self.args {
            write!(formatter, " {arg}")?;
        }

        match &self.input {
            Some(Input::Secret(_)) => write!(formatter, " < <{}>", tt!("secret", "секрет")),
            Some(Input::Script(script)) => write!(formatter, " <<< {script:?}"),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub contents: String,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Run(Command),
    Write(File),
    /// Размонтировать, отключить подкачку и остановить RAID, LVM и LUKS на этих дисках.
    Release(Vec<String>),
}

impl fmt::Display for Step {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Run(command) => write!(formatter, "{command}"),
            Step::Write(file) => write!(
                formatter,
                "{}",
                tr!(
                    "write {TARGET}{} ({:o})",
                    "записать {TARGET}{} ({:o})",
                    file.path,
                    file.mode
                )
            ),
            Step::Release(disks) => write!(
                formatter,
                "{}",
                tr!(
                    "release disks (umount, swapoff, stop RAID/LVM/LUKS): {}",
                    "освободить диски (umount, swapoff, остановка RAID/LVM/LUKS): {}",
                    disks.join(" ")
                )
            ),
        }
    }
}

/// Идёт от самых вложенных устройств к диску: подкачка и файловые системы, затем слои dm и md.
const RELEASE_SCRIPT: &str = r#"set -e
for disk do
  lsblk -nrpo NAME,TYPE,MOUNTPOINT "$disk" | tac | while read -r name type mount; do
    case "$mount" in
      '') ;;
      '[SWAP]') swapoff "$name" ;;
      *) ! mountpoint -q "$mount" || umount -R "$mount" ;;
    esac
    case "$type" in
      lvm|crypt) dmsetup remove "$name" ;;
      raid*) mdadm --stop "$name" ;;
    esac
  done
done
"#;

pub fn release_command(disks: &[String]) -> Command {
    let mut args = vec!["-c".to_string(), RELEASE_SCRIPT.into(), "release".into()];

    args.extend(disks.iter().cloned());
    Command::new("sh", args)
}

/// Полный план: сначала недостающие утилиты установщика, затем сама установка.
pub fn installation(config: &Config, ids: &Ids, present: impl Fn(&str) -> bool) -> Vec<Step> {
    let mut steps = storage::fetch(config, present);

    steps.extend(build(config, ids));
    steps
}

pub fn build(config: &Config, ids: &Ids) -> Vec<Step> {
    let mut steps = Vec::new();

    steps.extend(storage::partitioning(config, ids));
    steps.extend(storage::stacking(config, ids));
    steps.extend(storage::formatting(config, ids));
    steps.extend(storage::mounting(config));
    steps.extend(system_mounts());
    steps.push(file(
        "/etc/kernel/cmdline",
        format!("{}\n", kernel_cmdline(config, ids)),
        0o644,
    ));
    steps.extend(installing(config));
    steps.extend(settings(config, ids));
    steps.extend(accounts(config));
    steps.extend(system_network(config));
    steps.extend(services(config));
    steps.push(Step::Run(Command::in_target(
        "dracut",
        ["--force", "--regenerate-all"],
    )));
    steps.extend(bootloader(config, ids));
    steps.extend(storage::finish(config));

    steps
}

fn installing(config: &Config) -> Vec<Step> {
    let mut steps = stage_repositories(&config.source);

    steps.push(run("dnf", dnf_args(config, TARGET, "--assumeyes")));

    if !config.source.repositories.is_empty() {
        steps.push(run("rm", ["-rf", STAGE_DIR]));
    }

    steps.extend(config.source.repositories.iter().map(|repository| {
        file(
            &format!("/etc/yum.repos.d/{}.repo", repository.id),
            repository.file_text(),
            0o644,
        )
    }));

    if !config.source.copy.is_empty() {
        let directory = format!("{TARGET}/etc/yum.repos.d");

        steps.push(run("mkdir", ["-p", &directory]));
        steps.extend(
            config
                .source
                .copy
                .iter()
                .map(|path| run("cp", [path, &directory])),
        );
    }

    steps
}

/// `.repo`-файлы репозиториев из конфига кладутся в каталог установщика, который читает только `dnf`.
fn stage_repositories(source: &Source) -> Vec<Step> {
    if source.repositories.is_empty() {
        return Vec::new();
    }

    let mut steps = vec![run("mkdir", ["-p", STAGE_DIR])];

    for repository in &source.repositories {
        let path = format!("{STAGE_DIR}/{}.repo", repository.id);

        steps.push(Step::Run(
            Command::new("sh", ["-c", "cat > \"$1\"", "sh", &path])
                .input(Input::Script(repository.file_text())),
        ));
    }

    steps
}

/// Аргументы `dnf install` для установки в `root`; `assume` — `--assumeyes` или `--assumeno`.
/// Локальный каталог подключается как репозиторий `moonshine-localN` без проверки подписей.
pub fn dnf_args(config: &Config, root: &str, assume: &str) -> Vec<String> {
    let Source {
        repos,
        local,
        packages,
        options,
        repositories,
        priorities,
        ..
    } = &config.source;
    let mut args = vec![format!("--installroot={root}")];

    args.extend(options.iter().filter_map(|option| {
        let (key, value) = crate::config::split_option(option).ok()?;

        Some(if key == "releasever" {
            format!("--releasever={value}")
        } else {
            format!("--setopt={key}={value}")
        })
    }));
    args.extend(
        local
            .iter()
            .enumerate()
            .map(|(index, path)| format!("--repofrompath={LOCAL_REPO}{index},file://{path}")),
    );
    args.extend([assume.to_string(), "--disablerepo=*".to_string()]);
    args.extend(repos.iter().map(|id| format!("--enablerepo={id}")));
    args.extend(
        priorities
            .iter()
            .map(|(id, priority)| format!("--setopt={id}.priority={priority}")),
    );

    let reposdir = if repositories.is_empty() {
        HOST_REPOS.to_string()
    } else {
        format!("{HOST_REPOS},{STAGE_DIR}")
    };

    args.push(format!("--setopt=reposdir={reposdir}"));

    if !repositories.is_empty() {
        args.extend(
            repositories
                .iter()
                .map(|repository| format!("--enablerepo={}", repository.id)),
        );
    }

    for index in 0..local.len() {
        args.push(format!("--enablerepo={LOCAL_REPO}{index}"));
        args.push(format!("--setopt={LOCAL_REPO}{index}.gpgcheck=0"));
    }

    args.push("install".into());
    args.extend(base_packages(config));
    args.extend(packages.iter().cloned());
    args
}

/// Пакеты, без которых остальные шаги установки не выполнятся (имена семейства Fedora).
pub fn base_packages(config: &Config) -> Vec<String> {
    if config.source.bare {
        return Vec::new();
    }

    let mut names = vec!["@core", "kernel", "dracut"];

    if config.firmware == Firmware::Uboot {
        names.push("uboot-tools");
    } else {
        names.push("grub2-tools");
        names.extend(grub_targets(config).iter().map(|target| target.package));
    }

    if config.arch == Arch::X86_64 && config.cpu == Cpu::Intel {
        names.push("microcode_ctl");
    }

    if config.users.iter().any(|user| user.name != ROOT) {
        names.push("sudo");
    }

    names.extend(storage::packages(config));

    if config.initrd_network.cmdline(&config.hostname).is_some() {
        names.push("dracut-network");
    }

    if !config.ssh_keys.is_empty() {
        names.extend(["dracut-sshd", "openssh-server"]);
    }

    if config.system_network.setup(config.backend).is_some()
        && config.backend == Backend::NetworkManager
    {
        names.push("NetworkManager");
    }

    names.into_iter().map(String::from).collect()
}

pub(crate) fn uboot(config: &Config) -> bool {
    config.firmware == Firmware::Uboot
}

/// Заголовок LUKS на время установки: в оперативной памяти живой системы, а не на диске.
pub(crate) fn header_temp(name: &str) -> String {
    format!("/run/moonshine-{name}.hdr")
}

/// Куда заголовок кладётся в установленную систему.
pub(crate) fn header_path(name: &str) -> String {
    format!("{LUKS_DIR}/{name}.hdr")
}

pub(crate) fn key_path(name: &str) -> String {
    format!("{LUKS_DIR}/{name}.key")
}

pub(crate) fn run<S: Into<String>>(program: &str, args: impl IntoIterator<Item = S>) -> Step {
    Step::Run(Command::new(program, args))
}

fn system_mounts() -> Vec<Step> {
    let directories = ["dev", "sys", "proc", "run"].map(|name| format!("{TARGET}/{name}"));
    let mut steps = vec![run(
        "mkdir",
        ["-p".to_string()].into_iter().chain(directories),
    )];

    for name in ["dev", "sys", "run"] {
        let target = format!("{TARGET}/{name}");

        steps.push(run("mount", ["--rbind", &format!("/{name}"), &target]));
        steps.push(run("mount", ["--make-rslave", &target]));
    }

    steps.push(run(
        "mount",
        ["-t", "proc", "proc", &format!("{TARGET}/proc")],
    ));

    steps
}

pub(crate) fn file(path: &str, contents: String, mode: u32) -> Step {
    Step::Write(File {
        path: path.into(),
        contents,
        mode,
    })
}

fn kernel_cmdline(config: &Config, ids: &Ids) -> String {
    let mut args = storage::root_arguments(config, ids);

    if let Some(network) = config.initrd_network.cmdline(&config.hostname) {
        args.push(network);
    }

    args.extend(config.kernel_args.iter().cloned());
    args.join(" ")
}

fn settings(config: &Config, ids: &Ids) -> Vec<Step> {
    let cmdline = kernel_cmdline(config, ids);
    let mut modules = storage::dracut_modules(config);
    let mut steps = vec![
        file("/etc/fstab", storage::fstab(config, ids), 0o644),
        file("/etc/hostname", format!("{}\n", config.hostname), 0o644),
    ];

    steps.extend(storage::mdadm_conf(config, ids));

    steps.extend(storage::crypttab(config, ids));
    steps.extend(storage::luks_files(config));

    if !config.ssh_keys.is_empty() {
        modules.push("sshd");

        let keys: String = config
            .ssh_keys
            .iter()
            .map(|key| format!("{}\n", key.trim()))
            .collect();

        steps.push(file("/root/.ssh/authorized_keys", keys, 0o600));
        steps.push(Step::Run(Command::in_target(
            "chmod",
            ["700", "/root/.ssh"],
        )));
    }

    if !modules.is_empty() {
        let mut conf = format!("add_dracutmodules+=\" {} \"\n", modules.join(" "));

        let items = storage::dracut_items(config);

        if !items.is_empty() {
            conf.push_str(&format!("install_items+=\" {} \"\n", items.join(" ")));
        }

        steps.push(file("/etc/dracut.conf.d/10-moonshine.conf", conf, 0o644));
    }

    if !uboot(config) {
        let grub = format!(
            "GRUB_TIMEOUT=5\nGRUB_DISTRIBUTOR=\"{BOOTLOADER_ID}\"\nGRUB_DISABLE_RECOVERY=true\nGRUB_CMDLINE_LINUX=\"{cmdline}\"\n"
        );

        steps.push(file("/etc/default/grub", grub, 0o644));
    }

    steps
}

/// Пользователи кроме `root` получают `wheel` (а с ним `sudo`); без `root` в списке его вход блокируется.
fn accounts(config: &Config) -> Vec<Step> {
    let mut steps = vec![Step::Run(Command::in_target(
        "ln",
        [
            "-sf",
            &format!("/usr/share/zoneinfo/{}", config.timezone),
            "/etc/localtime",
        ],
    ))];

    for user in &config.users {
        if user.name != ROOT {
            steps.push(Step::Run(Command::in_target(
                "useradd",
                ["--create-home", "--groups", "wheel", &user.name],
            )));
        }

        let login = format!("{}:{}\n", user.name, user.password.expose());
        let args: &[&str] = if user.hashed { &["--encrypted"] } else { &[] };

        steps.push(Step::Run(
            Command::in_target("chpasswd", args.iter().copied())
                .input(Input::Secret(Secret::new(login))),
        ));
    }

    if config.users.iter().all(|user| user.name != ROOT) {
        steps.push(Step::Run(Command::in_target("passwd", ["--lock", ROOT])));
    }

    steps
}

fn system_network(config: &Config) -> Vec<Step> {
    let Some(setup) = config.system_network.setup(config.backend) else {
        return Vec::new();
    };

    let mut steps = vec![file(setup.path, setup.contents, setup.mode)];

    for service in &setup.masked {
        steps.push(Step::Run(Command::in_target(
            "systemctl",
            ["mask", service],
        )));
    }

    for service in &setup.services {
        steps.push(Step::Run(Command::in_target(
            "systemctl",
            ["enable", service],
        )));
    }

    if setup.services.contains(&"systemd-resolved") {
        steps.push(Step::Run(Command::in_target(
            "ln",
            [
                "-sf",
                "/run/systemd/resolve/stub-resolv.conf",
                "/etc/resolv.conf",
            ],
        )));
    }

    steps
}

/// Включает названные службы; те, что уже включила настройка сети, повторно не включаются.
fn services(config: &Config) -> Vec<Step> {
    let enabled = config
        .system_network
        .setup(config.backend)
        .map_or_else(Vec::new, |setup| setup.services);
    let mut steps = Vec::new();
    let mut seen: Vec<&str> = enabled.clone();

    for name in &config.services {
        if !seen.contains(&name.as_str()) {
            seen.push(name);
            steps.push(Step::Run(Command::in_target("systemctl", ["enable", name])));
        }
    }

    steps
}

struct GrubTarget {
    name: &'static str,
    package: &'static str,
}

const fn grub_target(name: &'static str, package: &'static str) -> GrubTarget {
    GrubTarget { name, package }
}

fn grub_targets(config: &Config) -> Vec<GrubTarget> {
    let mut targets = Vec::new();

    if config.firmware.has_bios() {
        targets.push(grub_target("i386-pc", "grub2-pc-modules"));
    }

    if config.firmware.has_efi() {
        match config.arch {
            Arch::X86_64 => {
                if config.firmware == Firmware::Hybrid {
                    targets.push(grub_target("i386-efi", "grub2-efi-ia32-modules"));
                }

                targets.push(grub_target("x86_64-efi", "grub2-efi-x64-modules"));
            }
            Arch::Aarch64 => targets.push(grub_target("arm64-efi", "grub2-efi-aa64-modules")),
        }
    }

    targets
}

fn bootloader(config: &Config, ids: &Ids) -> Vec<Step> {
    if config.firmware == Firmware::Uboot {
        return uboot_loader(config, ids);
    }

    let mut steps: Vec<Step> = grub_targets(config)
        .iter()
        .map(|target| {
            let mut args = vec![
                "--no-nvram".to_string(),
                "--force".into(),
                "--removable".into(),
                format!("--target={}", target.name),
                "--boot-directory=/boot".into(),
            ];

            if target.name.ends_with("-efi") {
                args.push("--efi-directory=/boot".into());
            }

            args.push(config.layout.boot_disk().into());

            Step::Run(Command::in_target("grub2-install", args))
        })
        .collect();

    steps.push(Step::Run(Command::in_target(
        "grub2-mkconfig",
        ["-o", GRUB_CONFIG],
    )));
    steps
}

/// U-Boot читает `extlinux/extlinux.conf` с раздела `/boot`; сам загрузчик при необходимости пишется сырым образом.
fn uboot_loader(config: &Config, ids: &Ids) -> Vec<Step> {
    let mut steps = Vec::new();

    if let Some(uboot) = &config.uboot {
        steps.push(run(
            "dd",
            [
                format!("if={}", uboot.image),
                format!("of={}", config.layout.boot_disk()),
                format!("seek={}", uboot.offset),
                "bs=512".to_string(),
                "conv=notrunc".to_string(),
            ],
        ));
    }

    steps.push(file(
        KERNEL_CMDLINE,
        format!("{}\n", kernel_cmdline(config, ids)),
        0o644,
    ));
    steps.push(Step::Run(Command::in_target("sh", ["-c", EXTLINUX_SCRIPT])));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Uboot;
    use crate::config::Unlock;
    use crate::config::User;
    use crate::layout::Crypt;
    use crate::layout::Layout;
    use crate::network::Backend;
    use crate::network::Manual;
    use crate::network::Network;
    use crate::repository::Repository;
    use crate::storage::TYPE_BIOS_BOOT;
    use crate::storage::TYPE_ESP;
    use crate::testing::config;
    use crate::testing::ids;
    use crate::testing::ids_for;

    const HEADER_TEMP: &str = "/run/moonshine-root.hdr";
    const TYPED: Crypt = Crypt {
        unlock: Unlock::Typed,
        detached: false,
    };
    const DETACHED: Crypt = Crypt {
        unlock: Unlock::Typed,
        detached: true,
    };
    const KEYED: Crypt = Crypt {
        unlock: Unlock::Keyfile,
        detached: true,
    };

    fn commands(steps: &[Step]) -> Vec<String> {
        steps.iter().map(Step::to_string).collect()
    }

    fn position(lines: &[String], text: &str) -> usize {
        lines
            .iter()
            .position(|line| line.contains(text))
            .unwrap_or_else(|| panic!("нет шага {text}: {lines:#?}"))
    }

    fn written<'a>(steps: &'a [Step], path: &str) -> Option<&'a File> {
        steps.iter().find_map(|step| match step {
            Step::Write(file) if file.path == path => Some(file),
            _ => None,
        })
    }

    fn packages(copy: &[&str]) -> Config {
        let mut config = config();

        config.source = Source {
            repos: vec!["baseos".into(), "moon".into()],
            local: Vec::new(),
            packages: vec!["vim-enhanced".into(), "@sound".into()],
            options: vec!["releasever=44".into(), "tsflags=nodocs".into()],
            copy: copy.iter().map(ToString::to_string).collect(),
            ..Source::default()
        };
        config
    }

    fn with_repositories() -> Config {
        let mut config = packages(&[]);

        config.source.repositories = vec![
            Repository::parse(
                "id=rocky,baseurl=https://m/$releasever,priority=10,gpgkey=file:///k",
            )
            .unwrap(),
            Repository::parse("id=extra,mirrorlist=https://m/list,gpgcheck=0").unwrap(),
        ];
        config.source.priorities = vec![("moon".into(), 5)];
        config
    }

    #[test]
    fn repositories_from_the_config_are_staged_for_dnf_and_kept_in_the_system() {
        let config = with_repositories();
        let steps = build(&config, &ids());
        let lines = commands(&steps);
        let dnf = &lines[position(&lines, "dnf ")];

        assert!(lines.contains(&format!("mkdir -p {STAGE_DIR}")));
        assert!(lines.iter().any(|line| {
            line.starts_with("sh -c cat > \"$1\" sh /run/moonshine/repos.d/rocky.repo")
        }));
        assert!(position(&lines, "mkdir -p /run/moonshine") < position(&lines, "dnf "));
        assert!(dnf.contains("--setopt=reposdir=/etc/yum.repos.d,/run/moonshine/repos.d"));
        assert!(dnf.contains("--enablerepo=rocky") && dnf.contains("--enablerepo=extra"));
        assert!(dnf.contains("--setopt=moon.priority=5"));
        assert!(position(&lines, "dnf ") < position(&lines, "rm -rf /run/moonshine/repos.d"));

        let kept = written(&steps, "/etc/yum.repos.d/rocky.repo").unwrap();

        assert!(kept.contents.contains("priority=10\n") && kept.mode == 0o644);
        assert!(written(&steps, "/etc/yum.repos.d/extra.repo").is_some());
    }

    #[test]
    fn without_defined_repositories_dnf_keeps_the_host_reposdir() {
        let lines = commands(&build(&packages(&[]), &ids()));

        let dnf = &lines[position(&lines, "dnf ")];

        assert!(dnf.contains("--setopt=reposdir=/etc/yum.repos.d install"));
        assert!(!dnf.contains("/run/moonshine/repos.d"));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("rm -rf /run/moonshine"))
        );
    }

    #[test]
    fn bare_installs_only_the_given_packages() {
        let mut config = packages(&[]);

        config.source.bare = true;

        let args = dnf_args(&config, TARGET, "--assumeyes");
        let install = args.iter().position(|arg| arg == "install").unwrap();

        assert_eq!(args[install + 1..], ["vim-enhanced", "@sound"]);
        assert!(base_packages(&config).is_empty());
        assert!(config.source.validate().is_ok());

        config.source.packages.clear();
        assert!(config.source.validate().is_err());
    }

    #[test]
    fn repository_ids_and_priorities_are_checked_against_each_other() {
        let mut config = with_repositories();

        assert!(config.source.validate().is_ok());

        config.source.priorities = vec![("unknown".into(), 5)];
        assert!(config.source.validate().is_err());

        config.source.priorities = vec![("moon".into(), 5), ("moon".into(), 6)];
        assert!(config.source.validate().is_err());

        config.source.priorities.clear();
        config.source.repositories[0].id = "baseos".into();
        assert!(config.source.validate().is_err());

        config.source.repositories[0].id = "moonshine-x".into();
        assert!(config.source.validate().is_err());
    }

    #[test]
    fn a_defined_repository_alone_is_enough_as_a_source() {
        let mut config = with_repositories();

        config.source.repos.clear();
        config.source.priorities.clear();
        assert!(config.source.validate().is_ok());

        config.source.repositories.clear();
        assert!(config.source.validate().is_err());
    }

    #[test]
    fn packages_are_installed_with_dnf_from_the_chosen_repositories_only() {
        let lines = commands(&build(&packages(&[]), &ids()));
        let dnf = &lines[position(&lines, "dnf ")];

        assert!(dnf.starts_with(&format!(
            "dnf --installroot={TARGET} --releasever=44 --setopt=tsflags=nodocs --assumeyes --disablerepo=*"
        )));
        assert!(dnf.contains(
            "--enablerepo=baseos --enablerepo=moon --setopt=reposdir=/etc/yum.repos.d install @core kernel"
        ));
        assert!(dnf.ends_with("vim-enhanced @sound"));
    }

    #[test]
    fn the_system_mounts_come_before_the_installation_of_packages() {
        let lines = commands(&build(&packages(&[]), &ids()));
        let mount = position(&lines, "mount /dev/mapper/cryptroot");
        let bind = position(&lines, "mount --rbind /dev");
        let dnf = position(&lines, "dnf ");

        assert!(mount < bind && bind < dnf);
        assert!(
            position(
                &lines,
                "mkdir -p /mnt/target/dev /mnt/target/sys /mnt/target/proc /mnt/target/run"
            ) < bind
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("mount --rbind /dev"))
                .count(),
            1
        );
        assert!(dnf < position(&lines, "dracut --force"));
    }

    #[test]
    fn the_initramfs_module_carries_the_tools_of_the_shell_scripts() {
        let module = include_str!("../dist/dracut/90moonshine-install/module-setup.sh");
        let words: Vec<&str> = module.split_whitespace().collect();

        for script in [RELEASE_SCRIPT] {
            for tool in [
                "lsblk",
                "tac",
                "mountpoint",
                "swapoff",
                "umount",
                "dmsetup",
                "mdadm",
                "cryptsetup",
            ] {
                let used = script
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word == tool);

                assert!(
                    !used || words.contains(&tool),
                    "{tool} is missing in module-setup.sh"
                );
            }
        }
    }

    #[test]
    fn extra_kernel_parameters_close_the_command_line() {
        let config = Config {
            kernel_args: vec!["quiet".into(), "console=ttyS0,115200".into()],
            ..detached()
        };
        let steps = build(&config, &ids());
        let grub = &written(&steps, "/etc/default/grub").unwrap().contents;

        assert!(grub.contains(" quiet console=ttyS0,115200\"\n"), "{grub}");
        assert!(
            !written(&build(&detached(), &ids()), "/etc/default/grub")
                .unwrap()
                .contents
                .contains("quiet")
        );
    }

    #[test]
    fn the_command_line_is_saved_for_kernel_install_before_the_packages_arrive() {
        let steps = build(&detached(), &ids());
        let cmdline = &written(&steps, "/etc/kernel/cmdline").unwrap().contents;
        let grub = &written(&steps, "/etc/default/grub").unwrap().contents;
        let saved = steps
            .iter()
            .position(
                |step| matches!(step, Step::Write(file) if file.path == "/etc/kernel/cmdline"),
            )
            .unwrap();
        let dnf = steps
            .iter()
            .position(|step| step.to_string().starts_with("dnf "))
            .unwrap();

        assert!(saved < dnf);
        assert!(cmdline.starts_with("root=UUID=") && cmdline.ends_with('\n'));
        assert!(grub.contains(&format!("GRUB_CMDLINE_LINUX=\"{}\"", cmdline.trim_end())));
    }

    #[test]
    fn services_are_enabled_once_after_the_network_ones() {
        let config = Config {
            system_network: Network::Dhcp { interface: None },
            backend: Backend::NetworkManager,
            services: vec![
                "sshd".into(),
                "NetworkManager".into(),
                "sshd".into(),
                "chronyd".into(),
            ],
            ..config()
        };
        let lines = commands(&build(&config, &ids()));
        let enabled: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("systemctl enable"))
            .collect();

        assert_eq!(
            enabled,
            [
                "chroot /mnt/target systemctl enable NetworkManager",
                "chroot /mnt/target systemctl enable sshd",
                "chroot /mnt/target systemctl enable chronyd",
            ]
        );
        assert!(position(&lines, "systemctl enable sshd") > position(&lines, "dnf "));
        assert!(
            !commands(&build(&self::config(), &ids()))
                .iter()
                .any(|line| line.contains("systemctl"))
        );
    }

    #[test]
    fn run_is_bound_from_the_host_like_dev_and_sys() {
        let lines = commands(&build(&packages(&[]), &ids()));
        let bind = position(&lines, "mount --rbind /run /mnt/target/run");

        assert_eq!(lines[bind + 1], "mount --make-rslave /mnt/target/run");
        assert!(!lines.iter().any(|line| line.contains("tmpfs")));
    }

    #[test]
    fn the_repository_files_are_copied_into_the_system() {
        let lines = commands(&build(
            &packages(&["/etc/yum.repos.d/a.repo", "/etc/yum.repos.d/moon.repo"]),
            &ids(),
        ));
        let dnf = position(&lines, "dnf ");
        let mkdir = position(&lines, &format!("mkdir -p {TARGET}/etc/yum.repos.d"));

        assert!(dnf < mkdir);
        assert!(lines.contains(&format!(
            "cp /etc/yum.repos.d/a.repo {TARGET}/etc/yum.repos.d"
        )));
        assert!(lines.contains(&format!(
            "cp /etc/yum.repos.d/moon.repo {TARGET}/etc/yum.repos.d"
        )));
        assert!(
            !commands(&build(&packages(&[]), &ids()))
                .iter()
                .any(|line| line.starts_with("cp "))
        );
    }

    #[test]
    fn the_preview_uses_the_same_arguments_with_another_root_and_no_questions() {
        let config = packages(&[]);
        let preview = dnf_args(&config, "/var/tmp/x", "--assumeno");
        let install = dnf_args(&config, TARGET, "--assumeyes");

        assert_eq!(preview[0], "--installroot=/var/tmp/x");
        assert_eq!(preview[1..3], install[1..3]);
        assert!(
            preview.contains(&"--assumeno".to_string())
                && !preview.contains(&"--assumeyes".to_string())
        );
        assert_eq!(preview[4..], install[4..]);
    }

    #[test]
    fn options_become_setopt_and_releasever_becomes_its_own_flag() {
        let args = dnf_args(&packages(&[]), TARGET, "--assumeyes");

        assert!(args.contains(&"--releasever=44".to_string()));
        assert!(args.contains(&"--setopt=tsflags=nodocs".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("setopt=releasever")));
    }

    #[test]
    fn a_broken_option_is_refused() {
        let mut config = packages(&[]);

        config.source.options.push("nodocs".into());

        assert!(config.validate().is_err());
    }

    #[test]
    fn the_dnf_options_are_optional() {
        let mut config = packages(&[]);

        config.source.options.clear();

        let lines = commands(&build(&config, &ids()));

        assert!(!lines[position(&lines, "dnf ")].contains("--releasever"));
        assert!(!lines[position(&lines, "dnf ")].contains("--setopt=tsflags"));
    }

    #[test]
    fn base_packages_follow_the_chosen_features() {
        let mut config = config();
        let has =
            |config: &Config, name: &str| base_packages(config).iter().any(|item| item == name);

        assert!(has(&config, "@core") && has(&config, "kernel") && has(&config, "dracut"));
        assert!(
            has(&config, "grub2-tools")
                && has(&config, "grub2-efi-x64-modules")
                && !has(&config, "grub2-pc-modules")
                && !has(&config, "grub2-efi-ia32-modules")
        );
        assert!(
            has(&config, "cryptsetup")
                && !has(&config, "dracut-network")
                && !has(&config, "dracut-sshd")
        );

        config.firmware = Firmware::Bios;
        config.layout = Layout::simple("/dev/sda", "/dev/sda", Firmware::Bios);
        assert!(
            has(&config, "grub2-pc-modules")
                && !has(&config, "grub2-efi-x64-modules")
                && !has(&config, "cryptsetup")
        );

        config.firmware = Firmware::Hybrid;
        assert!(
            has(&config, "grub2-pc-modules")
                && has(&config, "grub2-efi-ia32-modules")
                && has(&config, "grub2-efi-x64-modules")
        );

        config.passphrase = Some(Secret::new("x"));
        config.initrd_network = Network::Dhcp { interface: None };
        config.ssh_keys = vec!["ssh-ed25519 AAAA".into()];
        assert!(
            has(&config, "dracut-network")
                && has(&config, "dracut-sshd")
                && has(&config, "openssh-server")
        );

        config.system_network = Network::Dhcp { interface: None };
        config.backend = Backend::NetworkManager;
        assert!(has(&config, "NetworkManager"));
    }

    #[test]
    fn a_local_directory_becomes_an_unsigned_repository_enabled_next_to_the_others() {
        let mut config = packages(&[]);

        config.source.local = vec!["/srv/repo".into(), "/mnt/usb/rpms".into()];

        let args = dnf_args(&config, TARGET, "--assumeyes");

        for expected in [
            "--repofrompath=moonshine-local0,file:///srv/repo",
            "--repofrompath=moonshine-local1,file:///mnt/usb/rpms",
            "--enablerepo=moonshine-local0",
            "--setopt=moonshine-local1.gpgcheck=0",
        ] {
            assert!(args.contains(&expected.to_string()), "{expected}: {args:?}");
        }

        let disable = args
            .iter()
            .position(|arg| arg == "--disablerepo=*")
            .unwrap();
        let enable = args
            .iter()
            .position(|arg| arg == "--enablerepo=moonshine-local0")
            .unwrap();

        assert!(disable < enable);
        assert!(
            !dnf_args(&packages(&[]), TARGET, "--assumeyes")
                .iter()
                .any(|arg| arg.contains("repofrompath"))
        );
    }

    #[test]
    fn only_a_local_directory_is_enough_as_a_source() {
        let mut config = config();

        config.source = Source {
            local: vec!["/srv/repo".into()],
            ..Source::default()
        };

        assert!(config.validate().is_ok());
        assert!(
            Config {
                source: Source::default(),
                ..config
            }
            .validate()
            .is_err()
        );
    }

    fn user(name: &str) -> User {
        User {
            name: name.into(),
            password: Secret::new("$6$salt$hash"),
            hashed: true,
        }
    }

    #[test]
    fn ordinary_users_join_wheel_and_root_is_locked() {
        let config = Config {
            users: vec![user("alice"), user("bob")],
            ..config()
        };
        let lines = commands(&build(&config, &ids()));

        for name in ["alice", "bob"] {
            assert!(lines.contains(&format!(
                "chroot {TARGET} useradd --create-home --groups wheel {name}"
            )));
        }

        assert!(lines.contains(&format!("chroot {TARGET} passwd --lock root")));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("chpasswd --encrypted"))
                .count(),
            2
        );
        assert!(base_packages(&config).contains(&"sudo".to_string()));
    }

    #[test]
    fn root_in_the_list_is_enabled_and_is_not_created_or_put_in_wheel() {
        let config = Config {
            users: vec![user("root")],
            ..config()
        };
        let lines = commands(&build(&config, &ids()));

        assert!(
            !lines
                .iter()
                .any(|line| line.contains("useradd") || line.contains("--lock"))
        );
        assert!(lines.contains(&format!("chroot {TARGET} chpasswd --encrypted < <секрет>")));
        assert!(!base_packages(&config).contains(&"sudo".to_string()));
    }

    #[test]
    fn root_together_with_another_user_stays_enabled() {
        let config = Config {
            users: vec![user("root"), user("alice")],
            ..config()
        };
        let lines = commands(&build(&config, &ids()));

        assert!(!lines.iter().any(|line| line.contains("--lock")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("useradd") && line.ends_with("alice"))
        );
    }

    #[test]
    fn a_plain_password_is_not_marked_as_encrypted() {
        let lines = commands(&build(&config(), &ids()));

        assert!(lines.contains(&format!("chroot {TARGET} chpasswd < <секрет>")));
    }

    #[test]
    fn users_must_exist_be_unique_and_valid() {
        assert!(
            Config {
                users: Vec::new(),
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                users: vec![user("a"), user("a")],
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                users: vec![User {
                    name: "a".into(),
                    password: Secret::new("plain"),
                    hashed: true
                }],
                ..config()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn aarch64_uses_its_own_grub_packages_and_efi_target() {
        let config = Config {
            arch: Arch::Aarch64,
            ..config()
        };
        let packages = base_packages(&config);
        let lines = commands(&build(&config, &ids()));

        assert!(packages.contains(&"grub2-efi-aa64-modules".to_string()));
        assert!(!packages.contains(&"grub2-efi-x64-modules".to_string()));
        assert!(lines.iter().any(|line| line.contains("--target=arm64-efi")));
    }

    #[test]
    fn the_platform_must_match_the_architecture() {
        assert!(
            Config {
                firmware: Firmware::Bios,
                arch: Arch::Aarch64,
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                arch: Arch::X86_64,
                ..uboot_config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                arch: Arch::Aarch64,
                ..uboot_config()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn intel_microcode_is_installed_only_on_intel_x86() {
        let has = |config: &Config| base_packages(config).contains(&"microcode_ctl".to_string());

        assert!(has(&Config {
            cpu: Cpu::Intel,
            ..config()
        }));
        assert!(!has(&Config {
            cpu: Cpu::Amd,
            ..config()
        }));
        assert!(!has(&Config {
            cpu: Cpu::Intel,
            arch: Arch::Aarch64,
            ..config()
        }));
    }

    #[test]
    fn uefi_boot_is_one_fat_partition_that_is_also_the_esp() {
        let lines = commands(&build(&config(), &ids()));

        assert!(
            lines
                .iter()
                .any(|line| line.contains("size=512MiB") && line.contains(TYPE_ESP))
        );
        assert!(lines.contains(&"mkfs.fat -F 32 -i ABCD1234 /dev/sda1".to_string()));
        assert!(lines.contains(&format!("mount /dev/sda1 {TARGET}/boot")));
        assert!(!lines.iter().any(|line| line.contains("/boot/efi")));
        assert!(lines.iter().any(|line| {
            line.ends_with(
                "grub2-install --no-nvram --force --removable --target=x86_64-efi --boot-directory=/boot --efi-directory=/boot /dev/sda",
            )
        }));
    }

    #[test]
    fn bios_layout_has_a_4_mib_biosboot_before_the_fat_boot() {
        let bios = with_firmware(Firmware::Bios);
        let lines = commands(&build(&bios, &ids_for(&bios.layout)));

        assert!(
            lines
                .iter()
                .any(|line| line.contains("size=4MiB") && line.contains(TYPE_BIOS_BOOT))
        );
        assert!(lines.contains(&"mkfs.fat -F 32 -i ABCD1234 /dev/sda2".to_string()));
        assert!(!lines.iter().any(|line| line.contains("/boot/efi")));
        assert!(lines.contains(&format!(
            "chroot {TARGET} grub2-install --no-nvram --force --removable --target=i386-pc --boot-directory=/boot /dev/sda"
        )));
    }

    #[test]
    fn hybrid_installs_bios_and_both_efi_loaders_and_then_one_config() {
        let hybrid = with_firmware(Firmware::Hybrid);
        let lines = commands(&build(&hybrid, &ids_for(&hybrid.layout)));
        let first = position(&lines, "grub2-install");
        let install = |target: &str, efi: &str| {
            format!(
                "chroot {TARGET} grub2-install --no-nvram --force --removable --target={target} --boot-directory=/boot{efi} /dev/sda"
            )
        };

        assert_eq!(lines[first], install("i386-pc", ""));
        assert_eq!(
            lines[first + 1],
            install("i386-efi", " --efi-directory=/boot")
        );
        assert_eq!(
            lines[first + 2],
            install("x86_64-efi", " --efi-directory=/boot")
        );
        assert_eq!(
            lines[first + 3],
            format!("chroot {TARGET} grub2-mkconfig -o /boot/grub2/grub.cfg")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("size=4MiB") && line.contains(TYPE_BIOS_BOOT))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("size=512MiB") && line.contains(TYPE_ESP))
        );
    }

    #[test]
    fn the_root_is_the_second_partition_and_is_encrypted() {
        let lines = commands(&build(&config(), &ids()));

        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("cryptsetup luksFormat")
                    && line.contains("/dev/sda2")
                    && line.contains("--uuid 00000000-0000-0000-0000-000000000001"))
        );
        assert!(
            lines.contains(
                &"mkfs.ext4 -F -U 00000000-0000-0000-0000-000000000005 /dev/mapper/cryptroot"
                    .to_string()
            )
        );
        assert!(lines.contains(&format!("mount /dev/mapper/cryptroot {TARGET}")));
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("cryptsetup close cryptroot")
        );
    }

    #[test]
    fn without_encryption_the_root_is_a_plain_partition() {
        let steps = build(
            &Config {
                layout: Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi),
                ..config()
            },
            &ids(),
        );
        let lines = commands(&steps);

        assert!(!lines.iter().any(|line| line.contains("cryptsetup")));
        assert!(lines.contains(&format!("mount /dev/sda2 {TARGET}")));
        assert!(written(&steps, "/etc/crypttab").is_none());
        assert!(
            !written(&steps, "/etc/default/grub")
                .unwrap()
                .contents
                .contains("rd.luks")
        );
    }

    #[test]
    fn passphrase_and_password_never_appear_in_the_printed_plan() {
        let text = commands(&build(&config(), &ids())).join("\n");

        assert!(!text.contains("phrase") && !text.contains("alice:pw"));
        assert!(text.contains("<секрет>"));
    }

    #[test]
    fn passwords_reach_the_commands_through_stdin() {
        let steps = build(&config(), &ids());
        let inputs: Vec<&Input> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Run(Command {
                    input: Some(input), ..
                }) => Some(input),
                _ => None,
            })
            .collect();

        assert!(inputs.contains(&&Input::Secret(Secret::new("phrase"))));
        assert!(inputs.contains(&&Input::Secret(Secret::new("alice:pw\n"))));
    }

    #[test]
    fn fstab_and_crypttab_use_the_planned_identifiers() {
        let steps = build(&config(), &ids());
        let fstab = &written(&steps, "/etc/fstab").unwrap().contents;

        assert!(fstab.contains("UUID=00000000-0000-0000-0000-000000000005 / ext4"));
        assert!(fstab.contains("UUID=ABCD-1234 /boot vfat"));
        assert_eq!(
            written(&steps, "/etc/crypttab").unwrap().contents,
            "cryptroot UUID=00000000-0000-0000-0000-000000000001 none luks,x-initrd.attach\n"
        );
    }

    fn detached() -> Config {
        Config {
            layout: config().layout.with_root_crypt(Some(DETACHED)),
            ..config()
        }
    }

    fn two_disks() -> Config {
        Config {
            layout: Layout::simple("/dev/sdb", "/dev/sdc", Firmware::Uefi)
                .with_root_crypt(Some(TYPED)),
            ..config()
        }
    }

    fn uboot_config() -> Config {
        with_firmware(Firmware::Uboot)
    }

    fn with_firmware(firmware: Firmware) -> Config {
        Config {
            layout: Layout::simple("/dev/sda", "/dev/sda", firmware).with_root_crypt(Some(TYPED)),
            firmware,
            ..config()
        }
    }

    #[test]
    fn a_detached_header_is_created_in_memory_and_used_for_formatting_and_opening() {
        let lines = commands(&build(&detached(), &ids()));

        assert!(lines.contains(&format!("truncate -s 16MiB {HEADER_TEMP}")));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("cryptsetup luksFormat")
                    && line.contains(&format!("--header {HEADER_TEMP} /dev/sda2")))
        );
        assert!(lines.iter().any(|line| line.starts_with("cryptsetup open")
            && line.contains(&format!("--header {HEADER_TEMP} /dev/sda2 cryptroot"))));
        assert!(position(&lines, "truncate") < position(&lines, "cryptsetup luksFormat"));
        assert!(
            !commands(&build(&config(), &ids()))
                .iter()
                .any(|line| line.contains("--header"))
        );
    }

    #[test]
    fn the_root_partition_gets_a_partuuid_to_find_it_without_a_luks_signature() {
        let lines = commands(&build(&detached(), &ids()));
        let sfdisk = &lines[position(&lines, "sfdisk")];

        assert!(
            sfdisk.contains("uuid=00000000-0000-0000-0000-000000000002"),
            "{sfdisk}"
        );
        assert!(
            !commands(&build(&config(), &ids()))
                .iter()
                .any(|line| line.contains("uuid=00000000-0000-0000-0000-000000000002"))
        );
    }

    #[test]
    fn crypttab_and_dracut_carry_the_header_into_the_initramfs() {
        let steps = build(&detached(), &ids());
        let dracut = &written(&steps, "/etc/dracut.conf.d/10-moonshine.conf")
            .unwrap()
            .contents;
        let grub = &written(&steps, "/etc/default/grub").unwrap().contents;

        assert_eq!(
            written(&steps, "/etc/crypttab").unwrap().contents,
            "cryptroot PARTUUID=00000000-0000-0000-0000-000000000002 none luks,header=/etc/luks/root.hdr,x-initrd.attach\n"
        );
        assert!(
            dracut.contains("add_dracutmodules+=\" crypt \""),
            "{dracut}"
        );
        assert!(
            dracut.contains("install_items+=\" /etc/crypttab /etc/luks/root.hdr \""),
            "{dracut}"
        );
        assert!(
            !grub.contains("rd.luks.uuid"),
            "диск без сигнатуры LUKS по UUID не найти: {grub}"
        );
    }

    #[test]
    fn the_header_is_stored_in_the_system_copied_and_removed_from_memory() {
        let lines = commands(&build(&detached(), &ids()));

        assert!(lines.contains(&format!("cp {HEADER_TEMP} {TARGET}/etc/luks/root.hdr")));
        assert!(lines.contains(&format!("chmod 600 {TARGET}/etc/luks/root.hdr")));
        assert!(
            position(&lines, "dnf ") < position(&lines, "cp /run/moonshine-root.hdr /mnt/target")
        );
        assert!(
            position(&lines, "cp /run/moonshine-root.hdr /mnt/target")
                < position(&lines, "dracut --force")
        );
        assert_eq!(lines.last().unwrap(), &format!("rm -f {HEADER_TEMP}"));
    }

    #[test]
    fn one_disk_keeps_two_partitions_and_boot_on_the_same_disk() {
        let lines = commands(&build(&config(), &ids()));

        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("sfdisk"))
                .count(),
            1
        );
        assert!(lines.iter().any(|line| line.contains("/dev/sda2")));
        assert!(!config().layout.separate_boot());
    }

    #[test]
    fn two_disks_put_boot_on_the_first_and_root_alone_on_the_second() {
        let steps = build(&two_disks(), &ids());
        let lines = commands(&steps);

        assert!(lines.contains(&"wipefs --all /dev/sdc".to_string()));
        assert!(lines.contains(&"wipefs --all /dev/sdb".to_string()));
        assert!(lines.iter().any(|line| line.starts_with("sfdisk /dev/sdc")));
        assert!(!lines.iter().any(|line| line.starts_with("sfdisk /dev/sdb")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/dev/sdc1") && line.contains("/boot"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("cryptsetup open")
                    && line.contains("/dev/sdb cryptroot"))
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("/dev/sdb1") || line.contains("/dev/sdb2"))
        );
        assert!(!lines.iter().any(|line| line.contains("/dev/sdc2")));
    }

    #[test]
    fn two_disks_install_a_removable_uefi_loader_on_the_boot_disk() {
        let lines = commands(&build(&two_disks(), &ids()));

        assert!(lines.iter().any(|line| {
            line.ends_with(
                "--target=x86_64-efi --boot-directory=/boot --efi-directory=/boot /dev/sdc",
            )
        }));
    }

    #[test]
    fn two_disks_install_the_bios_loader_on_the_boot_disk() {
        let config = Config {
            firmware: Firmware::Bios,
            ..two_disks()
        };
        let lines = commands(&build(&config, &ids()));

        assert!(
            lines
                .iter()
                .any(|line| line.ends_with("--target=i386-pc --boot-directory=/boot /dev/sdc"))
        );
    }

    #[test]
    fn a_detached_header_on_two_disks_names_the_raw_disk_in_crypttab() {
        let config = Config {
            layout: two_disks().layout.with_root_crypt(Some(DETACHED)),
            ..two_disks()
        };
        let steps = build(&config, &ids());
        let crypttab = &written(&steps, "/etc/crypttab").unwrap().contents;

        assert_eq!(
            crypttab,
            "cryptroot /dev/sdb none luks,header=/etc/luks/root.hdr,x-initrd.attach\n"
        );
        assert!(
            !commands(&steps)
                .iter()
                .any(|line| line.contains("PARTUUID"))
        );
    }

    #[test]
    fn the_keyfile_goes_into_the_system_and_the_initramfs() {
        let config = Config {
            layout: two_disks().layout.with_root_crypt(Some(KEYED)),
            keys: vec![("/dev/sdb".into(), Secret::new("phrase"))],
            ..two_disks()
        };
        let steps = build(&config, &ids());

        assert_eq!(
            written(&steps, "/etc/luks/root.key").unwrap().contents,
            "phrase"
        );
        assert!(
            written(&steps, "/etc/crypttab")
                .unwrap()
                .contents
                .contains(" /etc/luks/root.key luks")
        );
        assert!(
            written(&steps, "/etc/dracut.conf.d/10-moonshine.conf")
                .unwrap()
                .contents
                .contains("/etc/luks/root.hdr /etc/luks/root.key")
        );
    }

    #[test]
    fn a_keyfile_needs_a_detached_header_and_a_separate_boot_disk_needs_it_too() {
        let key = |path: &str| vec![(path.to_string(), Secret::new("k".repeat(64)))];
        let exposed = Crypt {
            unlock: Unlock::Keyfile,
            detached: false,
        };

        assert!(
            Config {
                layout: config().layout.with_root_crypt(Some(exposed)),
                keys: key("/dev/sda2"),
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(two_disks().validate().is_err());
        assert!(
            Config {
                layout: two_disks().layout.with_root_crypt(Some(DETACHED)),
                ..two_disks()
            }
            .validate()
            .is_ok()
        );
        assert!(
            Config {
                layout: config().layout.with_root_crypt(Some(KEYED)),
                keys: key("/dev/sda2"),
                ..config()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn uboot_boots_from_the_first_partition_through_extlinux() {
        let config = Config {
            uboot: Some(Uboot {
                image: "/srv/u-boot.bin".into(),
                offset: 64,
            }),
            ..uboot_config()
        };
        let lines = commands(&build(&config, &ids_for(&config.layout)));

        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("sfdisk /dev/sda") && line.contains("start=16MiB"))
        );
        assert!(lines.contains(&"mount /dev/sda1 /mnt/target/boot".to_string()));
        assert!(lines.iter().any(
            |line| line.starts_with("cryptsetup open") && line.contains("/dev/sda2 cryptroot")
        ));
        assert!(lines.contains(
            &"dd if=/srv/u-boot.bin of=/dev/sda seek=64 bs=512 conv=notrunc".to_string()
        ));
        assert!(lines.iter().any(|line| line.contains("extlinux.conf")));
        assert!(!lines.iter().any(|line| line.contains("grub2")));
    }

    #[test]
    fn the_extlinux_script_holds_no_user_data() {
        let config = Config {
            firmware: Firmware::Uboot,
            arch: Arch::Aarch64,
            hostname: "uniquehost".into(),
            initrd_network: Network::Dhcp {
                interface: Some("eth0".into()),
            },
            ..config()
        };
        let steps = build(&config, &ids());
        let script = steps
            .iter()
            .find_map(|step| match step {
                Step::Run(command)
                    if command.program == "chroot" && command.args.contains(&"sh".to_string()) =>
                {
                    command.args.last().cloned()
                }
                _ => None,
            })
            .unwrap();

        assert_eq!(script, EXTLINUX_SCRIPT);
        assert!(
            !script.contains("eth0")
                && !script.contains("uniquehost")
                && !script.contains("rd.neednet")
        );
        assert!(
            written(&steps, "/etc/kernel/cmdline")
                .unwrap()
                .contents
                .contains("ip=eth0:dhcp")
        );
    }

    #[test]
    fn uboot_without_an_image_does_not_write_the_disk() {
        let config = Config {
            firmware: Firmware::Uboot,
            ..config()
        };

        assert!(
            !commands(&build(&config, &ids()))
                .iter()
                .any(|line| line.starts_with("dd "))
        );
        assert!(base_packages(&config).contains(&"uboot-tools".to_string()));
    }

    #[test]
    fn the_uboot_offset_must_stay_in_the_reserved_area() {
        let uboot = |offset| Config {
            arch: Arch::Aarch64,
            uboot: Some(Uboot {
                image: "/srv/u-boot.bin".into(),
                offset,
            }),
            ..uboot_config()
        };

        assert!(uboot(64).validate().is_ok());
        assert!(
            uboot(crate::config::UBOOT_RESERVED_SECTORS)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn an_encrypted_device_without_a_secret_is_refused() {
        assert!(
            Config {
                passphrase: None,
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                layout: config().layout.with_root_crypt(Some(KEYED)),
                ..config()
            }
            .validate()
            .is_err()
        );
        assert!(detached().validate().is_ok());
    }

    #[test]
    fn ssh_keys_enable_remote_unlock_in_the_initramfs() {
        let steps = build(
            &Config {
                initrd_network: Network::Dhcp { interface: None },
                ssh_keys: vec!["ssh-ed25519 AAAA admin".into()],
                ..config()
            },
            &ids(),
        );
        let grub = &written(&steps, "/etc/default/grub").unwrap().contents;

        assert!(grub.contains("rd.neednet=1 ip=dhcp"));
        assert_eq!(
            written(&steps, "/root/.ssh/authorized_keys")
                .unwrap()
                .contents,
            "ssh-ed25519 AAAA admin\n"
        );
        assert_eq!(
            written(&steps, "/root/.ssh/authorized_keys").unwrap().mode,
            0o600
        );
        assert!(
            written(&steps, "/etc/dracut.conf.d/10-moonshine.conf")
                .unwrap()
                .contents
                .contains("crypt sshd")
        );
    }

    #[test]
    fn a_manual_network_is_passed_to_the_kernel_command_line() {
        let manual = Manual {
            address: "10.0.0.5".parse().unwrap(),
            prefix: 16,
            gateway: "10.0.0.1".parse().unwrap(),
            dns: Vec::new(),
            interface: "eth0".into(),
        };
        let steps = build(
            &Config {
                initrd_network: Network::Manual(manual),
                ..config()
            },
            &ids(),
        );

        assert!(
            written(&steps, "/etc/default/grub")
                .unwrap()
                .contents
                .contains("ip=10.0.0.5::10.0.0.1:255.255.0.0:moon:eth0:none")
        );
    }

    #[test]
    fn the_system_network_is_independent_of_the_initramfs_network() {
        let fixed = Manual {
            address: "10.0.0.5".parse().unwrap(),
            prefix: 24,
            gateway: "10.0.0.1".parse().unwrap(),
            dns: vec!["9.9.9.9".parse().unwrap()],
            interface: "eth0".into(),
        };
        let config = Config {
            system_network: Network::Manual(fixed),
            ..config()
        };
        let steps = build(&config, &ids());
        let lines = commands(&steps);

        assert!(
            !written(&steps, "/etc/default/grub")
                .unwrap()
                .contents
                .contains("ip=")
        );
        assert!(
            written(&steps, "/etc/systemd/network/10-moonshine.network")
                .unwrap()
                .contents
                .contains("Address=10.0.0.5/24")
        );
        assert!(
            position(&lines, "systemctl enable systemd-networkd")
                < position(&lines, "systemctl enable systemd-resolved")
        );
        assert!(lines.contains(&format!(
            "chroot {TARGET} ln -sf /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf"
        )));
    }

    #[test]
    fn network_manager_is_enabled_when_chosen() {
        let config = Config {
            system_network: Network::Dhcp { interface: None },
            backend: Backend::NetworkManager,
            ..config()
        };
        let steps = build(&config, &ids());

        assert_eq!(
            written(
                &steps,
                "/etc/NetworkManager/system-connections/moonshine.nmconnection"
            )
            .unwrap()
            .mode,
            0o600
        );
        assert!(
            commands(&steps).contains(&format!("chroot {TARGET} systemctl enable NetworkManager"))
        );
    }

    #[test]
    fn the_other_network_service_is_masked_before_the_chosen_one_is_enabled() {
        let masks = |backend| {
            let config = Config {
                system_network: Network::Dhcp { interface: None },
                backend,
                ..config()
            };
            let lines = commands(&build(&config, &ids()));
            let (mask, enable) = match backend {
                Backend::Networkd => ("NetworkManager", "systemd-networkd"),
                Backend::NetworkManager => ("systemd-networkd", "NetworkManager"),
            };
            position(&lines, &format!("systemctl mask {mask}"))
                < position(&lines, &format!("systemctl enable {enable}"))
        };

        assert!(masks(Backend::Networkd));
        assert!(masks(Backend::NetworkManager));
    }

    #[test]
    fn without_a_system_network_nothing_is_written_or_enabled() {
        let lines = commands(&build(&config(), &ids()));

        assert!(
            !lines
                .iter()
                .any(|line| line.contains("systemctl") || line.contains("systemd/network"))
        );
    }

    #[test]
    fn without_ssh_keys_the_initramfs_has_no_network() {
        let steps = build(&config(), &ids());

        assert!(
            !written(&steps, "/etc/default/grub")
                .unwrap()
                .contents
                .contains("ip=dhcp")
        );
        assert!(written(&steps, "/root/.ssh/authorized_keys").is_none());
        assert!(
            !written(&steps, "/etc/dracut.conf.d/10-moonshine.conf")
                .unwrap()
                .contents
                .contains("sshd")
        );
    }

    #[test]
    fn the_mount_directory_is_removed_after_the_unmount_and_before_the_layers_close() {
        let lines = commands(&build(&config(), &ids()));
        let umount = position(&lines, "umount --recursive");

        assert_eq!(lines[umount + 1], format!("rmdir {TARGET}"));
        assert!(umount + 1 < position(&lines, "cryptsetup close"));
    }

    #[test]
    fn steps_follow_a_safe_order() {
        let lines = commands(&build(&config(), &ids()));
        let order = [
            "освободить диски",
            "sfdisk",
            "cryptsetup luksFormat",
            "cryptsetup open",
            "mkfs.ext4 -F -U 00000000-0000-0000-0000-000000000005",
            "dnf ",
            "chpasswd",
            "dracut --force",
            "grub2-install",
            "grub2-mkconfig",
            "umount --recursive",
            "cryptsetup close",
        ];
        let positions: Vec<usize> = order.iter().map(|text| position(&lines, text)).collect();

        assert!(positions.is_sorted(), "{positions:?}");
    }
}
