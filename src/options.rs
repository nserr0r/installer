use std::net::Ipv4Addr;

use crate::config::Arch;
use crate::config::Cpu;
use crate::config::Firmware;
use crate::config::Source;
use crate::config::Uboot;
use crate::config::Unlock;
use crate::config::check_password_hash;
use crate::config::check_user;
use crate::config::is_safe_path;
use crate::error::Error;
use crate::hardware::Hardware;
use crate::i18n::Lang;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::layout;
use crate::layout::Crypt;
use crate::layout::Layout;
use crate::layout::Partition;
use crate::layout::Raid;
use crate::layout::Volume;
use crate::network::Backend;
use crate::network::Manual;
use crate::network::Network;
use crate::repository;
use crate::repository::Repository;

/// Пользователь из командной строки: имя и, если есть, готовый хеш пароля.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSpec {
    pub name: String,
    pub hash: Option<String>,
}

impl UserSpec {
    fn parse(text: &str) -> Result<Self, Error> {
        let (name, hash) = match text.split_once(':') {
            Some((name, hash)) => (name, Some(hash.to_string())),
            None => (text, None),
        };

        check_user(name)?;
        hash.as_deref().map_or(Ok(()), check_password_hash)?;

        Ok(Self {
            name: name.into(),
            hash,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub layout: Layout,
    pub source: Source,
    pub hostname: String,
    pub users: Vec<UserSpec>,
    pub timezone: String,
    pub firmware: Firmware,
    pub arch: Arch,
    pub cpu: Cpu,
    pub uboot: Option<Uboot>,
    pub initrd_network: Network,
    pub system_network: Network,
    pub backend: Backend,
    pub ssh_keys: Vec<String>,
    pub kernel_args: Vec<String>,
    pub services: Vec<String>,
    pub passphrase_file: Option<String>,
    pub assume_yes: bool,
}

pub fn usage() -> String {
    tr!(
        "Without arguments the interactive wizard opens. Check a config without changing anything: moonshine-install check FILE|https://…|git+URL#[branch:]path.toml [--plan]
Usage: moonshine-install (--disk /dev/sda [--boot-disk /dev/sdb] | --partition … --raid … --volume …) --hostname name --user name[:hash]... (--repo id | --repo-dir /dir | --repository …)...
  [--package name]... [--option key=value]... [--kernel-arg ARG]... [--service UNIT]... [--repo-file /etc/yum.repos.d/x.repo]...
  [--repository id=ID,baseurl|metalink|mirrorlist=URL[,name=…][,priority=1..99][,gpgcheck=0|1][,gpgkey=URL][,exclude=…][,includepkgs=…]]...
  [--repo-priority ID=1..99]... [--bare]  (--bare: no built-in package set, everything comes from --package)
  [--boot-size 512]  (MiB, at least 256; /boot is FAT32 and doubles as the ESP)
  [--timezone Europe/Moscow] [--firmware hybrid|uefi|bios|uboot] [--arch x86_64|aarch64] [--cpu intel|amd|other]
  [--uboot-image /path/u-boot.bin --uboot-offset sectors] [--keyfile] [--no-encryption] [--detached-header]
  [--ssh-key \"key\"]... [--passphrase-file /path] [--lang en|ru] [--yes]
  layout: --partition disk=/dev/sda,size=512MiB|rest|whole,point=biosboot|boot|reserve|swap|lvm:GROUP|raid:ARRAY|/path[,fs=ext4|xfs|btrfs][,mkfs=…][,options=…][,crypt=pass|key[,header=detached]]
          --raid name=md0,level=0|1|5|6|10,point=lvm:GROUP|swap|/path   --volume vg=GROUP,name=root,size=40GiB|rest,point=/path|swap|pool[,pool=POOL]
  --user: without a hash the password is asked on the console; root is optional — if it is not listed it is locked, and the others get wheel
  network: --initrd-network and --system-network dhcp|manual|none, plus --initrd-interface, --initrd-address 192.168.1.10/24, --initrd-gateway, --initrd-dns (same with --system-); --system-backend networkd|networkmanager
  The same can be put into a TOML file and run with --config path | https://… | git+URL#[branch:]path.toml",
        "Без аргументов открывается интерактивный мастер. Проверка конфига без изменений: moonshine-install check ФАЙЛ|https://…|git+URL#[ветка:]путь.toml [--plan]
Использование: moonshine-install (--disk /dev/sda [--boot-disk /dev/sdb] | --partition … --raid … --volume …) --hostname имя --user имя[:хеш]... (--repo id | --repo-dir /каталог | --repository …)...
  [--package имя]... [--option ключ=значение]... [--repo-file /etc/yum.repos.d/x.repo]...
  [--repository id=ID,baseurl|metalink|mirrorlist=URL[,name=…][,priority=1..99][,gpgcheck=0|1][,gpgkey=URL][,exclude=…][,includepkgs=…]]...
  [--repo-priority ID=1..99]... [--bare]  (--bare: без встроенного набора пакетов, всё задаётся через --package)
  [--boot-size 512]  (MiB, не меньше 256; /boot — FAT32, он же ESP)
  [--timezone Europe/Moscow] [--firmware hybrid|uefi|bios|uboot] [--arch x86_64|aarch64] [--cpu intel|amd|other]
  [--uboot-image /путь/u-boot.bin --uboot-offset секторов] [--keyfile] [--no-encryption] [--detached-header]
  [--ssh-key \"ключ\"]... [--passphrase-file /путь] [--lang en|ru] [--yes]
  разметка: --partition disk=/dev/sda,size=512MiB|rest|whole,point=biosboot|boot|reserve|swap|lvm:ГРУППА|raid:МАССИВ|/точка[,fs=ext4|xfs|btrfs][,mkfs=…][,options=…][,crypt=pass|key[,header=detached]]
            --raid name=md0,level=0|1|5|6|10,point=lvm:ГРУППА|swap|/точка   --volume vg=ГРУППА,name=root,size=40GiB|rest,point=/точка|swap|pool[,pool=ПУЛ]
  --user: без хеша пароль спросят на консоли; root не нужен — если его нет в списке, он блокируется, а остальные получают wheel
  сеть: --initrd-network и --system-network dhcp|manual|none, к ним --initrd-interface, --initrd-address 192.168.1.10/24, --initrd-gateway, --initrd-dns (то же с --system-); --system-backend networkd|networkmanager
  Всё то же можно положить в файл TOML и запустить с --config путь | https://… | git+URL#[ветка:]путь.toml"
    )
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, Error> {
        let mut args = args.into_iter();
        let mut raw = Raw::detect();

        while let Some(flag) = args.next() {
            raw.read(&flag, &mut || {
                args.next().ok_or_else(|| {
                    Error::Invalid(tr!(
                        "{flag} needs a value\n{}",
                        "у {flag} нет значения\n{}",
                        usage()
                    ))
                })
            })?;
        }

        raw.finish()
    }
}

/// Флаги в том виде, как их прочитали, до сборки и проверки целого.
struct Raw {
    disk: Option<String>,
    boot_disk: Option<String>,
    boot_size: Option<u64>,
    custom: Layout,
    source: Source,
    hostname: Option<String>,
    users: Vec<UserSpec>,
    timezone: String,
    firmware: Firmware,
    arch: Arch,
    cpu: Cpu,
    plain: bool,
    detached: bool,
    keyfile: bool,
    uboot_image: Option<String>,
    uboot_offset: u64,
    ssh_keys: Vec<String>,
    kernel_args: Vec<String>,
    services: Vec<String>,
    initrd: Flags,
    system: Flags,
    backend: Backend,
    passphrase_file: Option<String>,
    assume_yes: bool,
}

impl Raw {
    fn detect() -> Self {
        let hardware = Hardware::detect();

        Self {
            disk: None,
            boot_disk: None,
            boot_size: None,
            custom: Layout::default(),
            source: Source::default(),
            hostname: None,
            users: Vec::new(),
            timezone: "UTC".into(),
            firmware: hardware.firmware,
            arch: hardware.arch,
            cpu: hardware.cpu,
            plain: false,
            detached: false,
            keyfile: false,
            uboot_image: None,
            uboot_offset: 0,
            ssh_keys: Vec::new(),
            kernel_args: Vec::new(),
            services: Vec::new(),
            initrd: Flags::default(),
            system: Flags::default(),
            backend: Backend::Networkd,
            passphrase_file: None,
            assume_yes: false,
        }
    }

    fn read(
        &mut self,
        flag: &str,
        next: &mut dyn FnMut() -> Result<String, Error>,
    ) -> Result<(), Error> {
        match flag {
            "--disk" => self.disk = Some(next()?),
            "--boot-disk" => self.boot_disk = Some(next()?),
            "--boot-size" => self.boot_size = Some(size_mib(flag, &next()?)?),
            "--partition" => self.custom.partitions.push(Partition::parse(&next()?)?),
            "--raid" => self.custom.raids.push(Raid::parse(&next()?)?),
            "--volume" => self.custom.volumes.push(Volume::parse(&next()?)?),
            "--repo" => self.source.repos.push(next()?),
            "--repo-dir" => self.source.local.push(next()?),
            "--repo-file" => self.source.copy.push(next()?),
            "--repository" => self.source.repositories.push(Repository::parse(&next()?)?),
            "--repo-priority" => self
                .source
                .priorities
                .push(repository::parse_priority(&next()?)?),
            "--package" => self.source.packages.push(next()?),
            "--option" => self.source.options.push(next()?),
            "--bare" => self.source.bare = true,
            "--hostname" => self.hostname = Some(next()?),
            "--user" => self.users.push(UserSpec::parse(&next()?)?),
            "--timezone" => self.timezone = next()?,
            "--ssh-key" => self.ssh_keys.push(next()?),
            "--kernel-arg" => self.kernel_args.push(next()?),
            "--service" => self.services.push(next()?),
            "--arch" => self.arch = parse_arch(&next()?)?,
            "--cpu" => self.cpu = parse_cpu(&next()?)?,
            "--firmware" => self.firmware = parse_firmware(&next()?)?,
            "--system-backend" => self.backend = parse_backend(&next()?)?,
            _ if flag.starts_with("--initrd-") => {
                self.initrd.set(&flag["--initrd-".len()..], &next()?)?;
            }
            _ if flag.starts_with("--system-") => {
                self.system.set(&flag["--system-".len()..], &next()?)?;
            }
            "--no-encryption" => self.plain = true,
            "--detached-header" => self.detached = true,
            "--keyfile" => self.keyfile = true,
            "--uboot-image" => self.uboot_image = Some(next()?),
            "--uboot-offset" => {
                let text = next()?;

                self.uboot_offset = text.parse().map_err(|_| {
                    Error::Invalid(tr!(
                        "invalid U-Boot offset: {text}",
                        "неверное смещение U-Boot: {text}"
                    ))
                })?;
            }
            "--passphrase-file" => self.passphrase_file = Some(next()?),
            "--yes" => self.assume_yes = true,
            "--lang" => {
                Lang::parse(&next()?)?;
            }
            other => {
                return Err(Error::Invalid(tr!(
                    "unknown argument: {other}\n{}",
                    "неизвестный аргумент: {other}\n{}",
                    usage()
                )));
            }
        }

        Ok(())
    }

    fn finish(self) -> Result<Options, Error> {
        let layout = self.layout()?;

        if self.users.is_empty() {
            return Err(Error::Invalid(tr!(
                "--user is required\n{}",
                "нужен --user\n{}",
                usage()
            )));
        }

        self.check_passphrase_file(&layout)?;

        let hostname = self.hostname.ok_or_else(|| {
            Error::Invalid(tr!(
                "--hostname is required\n{}",
                "нужен --hostname\n{}",
                usage()
            ))
        })?;
        let initrd = if self.ssh_keys.is_empty() {
            "none"
        } else {
            "dhcp"
        };

        Ok(Options {
            layout,
            source: self.source,
            hostname,
            users: self.users,
            timezone: self.timezone,
            firmware: self.firmware,
            arch: self.arch,
            cpu: self.cpu,
            initrd_network: self.initrd.into_network(initrd)?,
            system_network: self.system.into_network("none")?,
            backend: self.backend,
            uboot: self.uboot_image.map(|image| Uboot {
                image,
                offset: self.uboot_offset,
            }),
            ssh_keys: self.ssh_keys,
            kernel_args: self.kernel_args,
            services: self.services,
            passphrase_file: self.passphrase_file,
            assume_yes: self.assume_yes,
        })
    }

    /// Простая схема из `--disk` либо своя разметка; вместе они не сочетаются.
    fn layout(&self) -> Result<Layout, Error> {
        let custom = &self.custom;

        if custom.partitions.is_empty() && custom.raids.is_empty() && custom.volumes.is_empty() {
            return self.simple_layout();
        }

        let error = if self.boot_size.is_some() {
            tt!(
                "--boot-size works only with --disk: a custom layout sets sizes in its lines",
                "--boot-size работает только с --disk: в своей разметке размеры пишут в строках"
            )
        } else if self.plain || self.detached || self.keyfile {
            tt!(
                "--no-encryption, --keyfile and --detached-header work only with --disk: a custom layout sets crypt= and header= in its lines",
                "--no-encryption, --keyfile и --detached-header работают только с --disk: в своей разметке пишут crypt= и header= в строках"
            )
        } else if self.disk.is_some() || self.boot_disk.is_some() {
            tt!(
                "--disk and --boot-disk cannot be combined with --partition, --raid and --volume",
                "--disk и --boot-disk несовместимы с --partition, --raid и --volume"
            )
        } else {
            return Ok(custom.clone());
        };

        Err(Error::Invalid(error.into()))
    }

    fn simple_layout(&self) -> Result<Layout, Error> {
        let disk = self.disk.as_deref().ok_or_else(|| {
            Error::Invalid(tr!(
                "--disk or --partition is required\n{}",
                "нужен --disk или --partition\n{}",
                usage()
            ))
        })?;
        let boot_disk = self.boot_disk.as_deref().unwrap_or(disk);
        let layout = Layout::simple(disk, boot_disk, self.firmware)
            .with_boot_size(self.boot_size.unwrap_or(layout::BOOT_SIZE));
        let crypt = (!self.plain).then_some(Crypt {
            unlock: if self.keyfile {
                Unlock::Keyfile
            } else {
                Unlock::Typed
            },
            detached: self.detached || self.keyfile || layout.separate_boot(),
        });

        Ok(layout.with_root_crypt(crypt))
    }

    fn check_passphrase_file(&self, layout: &Layout) -> Result<(), Error> {
        let Some(path) = &self.passphrase_file else {
            return Ok(());
        };

        if !is_safe_path(path) {
            return Err(Error::Invalid(tr!(
                "--passphrase-file: an absolute path of letters, digits and \"/._-+@:~\" is required: {path}",
                "--passphrase-file: нужен абсолютный путь из букв, цифр и «/._-+@:~»: {path}"
            )));
        }

        if !layout.needs_passphrase() {
            return Err(Error::Invalid(
                tt!(
                    "--passphrase-file is not needed: no device has crypt=pass",
                    "--passphrase-file не нужен: нет устройства с crypt=pass"
                )
                .into(),
            ));
        }

        Ok(())
    }
}

fn parse_arch(text: &str) -> Result<Arch, Error> {
    match text {
        "x86_64" => Ok(Arch::X86_64),
        "aarch64" => Ok(Arch::Aarch64),
        other => Err(Error::Invalid(tr!(
            "unknown architecture: {other}",
            "неизвестная архитектура: {other}"
        ))),
    }
}

fn parse_cpu(text: &str) -> Result<Cpu, Error> {
    match text {
        "intel" => Ok(Cpu::Intel),
        "amd" => Ok(Cpu::Amd),
        "other" => Ok(Cpu::Other),
        other => Err(Error::Invalid(tr!(
            "unknown CPU: {other}",
            "неизвестный процессор: {other}"
        ))),
    }
}

fn parse_firmware(text: &str) -> Result<Firmware, Error> {
    match text {
        "uefi" => Ok(Firmware::Uefi),
        "bios" => Ok(Firmware::Bios),
        "hybrid" => Ok(Firmware::Hybrid),
        "uboot" => Ok(Firmware::Uboot),
        other => Err(Error::Invalid(tr!(
            "unknown firmware: {other}",
            "неизвестная прошивка: {other}"
        ))),
    }
}

fn parse_backend(text: &str) -> Result<Backend, Error> {
    match text {
        "networkd" => Ok(Backend::Networkd),
        "networkmanager" => Ok(Backend::NetworkManager),
        other => Err(Error::Invalid(tr!(
            "unknown network service: {other}",
            "неизвестная служба сети: {other}"
        ))),
    }
}

#[derive(Default)]
struct Flags {
    mode: Option<String>,
    interface: Option<String>,
    address: Option<(Ipv4Addr, u8)>,
    gateway: Option<Ipv4Addr>,
    dns: Vec<Ipv4Addr>,
}

impl Flags {
    fn set(&mut self, name: &str, value: &str) -> Result<(), Error> {
        match name {
            "network" => self.mode = Some(value.to_string()),
            "interface" => self.interface = Some(value.to_string()),
            "address" => self.address = Some(parse_address(value)?),
            "gateway" => self.gateway = Some(parse_ip(value)?),
            "dns" => self.dns.push(parse_ip(value)?),
            other => {
                return Err(Error::Invalid(tr!(
                    "unknown network argument: {other}\n{}",
                    "неизвестный аргумент сети: {other}\n{}",
                    usage()
                )));
            }
        }

        Ok(())
    }

    fn into_network(self, default_mode: &str) -> Result<Network, Error> {
        let missing =
            |name: &str| Error::Invalid(tr!("manual needs {name}", "для manual нужен {name}"));

        match self.mode.as_deref().unwrap_or(default_mode) {
            "none" => Ok(Network::None),
            "dhcp" => Ok(Network::Dhcp {
                interface: self.interface,
            }),
            "manual" => {
                let (address, prefix) = self.address.ok_or_else(|| missing("address"))?;

                Ok(Network::Manual(Manual {
                    address,
                    prefix,
                    gateway: self.gateway.ok_or_else(|| missing("gateway"))?,
                    dns: self.dns,
                    interface: self.interface.ok_or_else(|| missing("interface"))?,
                }))
            }
            other => Err(Error::Invalid(tr!(
                "unknown network mode: {other}",
                "неизвестный режим сети: {other}"
            ))),
        }
    }
}

fn size_mib(name: &str, text: &str) -> Result<u64, Error> {
    text.parse().map_err(|_| {
        Error::Invalid(tr!(
            "{name}: an integer number of MiB is required: {text}",
            "{name}: нужно целое число MiB: {text}"
        ))
    })
}

fn parse_ip(text: &str) -> Result<Ipv4Addr, Error> {
    text.parse().map_err(|_| {
        Error::Invalid(tr!(
            "invalid IPv4 address: {text}",
            "неверный IPv4-адрес: {text}"
        ))
    })
}

fn parse_address(text: &str) -> Result<(Ipv4Addr, u8), Error> {
    let invalid = || {
        Error::Invalid(tr!(
            "the address must look like 192.168.1.10/24: {text}",
            "адрес должен быть вида 192.168.1.10/24: {text}"
        ))
    };
    let (address, prefix) = text.split_once('/').ok_or_else(invalid)?;
    let prefix: u8 = prefix.parse().map_err(|_| invalid())?;

    if prefix > 32 {
        return Err(invalid());
    }

    Ok((parse_ip(address)?, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Options, Error> {
        Options::parse(line.split_whitespace().map(String::from))
    }

    #[test]
    fn packages_come_from_repositories_and_local_directories_with_extras() {
        let base = "--disk /dev/sda --hostname h --user u";
        let line = format!(
            "{base} --repo evillin --repo moon --repo-dir /srv/rpms --package vim --package @sound --option releasever=44 --option tsflags= --repo-file /etc/yum.repos.d/moon.repo"
        );

        assert_eq!(
            parse(&line).unwrap().source,
            Source {
                repos: vec!["evillin".into(), "moon".into()],
                local: vec!["/srv/rpms".into()],
                packages: vec!["vim".into(), "@sound".into()],
                options: vec!["releasever=44".into(), "tsflags=".into()],
                copy: vec!["/etc/yum.repos.d/moon.repo".into()],
                ..Source::default()
            }
        );
    }

    #[test]
    fn repositories_priorities_and_bare_are_read() {
        let base = "--disk /dev/sda --hostname h --user u";
        let line = format!(
            "{base} --repo evillin --repo-priority evillin=5 --repository id=rocky,baseurl=https://m/x,priority=10,gpgcheck=0 --bare --package bash"
        );
        let source = parse(&line).unwrap().source;

        assert_eq!(source.priorities, [("evillin".to_string(), 5)]);
        assert_eq!(source.repositories[0].id, "rocky");
        assert_eq!(source.repositories[0].priority, Some(10));
        assert!(source.bare);
        assert!(parse(&format!("{base} --repository id=r")).is_err());
        assert!(parse(&format!("{base} --repo-priority evillin")).is_err());
        assert!(
            parse(&format!(
                "{base} --repository id=r,baseurl=https://m/x,gpgcheck=0"
            ))
            .is_ok()
        );
    }

    #[test]
    fn options_are_exactly_what_was_given() {
        let base = "--disk /dev/sda --hostname h --user u --repo r";

        assert!(parse(base).unwrap().source.options.is_empty());
        assert_eq!(
            parse(&format!(
                "{base} --option tsflags=nodocs --option weak_deps=False"
            ))
            .unwrap()
            .source
            .options,
            ["tsflags=nodocs", "weak_deps=False"]
        );
    }

    #[test]
    fn the_passphrase_file_needs_a_typed_device_and_a_safe_path() {
        let base = "--disk /dev/sda --hostname h --user u --repo r";

        assert_eq!(
            parse(&format!("{base} --passphrase-file /root/luks.txt"))
                .unwrap()
                .passphrase_file
                .as_deref(),
            Some("/root/luks.txt")
        );
        assert!(parse(base).unwrap().passphrase_file.is_none());
        assert!(parse(&format!("{base} --passphrase-file relative")).is_err());
        assert!(parse(&format!("{base} --passphrase-file /a/../b")).is_err());
        assert!(
            parse(&format!(
                "{base} --no-encryption --passphrase-file /root/luks.txt"
            ))
            .is_err()
        );
        assert!(
            parse(&format!(
                "{base} --keyfile --passphrase-file /root/luks.txt"
            ))
            .is_err()
        );
    }

    #[test]
    fn dry_run_no_longer_exists() {
        assert!(parse("--disk /dev/sda --hostname h --user u --repo r --dry-run").is_err());
    }

    #[test]
    fn users_are_listed_with_an_optional_hash() {
        let base = "--disk /dev/sda --hostname h --repo r";
        let options = parse(&format!("{base} --user alice:$6$salt$hash --user bob")).unwrap();

        assert_eq!(
            options.users,
            [
                UserSpec {
                    name: "alice".into(),
                    hash: Some("$6$salt$hash".into())
                },
                UserSpec {
                    name: "bob".into(),
                    hash: None
                },
            ]
        );
        assert!(parse(base).is_err());
        assert!(parse(&format!("{base} --user Alice")).is_err());
        assert!(parse(&format!("{base} --user alice:plain")).is_err());
    }

    #[test]
    fn required_and_optional_arguments_are_read() {
        let options = parse("--disk /dev/sda --repo evillin --hostname moon --user alice --firmware bios --arch x86_64 --cpu intel --no-encryption --ssh-key key --ssh-key other").unwrap();

        assert_eq!(options.layout.root_device(), "/dev/sda3");
        assert_eq!(options.source.repos, ["evillin"]);
        assert_eq!(options.firmware, Firmware::Bios);
        assert_eq!((options.arch, options.cpu), (Arch::X86_64, Cpu::Intel));
        assert_eq!(options.timezone, "UTC");
        assert_eq!(options.ssh_keys, ["key", "other"]);
        assert!(!options.layout.has_crypt() && !options.assume_yes);
    }

    #[test]
    fn the_architecture_and_the_processor_are_checked() {
        let base = "--disk /dev/sda --repo r --hostname h --user u";

        assert_eq!(
            parse(&format!("{base} --arch aarch64")).unwrap().arch,
            Arch::Aarch64
        );
        assert!(parse(&format!("{base} --arch riscv")).is_err());
        assert!(parse(&format!("{base} --cpu via")).is_err());
    }

    #[test]
    fn kernel_arguments_repeat_and_keep_their_commas() {
        let line = "--disk /dev/sda --repo r --hostname h --user u --kernel-arg quiet --kernel-arg console=ttyS0,115200";
        let options = parse(line).unwrap();

        assert_eq!(options.kernel_args, ["quiet", "console=ttyS0,115200"]);
        assert!(parse("--disk /dev/sda --repo r --hostname h --user u --kernel-arg").is_err());
    }

    #[test]
    fn services_repeat() {
        let line =
            "--disk /dev/sda --repo r --hostname h --user u --service sshd --service chronyd";

        assert_eq!(parse(line).unwrap().services, ["sshd", "chronyd"]);
        assert!(parse("--disk /dev/sda --repo r --hostname h --user u --service").is_err());
    }

    #[test]
    fn the_initramfs_network_defaults_follow_the_ssh_keys() {
        let base = "--disk /dev/sda --repo r --hostname h --user u";

        assert_eq!(parse(base).unwrap().initrd_network, Network::None);
        assert_eq!(
            parse(&format!("{base} --ssh-key k"))
                .unwrap()
                .initrd_network,
            Network::Dhcp { interface: None }
        );
        assert_eq!(
            parse(&format!(
                "{base} --initrd-network dhcp --initrd-interface eth1"
            ))
            .unwrap()
            .initrd_network,
            Network::Dhcp {
                interface: Some("eth1".into())
            }
        );
    }

    #[test]
    fn the_two_networks_are_configured_separately() {
        let base = "--disk /dev/sda --repo r --hostname h --user u --initrd-network dhcp --system-network manual --system-address 10.0.0.2/8 --system-gateway 10.0.0.1 --system-interface eth0 --system-dns 9.9.9.9";
        let options = parse(base).unwrap();

        assert_eq!(options.initrd_network, Network::Dhcp { interface: None });
        assert!(
            matches!(options.system_network, Network::Manual(ref manual) if manual.prefix == 8 && manual.dns.len() == 1)
        );
        assert_eq!(options.backend, Backend::Networkd);
        assert_eq!(
            parse(&format!("{base} --system-backend networkmanager"))
                .unwrap()
                .backend,
            Backend::NetworkManager
        );
        assert!(parse(&format!("{base} --system-backend openrc")).is_err());
    }

    #[test]
    fn a_manual_network_needs_address_gateway_and_interface() {
        let base = "--disk /dev/sda --repo r --hostname h --user u --initrd-network manual";
        let full = format!(
            "{base} --initrd-address 192.168.1.10/24 --initrd-gateway 192.168.1.1 --initrd-interface eth0 --initrd-dns 1.1.1.1 --initrd-dns 8.8.8.8"
        );

        let Network::Manual(manual) = parse(&full).unwrap().initrd_network else {
            panic!("ожидалась ручная сеть")
        };

        assert_eq!(manual.prefix, 24);
        assert_eq!(manual.dns.len(), 2);
        assert!(parse(base).is_err());
        assert!(
            parse(&format!(
                "{base} --initrd-address 192.168.1.10/24 --initrd-gateway 192.168.1.1"
            ))
            .is_err()
        );
        assert!(parse(&format!("{base} --initrd-address 192.168.1.10 --initrd-gateway 192.168.1.1 --initrd-interface eth0")).is_err());
        assert!(parse(&format!("{base} --initrd-address 192.168.1.10/33 --initrd-gateway 192.168.1.1 --initrd-interface eth0")).is_err());
        assert!(parse(&format!("{base} --initrd-address 192.168.1.10/24 --initrd-gateway nope --initrd-interface eth0")).is_err());
        assert!(
            parse("--disk /dev/sda --repo r --hostname h --user u --initrd-network wifi").is_err()
        );
        assert!(
            parse("--disk /dev/sda --repo r --hostname h --user u --initrd-color red").is_err()
        );
    }

    fn root_crypt(line: &str) -> Option<Crypt> {
        let layout = parse(line).unwrap().layout;

        layout.crypt_of(&layout.root_device())
    }

    #[test]
    fn a_detached_header_is_read() {
        let base = "--disk /dev/sda --repo r --hostname h --user u";

        assert_eq!(
            root_crypt(base),
            Some(Crypt {
                unlock: Unlock::Typed,
                detached: false
            })
        );
        assert_eq!(
            root_crypt(&format!("{base} --detached-header")),
            Some(Crypt {
                unlock: Unlock::Typed,
                detached: true
            })
        );
    }

    #[test]
    fn a_separate_boot_disk_and_a_keyfile_make_the_header_detached() {
        let base = "--disk /dev/sda --boot-disk /dev/sdb --repo r --hostname h --user u";
        let options = parse(&format!(
            "{base} --keyfile --firmware uboot --uboot-image /u.bin --uboot-offset 64"
        ))
        .unwrap();

        assert_eq!(
            options.layout.crypt_of(&options.layout.root_device()),
            Some(Crypt {
                unlock: Unlock::Keyfile,
                detached: true
            })
        );
        assert_eq!(
            root_crypt(&format!("{base} --firmware uefi")).map(|crypt| crypt.detached),
            Some(true)
        );
        assert_eq!(
            root_crypt("--disk /dev/sda --repo r --hostname h --user u --keyfile")
                .map(|crypt| crypt.detached),
            Some(true)
        );
        assert_eq!(options.firmware, Firmware::Uboot);
        assert_eq!(
            options.uboot,
            Some(Uboot {
                image: "/u.bin".into(),
                offset: 64
            })
        );
        assert!(parse(&format!("{base} --uboot-offset x")).is_err());
    }

    #[test]
    fn the_boot_disk_defaults_to_the_disk() {
        let base = "--disk /dev/sda --repo r --hostname h --user u";

        assert_eq!(parse(base).unwrap().layout.boot_disk(), "/dev/sda");
        assert_eq!(
            parse(&format!("{base} --boot-disk /dev/sdb"))
                .unwrap()
                .layout
                .boot_disk(),
            "/dev/sdb"
        );
    }

    #[test]
    fn encryption_is_on_by_default() {
        assert!(
            parse("--disk /dev/sda --repo r --hostname h --user u")
                .unwrap()
                .layout
                .has_crypt()
        );
        assert!(
            !parse("--disk /dev/sda --repo r --hostname h --user u --no-encryption")
                .unwrap()
                .layout
                .has_crypt()
        );
    }

    #[test]
    fn missing_values_and_unknown_arguments_are_errors() {
        assert!(parse("--repo r --hostname h --user u").is_err());
        assert!(parse("--disk /dev/sda --image").is_err());
        assert!(parse("--disk /dev/sda --repo r --hostname h --user u --firmware arm").is_err());
        assert!(parse("--disk /dev/sda --repo r --hostname h --user u --force").is_err());
    }

    #[test]
    fn boot_size_is_set_in_mebibytes() {
        let base = "--disk /dev/sda --repo r --hostname h --user u --firmware uefi";
        let options = parse(&format!("{base} --boot-size 300")).unwrap();

        assert_eq!(options.layout.partitions[0].size, layout::Size::Mib(300));
        assert!(options.layout.validate(options.firmware).is_ok());
        assert!(parse(&format!("{base} --boot-size big")).is_err());
        assert!(
            parse(&format!("{base} --boot-size 100"))
                .unwrap()
                .layout
                .validate(Firmware::Uefi)
                .is_err()
        );
        assert!(parse("--repo r --hostname h --user u --boot-size 300 --partition disk=/dev/sda,size=rest,point=/").is_err());
    }

    #[test]
    fn partitions_raids_and_volumes_make_a_custom_layout() {
        let base = "--repo r --hostname h --user u --firmware uefi";
        let line = format!(
            "{base} --partition disk=/dev/sda,size=1GiB,point=boot --partition disk=/dev/sda,size=rest,point=lvm:vg --volume vg=vg,name=root,size=rest,point=/"
        );
        let options = parse(&line).unwrap();

        assert_eq!(options.layout.partitions.len(), 2);
        assert_eq!(options.layout.root_device(), "/dev/vg/root");
        assert!(options.layout.validate(options.firmware).is_ok());
        assert!(!options.layout.has_crypt());
    }

    #[test]
    fn a_custom_layout_cannot_be_mixed_with_disk_flags() {
        let line = "--disk /dev/sda --repo r --hostname h --user u --partition disk=/dev/sda,size=rest,point=/";

        assert!(parse(line).is_err());
        assert!(parse("--boot-disk /dev/sdb --repo r --hostname h --user u --volume vg=v,name=n,size=rest,point=/").is_err());
    }

    #[test]
    fn encryption_of_a_custom_layout_is_written_in_the_rows() {
        let line = "--repo r --hostname h --user u --firmware uefi --partition disk=/dev/sda,size=1GiB,point=boot --partition disk=/dev/sda,size=rest,point=/,crypt=pass --partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key,header=detached";
        let layout = parse(line).unwrap().layout;

        assert_eq!(layout.crypts().len(), 2);
        assert_eq!(layout.early(), ["/dev/sda2"]);
        assert!(layout.validate(Firmware::Uefi).is_ok());
    }

    #[test]
    fn global_encryption_flags_do_not_work_with_a_custom_layout() {
        let line = "--repo r --hostname h --user u --partition disk=/dev/sda,size=rest,point=/";

        for flag in ["--no-encryption", "--keyfile", "--detached-header"] {
            assert!(parse(&format!("{line} {flag}")).is_err(), "{flag}");
        }
    }

    #[test]
    fn broken_layout_flags_are_refused_while_parsing() {
        for flag in ["--partition", "--raid", "--volume"] {
            assert!(parse(&format!("--repo r --hostname h --user u {flag} nonsense")).is_err());
            assert!(parse(&format!("--repo r --hostname h --user u {flag}")).is_err());
        }

        assert!(
            parse("--repo r --hostname h --user u --partition disk=/dev/sda,size=1TB,point=swap")
                .is_err()
        );
        assert!(
            parse("--repo r --hostname h --user u --raid name=md0,level=2,point=swap").is_err()
        );
    }
}
