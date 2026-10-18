use std::net::Ipv4Addr;
use std::path::Path;

use crate::config;
use crate::config::Config;
use crate::config::Firmware;
use crate::config::Secret;
use crate::config::Source;
use crate::config::Uboot;
use crate::config::Unlock;
use crate::config::User;
use crate::disks::Disk;
use crate::hardware::Hardware;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::input::Input;
use crate::layout;
use crate::layout::Crypt;
use crate::layout::Layout;
use crate::layout::Level;
use crate::network::Backend;
use crate::network::Manual;
use crate::network::Network;
use crate::repos::Repo;
use crate::sources::Sources;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id {
    Scheme,
    BootSize,
    Disk,
    DataDisks,
    RaidLevel,
    Layout,
    Firmware,
    Repos,
    LocalRepos,
    Options,
    Packages,
    CopyRepos,
    Hostname,
    User,
    Password,
    PasswordAgain,
    Timezone,
    KernelArgs,
    Services,
    Encrypt,
    Passphrase,
    PassphraseAgain,
    Header,
    BootDisk,
    UbootImage,
    UbootOffset,
    Unlock,
    SshKeys,
    InitrdMode,
    InitrdInterface,
    InitrdAddress,
    InitrdGateway,
    InitrdDns,
    SystemMode,
    Backend,
    SystemInterface,
    SystemAddress,
    SystemGateway,
    SystemDns,
    PackageList,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Disk,
    Repos,
    System,
    Encryption,
    InitrdNetwork,
    SystemNetwork,
    Packages,
    Summary,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Encryption,
        Page::Disk,
        Page::Repos,
        Page::System,
        Page::InitrdNetwork,
        Page::SystemNetwork,
        Page::Packages,
        Page::Summary,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Disk => tt!("Disk", "Диск"),
            Page::Repos => tt!("Repositories", "Репозитории"),
            Page::System => tt!("System and user", "Система и пользователь"),
            Page::Encryption => tt!("Encryption", "Шифрование"),
            Page::InitrdNetwork => tt!("Network in initramfs", "Сеть в initramfs"),
            Page::SystemNetwork => tt!("Network in system", "Сеть в системе"),
            Page::Packages => tt!("Packages", "Пакеты"),
            Page::Summary => tt!("Summary", "Итог"),
        }
    }

    pub fn fields(self) -> &'static [Id] {
        match self {
            Page::Disk => &[
                Id::Scheme,
                Id::BootDisk,
                Id::BootSize,
                Id::Disk,
                Id::DataDisks,
                Id::RaidLevel,
                Id::Layout,
                Id::Firmware,
                Id::UbootImage,
                Id::UbootOffset,
            ],
            Page::Repos => &[
                Id::Repos,
                Id::LocalRepos,
                Id::Options,
                Id::Packages,
                Id::CopyRepos,
            ],
            Page::System => &[
                Id::Hostname,
                Id::User,
                Id::Password,
                Id::PasswordAgain,
                Id::Timezone,
                Id::KernelArgs,
                Id::Services,
            ],
            Page::Encryption => &[
                Id::Encrypt,
                Id::Unlock,
                Id::Header,
                Id::Passphrase,
                Id::PassphraseAgain,
                Id::SshKeys,
            ],
            Page::InitrdNetwork => &[
                Id::InitrdMode,
                Id::InitrdInterface,
                Id::InitrdAddress,
                Id::InitrdGateway,
                Id::InitrdDns,
            ],
            Page::SystemNetwork => &[
                Id::SystemMode,
                Id::Backend,
                Id::SystemInterface,
                Id::SystemAddress,
                Id::SystemGateway,
                Id::SystemDns,
            ],
            Page::Packages => &[Id::PackageList],
            Page::Summary => &[Id::Confirm],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Secret,
    /// Значение выбирается из списка: Enter открывает список.
    Select,
    /// Только просмотр: ↑↓ прокручивают, Enter пересчитывает.
    Viewer,
    /// Несколько вариантов с галочками: в списке Enter включает и выключает, Esc закрывает.
    Multi,
    /// Несколько значений: Enter добавляет введённое, Backspace в пустой строке убирает последнее.
    List,
    /// Разметка дисков: Enter открывает редактор.
    Layout,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub id: Id,
    pub label: &'static str,
    pub hint: &'static str,
    pub kind: Kind,
    pub input: Input,
    pub options: Vec<String>,
    pub index: usize,
    pub items: Vec<String>,
    pub suggestions: Vec<String>,
    pub checked: Vec<bool>,
}

impl Field {
    fn new(id: Id, label: &'static str, hint: &'static str, kind: Kind) -> Self {
        Self {
            id,
            label,
            hint,
            kind,
            input: Input::default(),
            options: Vec::new(),
            index: 0,
            items: Vec::new(),
            suggestions: Vec::new(),
            checked: Vec::new(),
        }
    }

    fn text(id: Id, label: &'static str, hint: &'static str, value: &str) -> Self {
        Self {
            input: Input::with(value),
            ..Self::new(id, label, hint, Kind::Text)
        }
    }

    fn secret(id: Id, label: &'static str, hint: &'static str) -> Self {
        Self::new(id, label, hint, Kind::Secret)
    }

    fn select(
        id: Id,
        label: &'static str,
        hint: &'static str,
        options: Vec<String>,
        index: usize,
    ) -> Self {
        Self {
            options,
            index,
            ..Self::new(id, label, hint, Kind::Select)
        }
    }

    fn options(
        id: Id,
        label: &'static str,
        hint: &'static str,
        options: &[&str],
        index: usize,
    ) -> Self {
        Self::select(
            id,
            label,
            hint,
            options.iter().map(ToString::to_string).collect(),
            index,
        )
    }

    /// Список с галочками: `ids` — что вернётся в конфиг, `labels` — что видно в списке.
    fn multi(
        id: Id,
        label: &'static str,
        hint: &'static str,
        ids: Vec<String>,
        labels: Vec<String>,
        checked: Vec<bool>,
    ) -> Self {
        Self {
            options: labels,
            items: ids,
            checked,
            ..Self::new(id, label, hint, Kind::Multi)
        }
    }

    pub fn display(&self) -> String {
        if self.kind != Kind::Multi {
            return self.choice_text().to_string();
        }

        let chosen = self.chosen();

        if chosen.is_empty() {
            tt!("nothing chosen", "ничего не выбрано").into()
        } else {
            tr!(
                "{} of {}: {}",
                "{} из {}: {}",
                chosen.len(),
                self.items.len(),
                chosen.join(", ")
            )
        }
    }

    pub fn chosen(&self) -> Vec<String> {
        self.items
            .iter()
            .zip(&self.checked)
            .filter(|(_, on)| **on)
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn suggesting(self, suggestions: Vec<String>) -> Self {
        Self {
            suggestions,
            ..self
        }
    }

    fn viewer(id: Id, label: &'static str, hint: &'static str) -> Self {
        Self::new(id, label, hint, Kind::Viewer)
    }

    fn list(id: Id, label: &'static str, hint: &'static str) -> Self {
        Self::new(id, label, hint, Kind::List)
    }

    fn layout(id: Id, label: &'static str, hint: &'static str) -> Self {
        Self::new(id, label, hint, Kind::Layout)
    }

    pub fn on(&self) -> bool {
        self.index == 0
    }

    pub fn value(&self) -> &str {
        self.input.text()
    }

    pub fn choice_text(&self) -> &str {
        self.options.get(self.index).map_or("", String::as_str)
    }
}

fn network_modes() -> [&'static str; 3] {
    tt!(["none", "DHCP", "manual"], ["нет", "DHCP", "вручную"])
}

fn same_disk() -> &'static str {
    tt!("Same disk", "Тот же диск")
}

fn schemes() -> [&'static str; 4] {
    tt!(
        [
            "One disk: a partition for the system",
            "LVM on the chosen disks",
            "RAID + LVM on the chosen disks",
            "Custom layout (line editor)",
        ],
        [
            "Один диск: раздел под систему",
            "LVM на выбранных дисках",
            "RAID + LVM на выбранных дисках",
            "Своя разметка (редактор строк)",
        ]
    )
}

const CUSTOM: usize = 3;
const LEVELS: [Level; 5] = [
    Level::Raid1,
    Level::Raid0,
    Level::Raid5,
    Level::Raid6,
    Level::Raid10,
];
fn level_names() -> [&'static str; 5] {
    tt!(
        [
            "RAID1: mirror",
            "RAID0: striping without protection",
            "RAID5: one disk may fail",
            "RAID6: two disks may fail",
            "RAID10: striped mirrors",
        ],
        [
            "RAID1: зеркало",
            "RAID0: чередование без защиты",
            "RAID5: один диск может выйти из строя",
            "RAID6: два диска могут выйти из строя",
            "RAID10: зеркала с чередованием",
        ]
    )
}

fn yes_no() -> [&'static str; 2] {
    tt!(["Yes", "No"], ["Да", "Нет"])
}

const FIRMWARES: [&str; 4] = ["UEFI", "BIOS", "U-Boot", "BIOS+UEFI"];
fn unlocks() -> [&'static str; 2] {
    tt!(
        ["passphrase typed by hand", "key file in the initramfs"],
        ["парольная фраза вручную", "ключ-файл в initramfs"]
    )
}

fn headers() -> [&'static str; 2] {
    tt!(
        [
            "in the partition (plain LUKS)",
            "separate file in the initramfs"
        ],
        ["в разделе (обычный LUKS)", "отдельный файл в initramfs"]
    )
}

const BACKENDS: [&str; 2] = ["systemd-networkd", "NetworkManager"];

/// Всё, что пользователь ввёл в мастере; в `Config` превращается в `to_config`.
#[derive(Debug, Clone)]
pub struct Draft {
    pub fields: Vec<Field>,
    pub disks: Vec<Disk>,
    pub repos: Vec<Repo>,
    pub hardware: Hardware,
}

impl Draft {
    pub fn new(sources: Sources) -> Self {
        let Sources {
            disks,
            mut timezones,
            interfaces,
            repos,
            hardware,
        } = sources;

        if timezones.is_empty() {
            timezones.push("UTC".into());
        }

        let mut fields = Self::disk_fields(&disks, hardware);

        fields.extend(Self::repo_fields(&repos));
        fields.extend(Self::system_fields(timezones));
        fields.extend(Self::encryption_fields());
        fields.extend(Self::network_fields(interfaces));
        fields.extend(Self::closing_fields());

        let mut draft = Self {
            fields,
            disks,
            repos,
            hardware,
        };

        draft.sync();
        draft
    }

    fn disk_fields(disks: &[Disk], hardware: Hardware) -> Vec<Field> {
        let disk = Field::select(
            Id::Disk,
            tt!("System disk (root)", "Диск для системы (root)"),
            tt!(
                "The root partition goes here; the data on the disk will be erased",
                "Здесь будет корневой раздел; данные на диске будут стёрты"
            ),
            disks.iter().map(Disk::label).collect(),
            0,
        );
        let boot_disk = Field::select(
            Id::BootDisk,
            tt!(
                "Disk for the kernel and bootloader",
                "Диск для ядра и загрузчика"
            ),
            tt!(
                "A USB stick can be chosen: the system will not boot without it",
                "Можно выбрать флешку: без неё система не загрузится"
            ),
            std::iter::once(same_disk().to_string())
                .chain(disks.iter().map(Disk::label))
                .collect(),
            0,
        );

        let scheme = Field::options(
            Id::Scheme,
            tt!("Disk scheme", "Схема дисков"),
            tt!(
                "How the storage is built: a plain partition, LVM, RAID with LVM or a custom layout",
                "Как устроено хранилище: обычный раздел, LVM, RAID с LVM или своя разметка"
            ),
            &schemes(),
            0,
        );
        let data_disks = Field::multi(
            Id::DataDisks,
            tt!("Disks for the system", "Диски для системы"),
            tt!(
                "Enter opens the list, inside it Enter toggles; all chosen disks are joined into one LVM group",
                "Enter — открыть список, внутри Enter включает и выключает; все выбранные объединяются в одну группу LVM"
            ),
            disks.iter().map(|disk| disk.path.clone()).collect(),
            disks.iter().map(Disk::label).collect(),
            (0..disks.len()).map(|index| index == 0).collect(),
        );
        let raid_level = Field::options(
            Id::RaidLevel,
            tt!("RAID level", "Уровень RAID"),
            tt!(
                "The array is built from partitions on the chosen disks and handed to the LVM group",
                "Массив собирается из разделов на выбранных дисках и отдаётся группе LVM"
            ),
            &level_names(),
            0,
        );

        let boot_size = Field::text(
            Id::BootSize,
            tt!("/boot size, MiB", "Размер /boot, MiB"),
            tt!(
                "From 256. A FAT32 partition that is also the ESP; a kernel with its initramfs takes 100–200 MiB, 512 fits several kernels",
                "От 256. Раздел FAT32 и он же ESP; ядро с initramfs занимает 100–200 MiB, 512 хватает на несколько ядер"
            ),
            &layout::BOOT_SIZE.to_string(),
        );
        let layout = Field::layout(
            Id::Layout,
            tt!("Layout", "Разметка"),
            tt!(
                "Enter opens the editor of disks, partitions and volumes; editing switches to \"Custom layout\"",
                "Enter — редактор дисков, разделов и томов; правка переключает на «Своя разметка»"
            ),
        );

        vec![
            scheme,
            disk,
            data_disks,
            raid_level,
            boot_disk,
            boot_size,
            layout,
            Field::options(
                Id::Firmware,
                tt!("Firmware", "Прошивка"),
                tt!("How the computer boots", "Как загружается компьютер"),
                &FIRMWARES,
                match hardware.firmware {
                    Firmware::Uefi => 0,
                    Firmware::Bios => 1,
                    Firmware::Uboot => 2,
                    Firmware::Hybrid => 3,
                },
            ),
            Field::text(
                Id::UbootImage,
                tt!("U-Boot image", "Образ U-Boot"),
                tt!(
                    "A file to write to the boot disk; empty means U-Boot is already on the medium",
                    "Файл для записи на диск загрузки; пусто — U-Boot уже на носителе"
                ),
                "",
            ),
            Field::text(
                Id::UbootOffset,
                tt!("Image offset", "Смещение образа"),
                tt!(
                    "In 512-byte sectors, below 32768",
                    "В секторах по 512 байт, меньше 32768"
                ),
                "0",
            ),
        ]
    }

    fn repo_fields(repos: &[Repo]) -> Vec<Field> {
        vec![
            Field::multi(
                Id::Repos,
                tt!("Repositories", "Репозитории"),
                tt!(
                    "Enter opens the list, inside it Enter toggles",
                    "Enter — открыть список, внутри Enter включает и выключает"
                ),
                repos.iter().map(|repo| repo.id.clone()).collect(),
                repos.iter().map(Repo::label).collect(),
                repos.iter().map(|repo| repo.enabled).collect(),
            ),
            Field::list(
                Id::LocalRepos,
                tt!("Local package directories", "Локальные каталоги с пакетами"),
                tt!(
                    "An absolute path of a directory with rpm files and metadata (createrepo), Enter adds",
                    "Абсолютный путь каталога с rpm и метаданными (createrepo), Enter — добавить"
                ),
            ),
            Field::text(
                Id::Options,
                tt!("dnf options", "Параметры dnf"),
                tt!(
                    "Space-separated key=value, for example releasever=44 or tsflags=nodocs",
                    "Через пробел: ключ=значение, например releasever=44 или tsflags=nodocs"
                ),
                "",
            ),
            Field::text(
                Id::Packages,
                tt!("Extra packages", "Дополнительные пакеты"),
                tt!(
                    "Separated by spaces or commas: a name, @group or a pattern. The main ones are installed automatically",
                    "Через пробел или запятую: имя, @группа или шаблон. Основные ставятся сами"
                ),
                "",
            ),
            Field::options(
                Id::CopyRepos,
                tt!(
                    "Copy repositories to the system",
                    "Перенести репозитории в систему"
                ),
                tt!(
                    "Copy the .repo files of the chosen repositories into the installed system",
                    "Скопировать .repo выбранных репозиториев в установленную систему"
                ),
                &yes_no(),
                0,
            ),
        ]
    }

    fn system_fields(timezones: Vec<String>) -> Vec<Field> {
        let utc = timezones.iter().position(|zone| zone == "UTC").unwrap_or(0);

        vec![
            Field::text(
                Id::Hostname,
                tt!("Hostname", "Имя машины"),
                tt!("Latin letters, digits and \"-\"", "Латиница, цифры и «-»"),
                "moonshine",
            ),
            Field::text(
                Id::User,
                tt!("User", "Пользователь"),
                tt!(
                    "Lowercase Latin letters, digits, \"_\" and \"-\"",
                    "Строчные латинские буквы, цифры, «_» и «-»"
                ),
                "",
            ),
            Field::secret(
                Id::Password,
                tt!("Password", "Пароль"),
                tt!("User password", "Пароль пользователя"),
            ),
            Field::secret(
                Id::PasswordAgain,
                tt!("Password again", "Пароль ещё раз"),
                "",
            ),
            Field::select(
                Id::Timezone,
                tt!("Time zone", "Часовой пояс"),
                tt!(
                    "Enter opens the list, type inside it to search",
                    "Enter — список, внутри можно набирать для поиска"
                ),
                timezones,
                utc,
            ),
            Field::text(
                Id::Services,
                tt!("Services to enable", "Службы для включения"),
                tt!(
                    "Systemd units, separated by spaces or commas, for example sshd chronyd. The package must be installed",
                    "Юниты systemd через пробел или запятую, например sshd chronyd. Пакет должен быть установлен"
                ),
                "",
            ),
            Field::text(
                Id::KernelArgs,
                tt!("Kernel parameters", "Параметры ядра"),
                tt!(
                    "Space-separated, added to the kernel command line, for example quiet or rd.luks.allow-discards",
                    "Через пробел, добавляются в командную строку ядра, например quiet или rd.luks.allow-discards"
                ),
                "",
            ),
        ]
    }

    fn encryption_fields() -> Vec<Field> {
        vec![
            Field::options(
                Id::Encrypt,
                tt!("Encrypt the root (LUKS2)", "Шифровать корень (LUKS2)"),
                tt!(
                    "Other devices are encrypted in the layout editor: key c",
                    "Остальные устройства шифруются в редакторе разметки: клавиша c"
                ),
                &yes_no(),
                0,
            ),
            Field::secret(
                Id::Passphrase,
                tt!("Passphrase", "Парольная фраза"),
                tt!(
                    "One for all password devices; needed at boot",
                    "Одна на все устройства с паролем; нужна при загрузке"
                ),
            ),
            Field::secret(
                Id::PassphraseAgain,
                tt!("Passphrase again", "Фраза ещё раз"),
                "",
            ),
            Field::options(
                Id::Header,
                tt!("LUKS header", "Заголовок LUKS"),
                tt!(
                    "A detached header lives in /etc/luks and in the initramfs on the boot disk",
                    "Отдельный заголовок лежит в /etc/luks и в initramfs на диске загрузки"
                ),
                &headers(),
                0,
            ),
            Field::options(
                Id::Unlock,
                tt!("How to unlock the root", "Как открывать корень"),
                tt!(
                    "The key file lives in the initramfs on the boot disk, the header is a separate file",
                    "Ключ-файл лежит в initramfs на диске загрузки, заголовок отдельным файлом"
                ),
                &unlocks(),
                0,
            ),
            Field::list(
                Id::SshKeys,
                tt!(
                    "SSH keys for remote unlocking",
                    "SSH-ключи для удалённой разблокировки"
                ),
                tt!(
                    "A key line, Enter adds; needs a network in the initramfs",
                    "Строка ключа, Enter — добавить; нужна сеть в initramfs"
                ),
            ),
        ]
    }

    fn network_fields(interfaces: Vec<String>) -> Vec<Field> {
        vec![
            Field::options(
                Id::InitrdMode,
                tt!("Network in initramfs", "Сеть в initramfs"),
                tt!(
                    "Needed for unlocking over SSH",
                    "Нужна для разблокировки по SSH"
                ),
                &network_modes(),
                0,
            ),
            Field::text(
                Id::InitrdInterface,
                tt!("Interface", "Интерфейс"),
                tt!(
                    "Enter picks from the found ones; may be empty for DHCP",
                    "Enter — выбрать из найденных; для DHCP можно пусто"
                ),
                "",
            )
            .suggesting(interfaces.clone()),
            Field::text(
                Id::InitrdAddress,
                tt!("Address", "Адрес"),
                "192.168.1.10/24",
                "",
            ),
            Field::text(Id::InitrdGateway, tt!("Gateway", "Шлюз"), "192.168.1.1", ""),
            Field::list(Id::InitrdDns, "DNS", tt!("Enter adds", "Enter — добавить")),
            Field::options(
                Id::SystemMode,
                tt!("Network in system", "Сеть в системе"),
                tt!("After boot", "После загрузки"),
                &network_modes(),
                1,
            ),
            Field::options(
                Id::Backend,
                tt!("Network service", "Служба сети"),
                tt!(
                    "What to configure in the system",
                    "Что настраивать в системе"
                ),
                &BACKENDS,
                0,
            ),
            Field::text(
                Id::SystemInterface,
                tt!("Interface", "Интерфейс"),
                tt!(
                    "Enter picks from the found ones; may be empty for DHCP",
                    "Enter — выбрать из найденных; для DHCP можно пусто"
                ),
                "",
            )
            .suggesting(interfaces),
            Field::text(
                Id::SystemAddress,
                tt!("Address", "Адрес"),
                "192.168.1.10/24",
                "",
            ),
            Field::text(Id::SystemGateway, tt!("Gateway", "Шлюз"), "192.168.1.1", ""),
            Field::list(Id::SystemDns, "DNS", tt!("Enter adds", "Enter — добавить")),
        ]
    }

    fn closing_fields() -> Vec<Field> {
        vec![
            Field::viewer(
                Id::PackageList,
                tt!("Packages to install", "Пакеты к установке"),
                tt!(
                    "↑↓ scroll · Enter recalculates",
                    "↑↓ прокрутка · Enter пересчитать"
                ),
            ),
            Field::text(
                Id::Confirm,
                tt!("Confirmation", "Подтверждение"),
                tt!(
                    "Type YES to erase the disk and install",
                    "Введите YES, чтобы стереть диск и установить"
                ),
                "",
            ),
        ]
    }

    pub fn field(&self, id: Id) -> &Field {
        self.fields
            .iter()
            .find(|field| field.id == id)
            .expect(tt!("the field is in the list", "поле есть в списке"))
    }

    pub fn field_mut(&mut self, id: Id) -> &mut Field {
        self.fields
            .iter_mut()
            .find(|field| field.id == id)
            .expect(tt!("the field is in the list", "поле есть в списке"))
    }

    fn mode(&self, id: Id) -> usize {
        self.field(id).index
    }

    pub fn encrypted(&self) -> bool {
        self.layout().is_ok_and(|layout| layout.has_crypt())
    }

    /// Нужна ли парольная фраза: есть устройство с `crypt=pass`.
    pub fn typed(&self) -> bool {
        self.layout().is_ok_and(|layout| {
            layout.crypts().iter().any(|device| {
                matches!(
                    device.crypt,
                    Some(Crypt {
                        unlock: Unlock::Typed,
                        ..
                    })
                )
            })
        })
    }

    /// Шифрование корня в схемах без ручной правки: ключ-файл всегда с отдельным заголовком.
    fn root_crypt(&self, separate: bool) -> Option<Crypt> {
        let keyfile = self.field(Id::Unlock).index == 1;

        self.field(Id::Encrypt).on().then_some(Crypt {
            unlock: if keyfile {
                Unlock::Keyfile
            } else {
                Unlock::Typed
            },
            detached: keyfile || separate || self.field(Id::Header).index == 1,
        })
    }

    fn uboot(&self) -> Option<Uboot> {
        let image = self.field(Id::UbootImage).value().trim();

        (self.field(Id::Firmware).index == 2 && !image.is_empty()).then(|| Uboot {
            image: image.into(),
            offset: self
                .field(Id::UbootOffset)
                .value()
                .trim()
                .parse()
                .unwrap_or_default(),
        })
    }

    /// Путь, который не меняется между загрузками: у диска без разметки нет UUID.
    fn root_disk_stable_path(&self) -> Option<&str> {
        self.disks
            .get(self.field(Id::Disk).index)
            .map(|disk| disk.stable.as_str())
    }

    pub fn root_disk_path(&self) -> Option<&str> {
        self.disks
            .get(self.field(Id::Disk).index)
            .map(|disk| disk.path.as_str())
    }

    pub fn boot_disk_path(&self) -> Option<&str> {
        let index = self.field(Id::BootDisk).index.checked_sub(1)?;

        self.disks.get(index).map(|disk| disk.path.as_str())
    }

    pub fn scheme(&self) -> usize {
        self.field(Id::Scheme).index
    }

    fn firmware(&self) -> Firmware {
        match self.field(Id::Firmware).index {
            0 => Firmware::Uefi,
            1 => Firmware::Bios,
            2 => Firmware::Uboot,
            _ => Firmware::Hybrid,
        }
    }

    fn data_disks(&self) -> Vec<String> {
        self.field(Id::DataDisks).chosen()
    }

    fn raid_level(&self) -> Level {
        LEVELS[self.field(Id::RaidLevel).index.min(LEVELS.len() - 1)]
    }

    /// Разметка по выбранной схеме; для своей схемы — по строкам редактора.
    pub fn layout(&self) -> Result<Layout, String> {
        if self.scheme() == CUSTOM {
            return Layout::from_rows(&self.field(Id::Layout).items)
                .map_err(|error| error.to_string());
        }

        self.preset()
            .ok_or_else(|| tt!("no disk is chosen", "не выбран ни один диск").into())
    }

    fn size_of(&self, id: Id, default: u64) -> u64 {
        self.field(id).value().trim().parse().unwrap_or(default)
    }

    fn preset(&self) -> Option<Layout> {
        let firmware = self.firmware();
        let boot = self.size_of(Id::BootSize, layout::BOOT_SIZE);
        let sized = |layout: Layout| {
            let layout = layout.with_boot_size(boot);
            let crypt = self.root_crypt(layout.separate_boot());

            Some(layout.with_root_crypt(crypt))
        };

        match self.scheme() {
            0 => {
                let path = self.root_disk_path()?;
                let disk = match self.boot_disk_path() {
                    Some(_) => self.root_disk_stable_path()?,
                    None => path,
                };

                sized(Layout::simple(
                    disk,
                    self.boot_disk_path().unwrap_or(disk),
                    firmware,
                ))
            }
            scheme @ (1 | 2) => {
                let chosen = self.data_disks();
                let boot = self
                    .boot_disk_path()
                    .map(String::from)
                    .or_else(|| chosen.first().cloned())?;
                let raid = (scheme == 2).then(|| self.raid_level());

                sized(Layout::pooled(&chosen, &boot, raid, firmware))
            }
            _ => None,
        }
    }

    /// Строки редактора повторяют выбранную схему, пока разметку не начали править вручную.
    pub fn sync(&mut self) {
        if self.scheme() != CUSTOM {
            let rows = self.preset().map(|layout| layout.rows());

            self.field_mut(Id::Layout).items = rows.unwrap_or_default();
        }
    }

    pub fn is_custom(&self) -> bool {
        self.scheme() == CUSTOM
    }

    /// Разметка из редактора становится своей: схема переключается, строки остаются как были.
    pub fn set_layout(&mut self, layout: &Layout) {
        self.field_mut(Id::Layout).items = layout.rows();
        self.field_mut(Id::Scheme).index = CUSTOM;
    }

    pub fn disk_size(&self, path: &str) -> Option<u64> {
        self.disks
            .iter()
            .find(|disk| disk.path == path || disk.stable == path)
            .map(|disk| disk.size)
    }

    pub fn layout_problem(&self) -> Option<String> {
        let layout = match self.layout() {
            Ok(layout) => layout,
            Err(error) => return Some(error),
        };
        let firmware = self.firmware();

        layout
            .validate(firmware)
            .and_then(|()| layout.check_fit(firmware, |disk| self.disk_size(disk)))
            .err()
            .map(|error| error.to_string())
    }

    /// Ядро и загрузчик лежат вне дисков корня.
    pub fn separate(&self) -> bool {
        self.layout().is_ok_and(|layout| layout.separate_boot())
    }

    fn root_encryption(&self) -> bool {
        self.scheme() != CUSTOM && self.field(Id::Encrypt).on()
    }

    pub fn visible(&self, id: Id) -> bool {
        match id {
            Id::Disk => self.scheme() == 0,
            Id::DataDisks => matches!(self.scheme(), 1 | 2),
            Id::RaidLevel => self.scheme() == 2,
            Id::BootDisk | Id::BootSize | Id::Encrypt => self.scheme() != CUSTOM,
            Id::Passphrase | Id::PassphraseAgain => self.typed(),
            Id::Unlock => self.root_encryption(),
            Id::Header => {
                self.root_encryption() && !self.separate() && self.field(Id::Unlock).index == 0
            }
            Id::SshKeys => self.encrypted(),
            Id::UbootImage | Id::UbootOffset => self.field(Id::Firmware).index == 2,
            Id::InitrdInterface => self.mode(Id::InitrdMode) != 0,
            Id::InitrdAddress | Id::InitrdGateway | Id::InitrdDns => self.mode(Id::InitrdMode) == 2,
            Id::Backend | Id::SystemInterface => self.mode(Id::SystemMode) != 0,
            Id::SystemAddress | Id::SystemGateway | Id::SystemDns => self.mode(Id::SystemMode) == 2,
            _ => true,
        }
    }

    pub fn visible_fields(&self, page: Page) -> Vec<Id> {
        page.fields()
            .iter()
            .copied()
            .filter(|id| self.visible(*id))
            .collect()
    }

    /// Проверка принимаемого значения списка (ключ SSH, каталог копии, адрес DNS).
    pub fn check_item(id: Id, value: &str) -> Result<(), String> {
        let result = match id {
            Id::SshKeys => config::check_ssh_key(value),
            Id::LocalRepos => config::check_local_repo(value),
            _ => parse_ip(value).map(|_| ()),
        };

        result.map_err(|error| error.to_string())
    }

    pub fn error(&self, id: Id) -> Option<String> {
        if !self.visible(id) {
            return None;
        }

        let field = self.field(id);
        let text = field.value();
        let failed =
            |result: Result<(), crate::error::Error>| result.err().map(|error| error.to_string());

        match id {
            Id::UbootImage if !text.trim().is_empty() && !Path::new(text.trim()).is_file() => {
                Some(tr!("file not found: {text}", "файл не найден: {text}"))
            }
            Id::UbootOffset
                if !self.field(Id::UbootImage).value().trim().is_empty()
                    && text
                        .trim()
                        .parse::<u64>()
                        .map_or(true, |offset| offset >= config::UBOOT_RESERVED_SECTORS) =>
            {
                Some(tr!(
                    "offset: a number of sectors below {}",
                    "смещение: число секторов меньше {}",
                    config::UBOOT_RESERVED_SECTORS
                ))
            }
            Id::Disk if self.disks.is_empty() => {
                Some(tt!("no disks found", "дисков не найдено").into())
            }
            Id::DataDisks if self.data_disks().is_empty() => {
                Some(tt!("choose at least one disk", "выберите хотя бы один диск").into())
            }
            Id::BootDisk
                if self.scheme() == 0
                    && self.boot_disk_path().is_some()
                    && self.boot_disk_path() == self.root_disk_path() =>
            {
                Some(
                    tt!(
                        "the boot disk equals the system disk: choose \"Same disk\"",
                        "диск загрузки совпадает с диском системы: выберите «Тот же диск»"
                    )
                    .into(),
                )
            }
            Id::BootSize => size_error(text, layout::BOOT_MIN),
            Id::Layout => self.layout_problem(),
            Id::Repos if self.no_repositories() && self.repos.is_empty() => Some(
                tt!(
                    "/etc/yum.repos.d has no repositories: add a local directory",
                    "в /etc/yum.repos.d нет репозиториев: добавьте локальный каталог"
                )
                .into(),
            ),
            Id::Repos if self.no_repositories() => Some(
                tt!(
                    "enable at least one repository or add a local directory",
                    "включите хотя бы один репозиторий или добавьте локальный каталог"
                )
                .into(),
            ),
            Id::Hostname => failed(config::check_hostname(text)),
            Id::User => failed(config::check_user(text)),
            Id::Password => failed(config::check_password(text)),
            Id::PasswordAgain if text != self.field(Id::Password).value() => {
                Some(tt!("the passwords do not match", "пароли не совпадают").into())
            }
            Id::Packages => words(text)
                .iter()
                .find_map(|spec| config::check_package(spec).err())
                .map(|error| error.to_string()),
            Id::Options => words(text)
                .iter()
                .find_map(|option| config::check_option(option).err())
                .map(|error| error.to_string()),
            Id::Timezone => failed(config::check_timezone(field.choice_text())),
            Id::Services => words(text)
                .iter()
                .find_map(|name| config::check_service(name).err())
                .map(|error| error.to_string()),
            Id::KernelArgs => text
                .split_whitespace()
                .find_map(|arg| config::check_kernel_arg(arg).err())
                .map(|error| error.to_string()),
            Id::Passphrase if text.is_empty() => {
                Some(tt!("enter a passphrase", "введите парольную фразу").into())
            }
            Id::PassphraseAgain if text != self.field(Id::Passphrase).value() => {
                Some(tt!("the passphrases do not match", "фразы не совпадают").into())
            }
            Id::InitrdMode
                if !self.field(Id::SshKeys).items.is_empty()
                    && self.mode(Id::InitrdMode) == 0
                    && self.encrypted() =>
            {
                Some(
                    tt!(
                        "SSH keys need a network in the initramfs",
                        "ключам SSH нужна сеть в initramfs"
                    )
                    .into(),
                )
            }
            Id::InitrdInterface => interface_error(text, self.mode(Id::InitrdMode) == 2),
            Id::InitrdAddress => parse_address(text).err().map(|error| error.to_string()),
            Id::InitrdGateway => parse_ip(text).err().map(|error| error.to_string()),
            Id::SystemInterface => interface_error(text, self.mode(Id::SystemMode) == 2),
            Id::SystemAddress => parse_address(text).err().map(|error| error.to_string()),
            Id::SystemGateway => parse_ip(text).err().map(|error| error.to_string()),
            _ => None,
        }
    }

    pub fn page_errors(&self, page: Page) -> Vec<(Id, String)> {
        self.visible_fields(page)
            .into_iter()
            .filter_map(|id| self.error(id).map(|error| (id, error)))
            .collect()
    }

    pub fn first_error(&self) -> Option<(Page, Id, String)> {
        Page::ALL.iter().find_map(|page| {
            self.page_errors(*page)
                .into_iter()
                .next()
                .map(|(id, error)| (*page, id, error))
        })
    }

    pub fn to_config(&self) -> Result<Config, String> {
        if let Some((_, _, error)) = self.first_error() {
            return Err(error);
        }

        let layout = self.layout()?;
        let encrypted = layout.has_crypt();
        let passphrase = self
            .typed()
            .then(|| Secret::new(self.field(Id::Passphrase).value()));
        let keys = config::generate_keys(&layout).map_err(|error| error.to_string())?;
        let config = Config {
            layout,
            firmware: self.firmware(),
            uboot: self.uboot(),
            source: self.source(),
            hostname: self.field(Id::Hostname).value().into(),
            arch: self.hardware.arch,
            cpu: self.hardware.cpu,
            users: vec![User {
                name: self.field(Id::User).value().into(),
                password: Secret::new(self.field(Id::Password).value()),
                hashed: false,
            }],
            passphrase,
            keys,
            timezone: self.field(Id::Timezone).choice_text().into(),
            services: words(self.field(Id::Services).value()),
            kernel_args: self
                .field(Id::KernelArgs)
                .value()
                .split_whitespace()
                .map(String::from)
                .collect(),
            initrd_network: self.network(
                Id::InitrdMode,
                Id::InitrdInterface,
                Id::InitrdAddress,
                Id::InitrdGateway,
                Id::InitrdDns,
            )?,
            system_network: self.network(
                Id::SystemMode,
                Id::SystemInterface,
                Id::SystemAddress,
                Id::SystemGateway,
                Id::SystemDns,
            )?,
            backend: if self.field(Id::Backend).index == 0 {
                Backend::Networkd
            } else {
                Backend::NetworkManager
            },
            ssh_keys: if encrypted {
                self.field(Id::SshKeys).items.clone()
            } else {
                Vec::new()
            },
        };

        config.validate().map_err(|error| error.to_string())?;
        Ok(config)
    }

    fn source(&self) -> Source {
        let repos = self.field(Id::Repos).chosen();
        let mut copy: Vec<String> = Vec::new();

        if self.field(Id::CopyRepos).on() {
            for repo in self.repos.iter().filter(|repo| repos.contains(&repo.id)) {
                if !copy.contains(&repo.file) {
                    copy.push(repo.file.clone());
                }
            }
        }

        Source {
            repos,
            local: self.field(Id::LocalRepos).items.clone(),
            packages: words(self.field(Id::Packages).value()),
            options: words(self.field(Id::Options).value()),
            copy,
            ..Source::default()
        }
    }

    fn no_repositories(&self) -> bool {
        self.field(Id::Repos).chosen().is_empty() && self.field(Id::LocalRepos).items.is_empty()
    }

    fn network(
        &self,
        mode: Id,
        interface: Id,
        address: Id,
        gateway: Id,
        dns: Id,
    ) -> Result<Network, String> {
        let interface_name = self.field(interface).value().trim();
        let interface_name = (!interface_name.is_empty()).then(|| interface_name.to_string());

        match self.mode(mode) {
            0 => Ok(Network::None),
            1 => Ok(Network::Dhcp {
                interface: interface_name,
            }),
            _ => {
                let (address, prefix) = parse_address(self.field(address).value())
                    .map_err(|error| error.to_string())?;
                let dns = self
                    .field(dns)
                    .items
                    .iter()
                    .map(|server| parse_ip(server))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;

                Ok(Network::Manual(Manual {
                    address,
                    prefix,
                    gateway: parse_ip(self.field(gateway).value())
                        .map_err(|error| error.to_string())?,
                    dns,
                    interface: interface_name.ok_or(tt!(
                        "a manual network needs an interface",
                        "для ручной сети нужен интерфейс"
                    ))?,
                }))
            }
        }
    }
}

fn size_error(text: &str, minimum: u64) -> Option<String> {
    match text.trim().parse::<u64>() {
        Ok(size) if size >= minimum => None,
        _ => Some(tr!(
            "an integer number of MiB, at least {minimum}, is required",
            "нужно целое число MiB, не меньше {minimum}"
        )),
    }
}

/// Имена пакетов из строки: через пробелы и запятые.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|spec| !spec.is_empty())
        .map(String::from)
        .collect()
}

fn interface_error(text: &str, required: bool) -> Option<String> {
    let text = text.trim();

    if text.is_empty() {
        return required.then(|| {
            tt!(
                "a manual network needs an interface",
                "для ручной сети нужен интерфейс"
            )
            .to_string()
        });
    }

    Network::Dhcp {
        interface: Some(text.to_string()),
    }
    .validate()
    .err()
    .map(|error| error.to_string())
}

pub fn parse_ip(text: &str) -> Result<Ipv4Addr, crate::error::Error> {
    text.trim().parse().map_err(|_| {
        crate::error::Error::Invalid(tr!(
            "invalid IPv4 address: {text}",
            "неверный IPv4-адрес: {text}"
        ))
    })
}

pub fn parse_address(text: &str) -> Result<(Ipv4Addr, u8), crate::error::Error> {
    let invalid = || {
        crate::error::Error::Invalid(tr!(
            "an address like 192.168.1.10/24, not \"{text}\"",
            "адрес вида 192.168.1.10/24, а не «{text}»"
        ))
    };
    let (address, prefix) = text.trim().split_once('/').ok_or_else(invalid)?;
    let prefix: u8 = prefix.parse().map_err(|_| invalid())?;

    if prefix > 32 {
        return Err(invalid());
    }

    Ok((parse_ip(address)?, prefix))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn disk(path: &str) -> Disk {
        Disk {
            path: path.into(),
            size: 500 << 30,
            model: "Test".into(),
            removable: false,
            medium: crate::disks::Medium::Ssd,
            stable: format!("/dev/disk/by-id/{}", path.trim_start_matches("/dev/")),
        }
    }

    pub fn filled() -> Draft {
        let mut draft = Draft::new(Sources {
            disks: vec![disk("/dev/sda"), disk("/dev/sdb")],
            repos: vec![Repo {
                id: "evillin".into(),
                name: "Evillin".into(),
                enabled: true,
                file: "/etc/yum.repos.d/evillin.repo".into(),
            }],
            hardware: Hardware {
                firmware: Firmware::Uefi,
                ..Hardware::default()
            },
            ..Sources::default()
        });

        for (id, value) in [
            (Id::User, "alice"),
            (Id::Password, "pw"),
            (Id::PasswordAgain, "pw"),
            (Id::Passphrase, "phrase"),
            (Id::PassphraseAgain, "phrase"),
        ] {
            draft.field_mut(id).input = Input::with(value);
        }

        draft
    }

    #[test]
    fn a_filled_draft_becomes_a_valid_config() {
        let config = filled().to_config().unwrap();

        assert_eq!(
            (
                config.layout.root_device().as_str(),
                config.hostname.as_str(),
                config.users[0].name.as_str()
            ),
            ("/dev/sda2", "moonshine", "alice")
        );
        assert_eq!(config.source.repos, ["evillin"]);
        assert_eq!(config.firmware, Firmware::Uefi);
        assert!(config.passphrase.is_some() && config.keys.is_empty());
        assert_eq!(root_crypt(&config).map(|crypt| crypt.detached), Some(false));
        assert_eq!(config.initrd_network, Network::None);
        assert_eq!(config.system_network, Network::Dhcp { interface: None });
    }

    #[test]
    fn each_mistake_is_reported_on_its_own_field() {
        let mut draft = filled();

        draft.field_mut(Id::Hostname).input = Input::with("-bad");
        draft.field_mut(Id::PasswordAgain).input = Input::with("other");
        draft.field_mut(Id::Packages).input = Input::with("vim -bad");

        let errors: Vec<Id> = Page::ALL
            .iter()
            .flat_map(|page| draft.page_errors(*page))
            .map(|(id, _)| id)
            .collect();

        assert_eq!(errors, [Id::Packages, Id::Hostname, Id::PasswordAgain]);
        assert!(draft.to_config().is_err());
        assert_eq!(
            draft.first_error().map(|(page, id, _)| (page, id)),
            Some((Page::Repos, Id::Packages))
        );
    }

    #[test]
    fn hidden_fields_are_never_validated() {
        let mut draft = filled();

        draft.field_mut(Id::Encrypt).index = 1;
        draft.field_mut(Id::Passphrase).input = Input::with("");
        draft.field_mut(Id::PassphraseAgain).input = Input::with("different");

        assert!(!draft.visible(Id::Passphrase) && draft.error(Id::PassphraseAgain).is_none());

        let config = draft.to_config().unwrap();

        assert!(config.passphrase.is_none() && config.ssh_keys.is_empty());
    }

    fn root_crypt(config: &Config) -> Option<Crypt> {
        config.layout.crypt_of(&config.layout.root_device())
    }

    #[test]
    fn the_detached_header_reaches_the_config() {
        let mut draft = filled();

        draft.field_mut(Id::Header).index = 1;

        assert_eq!(
            root_crypt(&draft.to_config().unwrap()).map(|crypt| crypt.detached),
            Some(true)
        );
    }

    #[test]
    fn the_boot_disk_defaults_to_the_root_disk() {
        let draft = filled();
        let config = draft.to_config().unwrap();

        assert_eq!(draft.boot_disk_path(), None);
        assert_eq!(config.layout.boot_disk(), "/dev/sda");
        assert!(!config.layout.separate_boot());
    }

    #[test]
    fn a_separate_boot_disk_reaches_the_config() {
        let mut draft = filled();

        draft.field_mut(Id::BootDisk).index = 2;
        let config = draft.to_config().unwrap();

        assert_eq!(config.layout.root_device(), "/dev/disk/by-id/sda");
        assert_eq!(config.layout.boot_disk(), "/dev/sdb");
    }

    #[test]
    fn a_separate_boot_disk_forces_the_detached_header_and_offers_a_keyfile() {
        let mut draft = filled();

        assert!(draft.visible(Id::Header) && draft.visible(Id::Unlock));

        draft.field_mut(Id::BootDisk).index = 2;

        assert!(!draft.visible(Id::Header) && draft.visible(Id::Unlock));
        assert_eq!(
            root_crypt(&draft.to_config().unwrap()).map(|crypt| crypt.detached),
            Some(true)
        );
    }

    #[test]
    fn a_keyfile_replaces_the_typed_passphrase_with_a_random_key() {
        let mut draft = filled();

        draft.field_mut(Id::BootDisk).index = 2;
        draft.field_mut(Id::Unlock).index = 1;
        draft.field_mut(Id::Passphrase).input = Input::with("");

        let config = draft.to_config().unwrap();
        let root = config.layout.root_device();
        let key = config.secret(&root).unwrap();

        assert!(!draft.visible(Id::Passphrase) && config.passphrase.is_none());
        assert_eq!(
            root_crypt(&config),
            Some(Crypt {
                unlock: Unlock::Keyfile,
                detached: true
            })
        );
        assert_eq!(key.expose().len(), 64);
        assert_ne!(
            key.expose(),
            draft.to_config().unwrap().secret(&root).unwrap().expose()
        );
    }

    #[test]
    fn uboot_fields_appear_only_for_uboot_and_reach_the_config() {
        let mut draft = filled();

        assert!(!draft.visible(Id::UbootImage));

        let image =
            std::env::temp_dir().join(format!("moonshine-uboot-{}.bin", std::process::id()));

        std::fs::write(&image, "x").unwrap();
        draft.hardware.arch = crate::config::Arch::Aarch64;
        draft.field_mut(Id::Firmware).index = 2;
        draft.field_mut(Id::UbootImage).input = Input::with(image.to_str().unwrap());
        draft.field_mut(Id::UbootOffset).input = Input::with("64");

        let config = draft.to_config().unwrap();

        assert_eq!(config.firmware, Firmware::Uboot);
        assert_eq!(
            config.uboot,
            Some(Uboot {
                image: image.to_str().unwrap().into(),
                offset: 64
            })
        );

        draft.field_mut(Id::UbootOffset).input = Input::with("99999");

        assert!(draft.error(Id::UbootOffset).is_some());
    }

    #[test]
    fn the_same_disk_chosen_as_a_separate_boot_disk_is_an_error() {
        let mut draft = filled();

        draft.field_mut(Id::BootDisk).index = 1;

        assert!(draft.error(Id::BootDisk).is_some());
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn a_manual_network_needs_all_its_parts_and_reaches_the_config() {
        let mut draft = filled();

        draft.field_mut(Id::InitrdMode).index = 2;
        assert!(
            draft.to_config().is_err(),
            "адрес, шлюз и интерфейс пока пусты"
        );

        for (id, value) in [
            (Id::InitrdInterface, "eth0"),
            (Id::InitrdAddress, "10.0.0.5/16"),
            (Id::InitrdGateway, "10.0.0.1"),
        ] {
            draft.field_mut(id).input = Input::with(value);
        }

        draft.field_mut(Id::InitrdDns).items = vec!["1.1.1.1".into()];

        let Network::Manual(manual) = draft.to_config().unwrap().initrd_network else {
            panic!("ожидалась ручная сеть")
        };

        assert_eq!(
            (manual.prefix, manual.interface.as_str(), manual.dns.len()),
            (16, "eth0", 1)
        );
    }

    #[test]
    fn services_are_split_by_spaces_or_commas_and_checked() {
        let mut draft = filled();

        draft.field_mut(Id::Services).input = Input::with("sshd, chronyd  getty@tty1");

        assert!(draft.error(Id::Services).is_none());
        assert_eq!(
            draft.to_config().unwrap().services,
            ["sshd", "chronyd", "getty@tty1"]
        );

        draft.field_mut(Id::Services).input = Input::with("sshd bad;name");

        assert!(draft.error(Id::Services).is_some());
    }

    #[test]
    fn kernel_parameters_are_split_by_spaces_and_checked() {
        let mut draft = filled();

        draft.field_mut(Id::KernelArgs).input = Input::with("quiet  console=ttyS0,115200");

        assert!(draft.error(Id::KernelArgs).is_none());
        assert_eq!(
            draft.to_config().unwrap().kernel_args,
            ["quiet", "console=ttyS0,115200"]
        );

        draft.field_mut(Id::KernelArgs).input = Input::with("quiet a$b");

        assert!(draft.error(Id::KernelArgs).is_some());
    }

    #[test]
    fn ssh_keys_without_an_initramfs_network_are_an_error_of_the_network_page() {
        let mut draft = filled();

        draft.field_mut(Id::SshKeys).items = vec!["ssh-ed25519 AAAA admin".into()];

        assert_eq!(
            draft.error(Id::InitrdMode).as_deref(),
            Some("ключам SSH нужна сеть в initramfs")
        );

        draft.field_mut(Id::InitrdMode).index = 1;

        assert!(draft.error(Id::InitrdMode).is_none() && draft.to_config().is_ok());
    }

    #[test]
    fn list_items_are_checked_when_added() {
        assert!(Draft::check_item(Id::SshKeys, "ssh-ed25519 AAAA").is_ok());
        assert!(Draft::check_item(Id::SshKeys, "  ").is_err());
        assert!(Draft::check_item(Id::InitrdDns, "8.8.8.8").is_ok());
        assert!(Draft::check_item(Id::SystemDns, "dns.example").is_err());
    }

    #[test]
    fn the_timezone_is_chosen_from_the_list_and_utc_is_the_default() {
        let mut draft = Draft::new(Sources {
            timezones: vec!["Asia/Tokyo".into(), "Europe/Moscow".into(), "UTC".into()],
            ..Sources::default()
        });

        assert_eq!(draft.field(Id::Timezone).choice_text(), "UTC");

        draft.field_mut(Id::Timezone).index = 1;

        assert_eq!(draft.field(Id::Timezone).choice_text(), "Europe/Moscow");
        assert_eq!(
            Draft::new(Sources::default())
                .field(Id::Timezone)
                .choice_text(),
            "UTC"
        );
    }

    #[test]
    fn found_interfaces_become_suggestions() {
        let draft = Draft::new(Sources {
            interfaces: vec!["eth0".into()],
            ..Sources::default()
        });

        assert_eq!(draft.field(Id::InitrdInterface).suggestions, ["eth0"]);
        assert_eq!(draft.field(Id::SystemInterface).suggestions, ["eth0"]);
    }

    fn repo(id: &str, file: &str, enabled: bool) -> Repo {
        Repo {
            id: id.into(),
            name: format!("{id} name"),
            enabled,
            file: file.into(),
        }
    }

    fn with_repos() -> Draft {
        let mut draft = Draft::new(Sources {
            disks: vec![disk("/dev/sda")],
            repos: vec![
                repo("baseos", "/etc/yum.repos.d/a.repo", true),
                repo("appstream", "/etc/yum.repos.d/a.repo", true),
                repo("debug", "/etc/yum.repos.d/b.repo", false),
                repo("moon", "/etc/yum.repos.d/moon.repo", true),
            ],
            ..Sources::default()
        });

        for (id, value) in [
            (Id::User, "alice"),
            (Id::Password, "pw"),
            (Id::PasswordAgain, "pw"),
            (Id::Passphrase, "phrase"),
            (Id::PassphraseAgain, "phrase"),
        ] {
            draft.field_mut(id).input = Input::with(value);
        }

        draft
    }

    #[test]
    fn repositories_are_read_with_their_enabled_state() {
        let draft = with_repos();

        assert_eq!(
            draft.field(Id::Repos).chosen(),
            ["baseos", "appstream", "moon"]
        );
        assert_eq!(
            draft.field(Id::Options).value(),
            "",
            "по умолчанию строка параметров пуста"
        );
        assert!(draft.visible(Id::Repos) && draft.visible(Id::Packages));
        assert!(
            draft
                .field(Id::Repos)
                .display()
                .starts_with("3 из 4: baseos")
        );
    }

    #[test]
    fn the_package_source_reaches_the_config_with_the_repo_files_to_copy() {
        let mut draft = with_repos();

        draft.field_mut(Id::Packages).input = Input::with("vim-enhanced, @sound  mc");
        draft.field_mut(Id::Options).input =
            Input::with("tsflags=nodocs weak_deps=False releasever=44");
        draft.field_mut(Id::Repos).checked[1] = false;

        let Source {
            repos,
            packages,
            options,
            copy,
            ..
        } = draft.to_config().unwrap().source;

        assert_eq!(repos, ["baseos", "moon"]);
        assert_eq!(packages, ["vim-enhanced", "@sound", "mc"]);
        assert_eq!(
            options,
            ["tsflags=nodocs", "weak_deps=False", "releasever=44"]
        );
        assert_eq!(
            copy,
            ["/etc/yum.repos.d/a.repo", "/etc/yum.repos.d/moon.repo"]
        );
    }

    #[test]
    fn copying_can_be_switched_off_and_the_options_can_be_emptied() {
        let mut draft = with_repos();

        draft.field_mut(Id::CopyRepos).index = 1;
        draft.field_mut(Id::Options).input = Input::with("");

        let Source { options, copy, .. } = draft.to_config().unwrap().source;

        assert!(copy.is_empty() && options.is_empty());
    }

    #[test]
    fn options_are_checked_word_by_word() {
        let mut draft = with_repos();

        assert!(draft.error(Id::Options).is_none());

        for bad in [
            "nodocs",
            "tsflags=nodocs =x",
            "a;b=1",
            "k=$(reboot)",
            "k=`id`",
            "k=a|b",
        ] {
            draft.field_mut(Id::Options).input = Input::with(bad);

            assert!(draft.error(Id::Options).is_some(), "{bad}");
        }

        draft.field_mut(Id::Options).input =
            Input::with("releasever=44, exclude=kernel* proxy=http://h:3128");

        assert!(draft.error(Id::Options).is_none());
    }

    #[test]
    fn at_least_one_repository_must_be_on() {
        let mut draft = with_repos();

        draft.field_mut(Id::Repos).checked.fill(false);

        assert_eq!(
            draft.error(Id::Repos).as_deref(),
            Some("включите хотя бы один репозиторий или добавьте локальный каталог")
        );
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn without_repositories_a_local_directory_is_the_way_out() {
        let mut draft = Draft::new(Sources::default());

        assert_eq!(
            draft.error(Id::Repos).as_deref(),
            Some("в /etc/yum.repos.d нет репозиториев: добавьте локальный каталог")
        );

        draft.field_mut(Id::LocalRepos).items = vec!["/srv/repo".into()];

        assert!(draft.error(Id::Repos).is_none());
        assert_eq!(draft.source().local, ["/srv/repo"]);
    }

    #[test]
    fn local_directories_are_checked_when_added() {
        assert!(Draft::check_item(Id::LocalRepos, "/srv/repo").is_ok());
        assert!(Draft::check_item(Id::LocalRepos, "repo").is_err());
    }

    #[test]
    fn extra_packages_are_one_line_split_by_spaces_and_commas() {
        assert_eq!(
            words(" vim,mc  @sound\tgit "),
            ["vim", "mc", "@sound", "git"]
        );
        assert!(words("  ,, ").is_empty());

        let mut draft = with_repos();

        assert!(draft.error(Id::Packages).is_none());

        draft.field_mut(Id::Packages).input = Input::with("vim -bad");
        assert!(draft.error(Id::Packages).is_some() && draft.to_config().is_err());

        draft.field_mut(Id::Packages).input = Input::with("vim a;b");
        assert!(draft.error(Id::Packages).is_some());
    }

    #[test]
    fn without_disks_there_is_nothing_to_install_to() {
        let draft = Draft::new(Sources::default());

        assert_eq!(draft.error(Id::Disk).as_deref(), Some("дисков не найдено"));
        assert_eq!(draft.field(Id::Firmware).choice_text(), "BIOS");
    }

    #[test]
    fn addresses_need_a_prefix_not_larger_than_32() {
        assert_eq!(
            parse_address("10.0.0.5/8").unwrap(),
            ("10.0.0.5".parse().unwrap(), 8)
        );
        assert!(parse_address("10.0.0.5").is_err());
        assert!(parse_address("10.0.0.5/33").is_err());
        assert!(parse_address("x/8").is_err());
    }

    fn choose(draft: &mut Draft, id: Id, paths: &[&str]) {
        let field = draft.field_mut(id);

        field.checked = field
            .items
            .iter()
            .map(|path| paths.contains(&path.as_str()))
            .collect();
    }

    #[test]
    fn the_simple_scheme_shows_its_partitions_as_rows() {
        let draft = filled();

        assert_eq!(draft.scheme(), 0);
        assert_eq!(
            draft.field(Id::Layout).items,
            [
                "partition disk=/dev/sda,size=512MiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/,crypt=pass",
            ]
        );
    }

    #[test]
    fn lvm_on_several_disks_pools_them_into_one_group() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 1;
        choose(&mut draft, Id::DataDisks, &["/dev/sda", "/dev/sdb"]);
        draft.sync();

        let config = draft.to_config().unwrap();

        assert_eq!(config.layout.groups(), ["moonshine"]);
        assert_eq!(
            config.layout.volumes_of("moonshine"),
            ["/dev/sda2", "/dev/sdb1"]
        );
        assert_eq!(config.layout.root_device(), "/dev/moonshine/root");
        assert_eq!(config.layout.boot_disk(), "/dev/sda");
        assert!(!config.layout.separate_boot());
        assert!(
            draft
                .field(Id::Layout)
                .items
                .iter()
                .any(|row| row.starts_with("volume "))
        );
    }

    #[test]
    fn raid_needs_enough_disks_and_the_level_comes_from_the_list() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 2;
        choose(&mut draft, Id::DataDisks, &["/dev/sda"]);
        assert!(draft.error(Id::Layout).is_some());

        choose(&mut draft, Id::DataDisks, &["/dev/sda", "/dev/sdb"]);
        assert_eq!(draft.error(Id::Layout), None);
        assert_eq!(
            draft.to_config().unwrap().layout.raids[0].level,
            Level::Raid1
        );

        draft.field_mut(Id::RaidLevel).index = 4;
        assert!(draft.error(Id::Layout).is_some());

        draft.field_mut(Id::RaidLevel).index = 1;
        assert_eq!(
            draft.to_config().unwrap().layout.raids[0].level,
            Level::Raid0
        );
    }

    #[test]
    fn at_least_one_data_disk_is_required() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 1;
        choose(&mut draft, Id::DataDisks, &[]);

        assert!(draft.error(Id::DataDisks).is_some());
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn the_fields_follow_the_scheme() {
        let mut draft = filled();
        let shown = |draft: &Draft| draft.visible_fields(Page::Disk);

        assert!(shown(&draft).contains(&Id::Disk));
        assert!(!shown(&draft).contains(&Id::DataDisks));

        draft.field_mut(Id::Scheme).index = 2;
        assert!(shown(&draft).contains(&Id::DataDisks) && shown(&draft).contains(&Id::RaidLevel));
        assert!(!shown(&draft).contains(&Id::Disk));

        draft.field_mut(Id::Scheme).index = 3;
        assert!(!shown(&draft).contains(&Id::BootDisk));
        assert!(shown(&draft).contains(&Id::Layout));
    }

    #[test]
    fn custom_rows_are_checked_as_a_whole_layout() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 3;
        draft.field_mut(Id::Layout).items = vec![
            "partition disk=/dev/sda,size=1GiB,point=boot".into(),
            "partition disk=/dev/sda,size=rest,point=/".into(),
            "partition disk=/dev/sdb,size=whole,point=/data".into(),
        ];
        assert_eq!(draft.error(Id::Layout), None);
        assert_eq!(draft.to_config().unwrap().layout.partitions.len(), 3);

        draft.field_mut(Id::Layout).items.pop();
        draft.field_mut(Id::Layout).items.pop();
        assert!(draft.error(Id::Layout).is_some());
        assert!(draft.to_config().is_err());
    }

    #[test]
    fn a_layout_larger_than_the_disks_is_refused_in_the_wizard() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 3;
        draft.field_mut(Id::Layout).items = vec![
            "partition disk=/dev/sda,size=1GiB,point=boot".into(),
            "partition disk=/dev/sda,size=900GiB,point=/".into(),
        ];

        assert!(draft.error(Id::Layout).unwrap().contains("/dev/sda"));

        draft.field_mut(Id::Layout).items[1] = "partition disk=/dev/nope,size=rest,point=/".into();
        assert!(draft.error(Id::Layout).is_some());
    }

    #[test]
    fn an_outside_boot_disk_in_a_preset_asks_for_the_detached_header() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 1;
        choose(&mut draft, Id::DataDisks, &["/dev/sda"]);
        draft.field_mut(Id::BootDisk).index = 2;

        assert!(draft.separate());
        assert!(draft.visible(Id::Unlock) && !draft.visible(Id::Header));
        assert_eq!(
            root_crypt(&draft.to_config().unwrap()).map(|crypt| crypt.detached),
            Some(true)
        );
    }

    #[test]
    fn a_custom_layout_encrypts_in_the_editor_and_the_wizard_asks_only_for_the_phrase() {
        let mut draft = filled();

        draft.field_mut(Id::Scheme).index = 3;
        draft.field_mut(Id::Layout).items = vec![
            "partition disk=/dev/sda,size=1GiB,point=boot".into(),
            "partition disk=/dev/sda,size=rest,point=/,crypt=key,header=detached".into(),
            "partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key,header=detached".into(),
        ];

        assert!(!draft.visible(Id::Encrypt) && !draft.visible(Id::Unlock));
        assert!(!draft.visible(Id::Header) && !draft.visible(Id::Passphrase));
        assert!(draft.encrypted() && !draft.typed());

        let config = draft.to_config().unwrap();

        assert!(config.passphrase.is_none());
        assert_eq!(config.keys.len(), 2);
        assert_eq!(config.keys[1].0, "/dev/sdb");

        draft.field_mut(Id::Layout).items[2] =
            "partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=pass".into();

        assert!(draft.visible(Id::Passphrase) && draft.typed());
        assert_eq!(draft.to_config().unwrap().keys.len(), 1);
    }

    #[test]
    fn the_boot_size_comes_from_the_field_and_is_checked() {
        let mut draft = filled();

        draft.field_mut(Id::BootSize).input = Input::with("300");
        assert_eq!(
            draft.to_config().unwrap().layout.partitions[0].size,
            layout::Size::Mib(300)
        );

        for bad in ["255", "1g", "", "-1"] {
            draft.field_mut(Id::BootSize).input = Input::with(bad);
            assert!(draft.error(Id::BootSize).is_some(), "{bad:?}");
            assert!(draft.to_config().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_boot_size_is_hidden_in_custom_layouts() {
        let mut draft = filled();

        assert!(draft.visible(Id::BootSize));

        draft.field_mut(Id::Scheme).index = 3;
        assert!(!draft.visible(Id::BootSize));
    }
}
