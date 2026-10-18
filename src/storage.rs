use crate::config::Config;
use crate::config::Firmware;
use crate::config::Ids;
use crate::config::Unlock;
use crate::layout::Device;
use crate::layout::Fs;
use crate::layout::Layout;
use crate::layout::Size;
use crate::layout::Use;
use crate::layout::Volume;
use crate::layout::raid_path;
use crate::layout::volume_path;
use crate::plan::Command;
use crate::plan::HEADER_SIZE;
use crate::plan::Input;
use crate::plan::Step;
use crate::plan::TARGET;
use crate::plan::UBOOT_START;
use crate::plan::file;
use crate::plan::header_path;
use crate::plan::header_temp;
use crate::plan::key_path;
use crate::plan::run;

pub(crate) const TYPE_BIOS_BOOT: &str = "21686148-6449-6E6F-744E-656564454649";
pub(crate) const TYPE_ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const TYPE_LINUX: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";
pub(crate) const TYPE_SWAP: &str = "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F";
const TYPE_RAID: &str = "A19D880F-05FC-4D3B-A006-743F0F84911E";
pub(crate) const TYPE_LVM: &str = "E6D6D379-F507-44C2-A23C-238F2A3DF928";

/// Откуда `crypttab` берёт зашифрованное устройство: с заголовком на нём — `UUID` LUKS, иначе раздел по `PARTUUID` или постоянный путь.
pub fn crypt_source(config: &Config, ids: &Ids, path: &str) -> String {
    let layout = &config.layout;
    let detached = layout.crypt_of(path).is_some_and(|crypt| crypt.detached);

    if !detached {
        return format!("UUID={}", ids.luks_of(path));
    }

    if on_partition(layout, path) {
        format!("PARTUUID={}", ids.partuuid(path))
    } else {
        path.into()
    }
}

fn on_partition(layout: &Layout, path: &str) -> bool {
    (0..layout.partitions.len()).any(|index| {
        layout.partitions[index].size != Size::Whole && layout.partition_device(index) == path
    })
}

fn type_guid(usage: &Use, firmware: Firmware) -> &'static str {
    match usage {
        Use::Boot if firmware.has_efi() => TYPE_ESP,
        Use::BiosBoot => TYPE_BIOS_BOOT,
        Use::Swap => TYPE_SWAP,
        Use::Raid(_) => TYPE_RAID,
        Use::Lvm(_) => TYPE_LVM,
        Use::Boot | Use::Reserve | Use::Mount { .. } | Use::Pool => TYPE_LINUX,
    }
}

pub fn partitioning(config: &Config, ids: &Ids) -> Vec<Step> {
    let disks = config.layout.disks();
    let mut steps = vec![Step::Release(disks.clone())];

    for disk in disks {
        steps.push(run("wipefs", ["--all", &disk]));

        if let Some(script) = table(config, ids, &disk) {
            steps.push(Step::Run(
                Command::new("sfdisk", [&disk]).input(Input::Script(script)),
            ));
        }
    }

    steps.push(run("udevadm", ["settle"]));
    steps
}

/// Таблица GPT для `sfdisk`; у диска целиком (`whole`) таблицы нет.
fn table(config: &Config, ids: &Ids, disk: &str) -> Option<String> {
    let layout = &config.layout;
    let indexes: Vec<usize> = (0..layout.partitions.len())
        .filter(|&index| {
            layout.partitions[index].disk == disk && layout.partitions[index].size != Size::Whole
        })
        .collect();

    if indexes.is_empty() {
        return None;
    }

    let mut script = String::from("label: gpt\n");

    for (position, &index) in indexes.iter().enumerate() {
        let part = &layout.partitions[index];
        let mut fields = Vec::new();

        if position == 0 && config.firmware == Firmware::Uboot && disk == layout.boot_disk() {
            fields.push(format!("start={UBOOT_START}"));
        }

        if let Size::Mib(mib) = part.size {
            fields.push(format!("size={mib}MiB"));
        }

        fields.push(format!("type={}", type_guid(&part.usage, config.firmware)));

        if part.crypt.is_some_and(|crypt| crypt.detached) {
            fields.push(format!(
                "uuid={}",
                ids.partuuid(&layout.partition_device(index))
            ));
        }

        script.push_str(&fields.join(", "));
        script.push('\n');
    }

    Some(script)
}

/// Слои снизу вверх: LUKS на разделах, массивы (и LUKS на них), группы LVM и тома (и LUKS на них).
pub fn stacking(config: &Config, ids: &Ids) -> Vec<Step> {
    let layout = &config.layout;
    let mut steps = Vec::new();

    for index in 0..layout.partitions.len() {
        steps.extend(encryption(config, ids, &layout.partition_device(index)));
    }

    for raid in &layout.raids {
        let path = raid_path(&raid.name);
        let mut args = vec![
            "--create".to_string(),
            path.clone(),
            "--run".into(),
            "--metadata=1.2".into(),
            format!("--level={}", raid.level.number()),
            format!("--raid-devices={}", layout.members(&raid.name).len()),
            format!("--uuid={}", ids.array(&raid.name)),
        ];

        args.extend(
            layout
                .members(&raid.name)
                .iter()
                .map(|member| layout.inner(member)),
        );
        steps.push(run("mdadm", args));
        steps.extend(encryption(config, ids, &path));
    }

    for group in layout.groups() {
        let physical: Vec<String> = layout
            .volumes_of(&group)
            .iter()
            .map(|path| layout.inner(path))
            .collect();
        let mut create = vec!["--yes".to_string()];
        let mut build = vec![group.clone()];

        create.extend(physical.iter().cloned());
        build.extend(physical);
        steps.push(run("pvcreate", create));
        steps.push(run("vgcreate", build));

        let mut volumes: Vec<&Volume> = layout
            .volumes
            .iter()
            .filter(|volume| volume.group == group)
            .collect();

        volumes.sort_by_key(|volume| (volume.pool.is_some(), volume.size == Size::Rest));

        for volume in volumes {
            steps.push(run("lvcreate", lvcreate_args(volume)));
            steps.extend(encryption(
                config,
                ids,
                &volume_path(&volume.group, &volume.name),
            ));
        }
    }

    steps
}

/// Обычный том и пул отличаются размером и типом, тонкий том создаётся внутри пула.
fn lvcreate_args(volume: &Volume) -> Vec<String> {
    let mut args = vec!["--yes".to_string(), "--name".into(), volume.name.clone()];

    if volume.is_pool() {
        args.extend(["--type".to_string(), "thin-pool".into()]);
    }

    match (volume.size, &volume.pool) {
        (Size::Mib(mib), Some(pool)) => {
            args.extend([
                "--thin".to_string(),
                "--virtualsize".into(),
                format!("{mib}m"),
            ]);
            args.push(format!("{}/{pool}", volume.group));
            return args;
        }
        (Size::Mib(mib), None) => args.extend(["--size".to_string(), format!("{mib}m")]),
        _ => args.extend(["--extents".to_string(), "100%FREE".to_string()]),
    }

    args.push(volume.group.clone());
    args
}

pub fn formatting(config: &Config, ids: &Ids) -> Vec<Step> {
    config
        .layout
        .filesystems()
        .iter()
        .map(|device| make_filesystem(device, &config.layout.inner(&device.path), ids))
        .collect()
}

fn make_filesystem(device: &Device, target: &str, ids: &Ids) -> Step {
    let uuid = ids.device(&device.path).to_string();

    match &device.usage {
        Use::Boot => run(
            "mkfs.fat",
            ["-F", "32", "-i", &format!("{:08X}", ids.fat), target],
        ),
        Use::Swap => run("mkswap", ["--uuid", &uuid, target]),
        Use::Mount {
            fs: Fs::Xfs,
            tuning,
            ..
        } => {
            let mut args = vec!["-f".to_string(), "-m".into(), format!("uuid={uuid}")];

            args.extend(tuning.mkfs.iter().cloned());
            args.push(target.into());
            run("mkfs.xfs", args)
        }
        Use::Mount {
            fs: Fs::Btrfs,
            tuning,
            ..
        } => {
            let mut args = vec!["-f".to_string(), "-U".into(), uuid];

            args.extend(tuning.mkfs.iter().cloned());
            args.push(target.into());
            run("mkfs.btrfs", args)
        }
        Use::Mount { tuning, .. } => {
            let mut args = vec!["-F".to_string(), "-U".into(), uuid];

            args.extend(tuning.mkfs.iter().cloned());
            args.push(target.into());
            run("mkfs.ext4", args)
        }
        _ => run("mkfs.ext4", ["-F", "-U", &uuid, target]),
    }
}

/// Создаёт LUKS2 на устройстве и открывает его; заголовок до копирования в систему живёт в `/run`.
fn encryption(config: &Config, ids: &Ids, path: &str) -> Vec<Step> {
    let layout = &config.layout;
    let (Some(crypt), Some(secret)) = (layout.crypt_of(path), config.secret(path)) else {
        return Vec::new();
    };
    let input = || Input::Secret(secret.clone());
    let mut steps = Vec::new();
    let mut format_args = vec![
        "luksFormat".to_string(),
        "--type".into(),
        "luks2".into(),
        "--batch-mode".into(),
        "--uuid".into(),
        ids.luks_of(path).to_string(),
        "--key-file".into(),
        "-".into(),
    ];
    let mut open_args = vec![
        "open".to_string(),
        "--type".into(),
        "luks".into(),
        "--key-file".into(),
        "-".into(),
    ];

    if crypt.detached {
        let temp = header_temp(&layout.crypt_name(path));

        steps.push(run("truncate", ["-s", HEADER_SIZE, &temp]));

        for args in [&mut format_args, &mut open_args] {
            args.extend(["--header".to_string(), temp.clone()]);
        }
    }

    format_args.push(path.into());
    open_args.extend([path.to_string(), layout.mapper(path)]);
    steps.push(Step::Run(
        Command::new("cryptsetup", format_args).input(input()),
    ));
    steps.push(Step::Run(
        Command::new("cryptsetup", open_args).input(input()),
    ));
    steps
}

fn depth(path: &str) -> usize {
    if path == "/" {
        0
    } else {
        path.split('/').count()
    }
}

/// Файловые системы с точками монтирования: родитель всегда раньше потомка.
fn mounted(config: &Config) -> Vec<Device> {
    let mut devices: Vec<Device> = config
        .layout
        .filesystems()
        .into_iter()
        .filter(|device| device.usage.mount_path().is_some())
        .collect();

    devices.sort_by_key(|device| depth(device.usage.mount_path().unwrap_or("/")));
    devices
}

pub fn mounting(config: &Config) -> Vec<Step> {
    let mut steps = vec![run("mkdir", ["-p", TARGET])];

    for device in mounted(config) {
        let path = device.usage.mount_path().unwrap_or("/");
        let source = config.layout.inner(&device.path);

        if path == "/" {
            steps.push(run("mount", [&source, TARGET]));
        } else {
            let target = format!("{TARGET}{path}");

            steps.push(run("mkdir", ["-p", &target]));
            steps.push(run("mount", [&source, &target]));
        }
    }

    steps
}

pub fn fstab(config: &Config, ids: &Ids) -> String {
    let mut text = String::new();

    for device in mounted(config) {
        let (spec, path, kind, options, pass) = match &device.usage {
            Use::Boot => (
                format!("UUID={}", ids.fat_label()),
                "/boot",
                "vfat",
                "umask=0077".to_string(),
                "2",
            ),
            Use::Mount { path, fs, tuning } => (
                format!("UUID={}", ids.device(&device.path)),
                path.as_str(),
                fs.name(),
                tuning.fstab_options(),
                match (fs, path.as_str()) {
                    (Fs::Xfs | Fs::Btrfs, _) => "0",
                    (_, "/") => "1",
                    _ => "2",
                },
            ),
            _ => continue,
        };

        text.push_str(&format!("{spec} {path} {kind} {options} 0 {pass}\n"));
    }

    for device in config.layout.filesystems() {
        if device.usage == Use::Swap {
            text.push_str(&format!(
                "UUID={} none swap defaults 0 0\n",
                ids.device(&device.path)
            ));
        }
    }

    text
}

/// Массивы для initramfs и системы: `mdadm.conf` с UUID, заданными при создании.
pub fn mdadm_conf(config: &Config, ids: &Ids) -> Option<Step> {
    if config.layout.raids.is_empty() {
        return None;
    }

    let lines: String = config
        .layout
        .raids
        .iter()
        .map(|raid| {
            format!(
                "ARRAY {} metadata=1.2 UUID={}\n",
                raid_path(&raid.name),
                ids.array(&raid.name)
            )
        })
        .collect();

    Some(file("/etc/mdadm.conf", lines, 0o644))
}

/// Параметры ядра, которые описывают корень.
pub fn root_arguments(config: &Config, ids: &Ids) -> Vec<String> {
    let layout = &config.layout;
    let root = layout.root_device();
    let mut args = vec![format!("root=UUID={}", ids.device(&root)), "rw".to_string()];

    for path in layout.early() {
        if layout.crypt_of(&path).is_some_and(|crypt| !crypt.detached) {
            args.push(format!("rd.luks.uuid={}", ids.luks_of(&path)));
        }
    }

    for volume in &layout.volumes {
        let path = volume_path(&volume.group, &volume.name);

        if path == root || volume.usage.mount_path() == Some("/usr") {
            args.push(format!("rd.lvm.lv={}/{}", volume.group, volume.name));
        }
    }

    args
}

pub fn dracut_modules(config: &Config) -> Vec<&'static str> {
    let mut modules = Vec::new();

    if !config.layout.early().is_empty() {
        modules.push("crypt");
    }

    if config.layout.needs_lvm() {
        modules.push("lvm");
    }

    if config.layout.needs_raid() {
        modules.push("mdraid");
    }

    if separate_usr(config) {
        modules.push("usrmount");
    }

    if config.layout.uses_fs(Fs::Btrfs) {
        modules.push("btrfs");
    }

    modules
}

fn separate_usr(config: &Config) -> bool {
    config
        .layout
        .devices()
        .iter()
        .any(|device| device.usage.mount_path() == Some("/usr"))
}

pub fn packages(config: &Config) -> Vec<&'static str> {
    let layout = &config.layout;
    let mut names = Vec::new();

    if layout.has_crypt() {
        names.push("cryptsetup");
    }

    if layout.needs_lvm() {
        names.push("lvm2");
    }

    if layout.needs_thin() {
        names.push("device-mapper-persistent-data");
    }

    if layout.needs_raid() {
        names.push("mdadm");
    }

    if layout.uses_fs(Fs::Xfs) {
        names.push("xfsprogs");
    }

    if layout.uses_fs(Fs::Btrfs) {
        names.push("btrfs-progs");
    }

    names
}

/// Утилита, которую установщик запускает у себя, и пакет, где она лежит.
struct Tool {
    program: &'static str,
    package: &'static str,
}

fn tools(layout: &Layout) -> Vec<Tool> {
    let mut tools = vec![Tool {
        program: "mkfs.fat",
        package: "dosfstools",
    }];

    if layout.uses_fs(Fs::Ext4) {
        tools.push(Tool {
            program: "mkfs.ext4",
            package: "e2fsprogs",
        });
    }

    if layout.uses_fs(Fs::Xfs) {
        tools.push(Tool {
            program: "mkfs.xfs",
            package: "xfsprogs",
        });
    }

    if layout.uses_fs(Fs::Btrfs) {
        tools.push(Tool {
            program: "mkfs.btrfs",
            package: "btrfs-progs",
        });
    }

    if layout.has_crypt() {
        tools.push(Tool {
            program: "cryptsetup",
            package: "cryptsetup",
        });
    }

    if layout.needs_lvm() {
        tools.push(Tool {
            program: "pvcreate",
            package: "lvm2",
        });
    }

    if layout.needs_thin() {
        tools.push(Tool {
            program: "thin_check",
            package: "device-mapper-persistent-data",
        });
    }

    if layout.needs_raid() {
        tools.push(Tool {
            program: "mdadm",
            package: "mdadm",
        });
    }

    tools
}

/// Пакеты, которых не хватает самому установщику для выбранной разметки.
pub fn missing(layout: &Layout, present: impl Fn(&str) -> bool) -> Vec<&'static str> {
    let mut packages = Vec::new();

    for tool in tools(layout) {
        if !present(tool.program) && !packages.contains(&tool.package) {
            packages.push(tool.package);
        }
    }

    packages
}

/// Кэш `dnf` на установщике лежит в памяти: корень живой системы или initramfs может быть тесным.
const HOST_CACHE: &str = "/run/moonshine/cache";

/// Недостающие утилиты доставляются на установщик до первого обращения к диску.
pub fn fetch(config: &Config, present: impl Fn(&str) -> bool) -> Vec<Step> {
    let packages = missing(&config.layout, present);

    if packages.is_empty() {
        return Vec::new();
    }

    let cache = format!("--setopt=cachedir={HOST_CACHE}");
    let mut args = vec!["install", "--assumeyes", cache.as_str()];

    args.extend(packages);

    vec![run("dnf", args)]
}

/// Crypttab: раскрытие устройств в живой системе; нужные initramfs помечены `x-initrd.attach`.
pub fn crypttab(config: &Config, ids: &Ids) -> Option<Step> {
    let layout = &config.layout;
    let early = layout.early();
    let mut text = String::new();

    for device in layout.crypts() {
        let Some(crypt) = device.crypt else { continue };
        let name = layout.crypt_name(&device.path);
        let key = match crypt.unlock {
            Unlock::Typed => "none".to_string(),
            Unlock::Keyfile => key_path(&name),
        };
        let mut options = String::from("luks");

        if crypt.detached {
            options.push_str(&format!(",header={}", header_path(&name)));
        }

        if early.contains(&device.path) {
            options.push_str(",x-initrd.attach");
        }

        text.push_str(&format!(
            "{} {} {key} {options}\n",
            layout.mapper(&device.path),
            crypt_source(config, ids, &device.path)
        ));
    }

    (!text.is_empty()).then(|| file("/etc/crypttab", text, 0o600))
}

/// Ключ-файлы и отдельные заголовки в `/etc/luks` установленной системы.
pub fn luks_files(config: &Config) -> Vec<Step> {
    let layout = &config.layout;
    let mut steps = Vec::new();

    for device in layout.crypts() {
        let Some(crypt) = device.crypt else { continue };
        let name = layout.crypt_name(&device.path);

        if crypt.unlock == Unlock::Keyfile
            && let Some(key) = config.secret(&device.path)
        {
            steps.push(file(&key_path(&name), key.expose().to_string(), 0o600));
        }

        if crypt.detached {
            let target = format!("{TARGET}{}", header_path(&name));

            steps.push(run("mkdir", ["-p", &format!("{TARGET}/etc/luks")]));
            steps.push(run("cp", [&header_temp(&name), &target]));
            steps.push(run("chmod", ["600", &target]));
        }
    }

    steps
}

/// Файлы, которые dracut кладёт в initramfs: `crypttab` и заголовки с ключами только у устройств, нужных для старта.
pub fn dracut_items(config: &Config) -> Vec<String> {
    let layout = &config.layout;
    let early = layout.early();

    if early.is_empty() {
        return Vec::new();
    }

    let mut items = vec!["/etc/crypttab".to_string()];

    for path in early {
        let Some(crypt) = layout.crypt_of(&path) else {
            continue;
        };
        let name = layout.crypt_name(&path);

        if crypt.detached {
            items.push(header_path(&name));
        }

        if crypt.unlock == Unlock::Keyfile {
            items.push(key_path(&name));
        }
    }

    items
}

/// Размонтирование и остановка слоёв в обратном порядке.
pub fn finish(config: &Config) -> Vec<Step> {
    let layout = &config.layout;
    let mut steps = vec![
        run("sync", Vec::<String>::new()),
        run("umount", ["--recursive", TARGET]),
        run("rmdir", [TARGET]),
    ];
    let volumes: Vec<String> = layout
        .volumes
        .iter()
        .map(|volume| volume_path(&volume.group, &volume.name))
        .collect();
    let arrays: Vec<String> = layout
        .raids
        .iter()
        .map(|raid| raid_path(&raid.name))
        .collect();
    let crypts: Vec<String> = layout
        .crypts()
        .into_iter()
        .map(|device| device.path)
        .collect();
    let closing = |steps: &mut Vec<Step>, wanted: &dyn Fn(&String) -> bool| {
        for path in crypts.iter().rev().filter(|path| wanted(path)) {
            steps.push(run("cryptsetup", ["close", &layout.mapper(path)]));
        }
    };

    closing(&mut steps, &|path| volumes.contains(path));

    for group in layout.groups() {
        steps.push(run("vgchange", ["--activate", "n", &group]));
    }

    closing(&mut steps, &|path| arrays.contains(path));

    for path in &arrays {
        steps.push(run("mdadm", ["--stop", path]));
    }

    closing(&mut steps, &|path| {
        !volumes.contains(path) && !arrays.contains(path)
    });

    for path in &crypts {
        if layout.crypt_of(path).is_some_and(|crypt| crypt.detached) {
            steps.push(run("rm", ["-f", &header_temp(&layout.crypt_name(path))]));
        }
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Firmware;
    use crate::config::Secret;
    use crate::layout::Crypt;
    use crate::layout::Layout;
    use crate::plan::build;
    use crate::testing::config;
    use crate::testing::ids_for;

    fn layout(rows: &[&str]) -> Layout {
        let rows: Vec<String> = rows.iter().map(ToString::to_string).collect();

        Layout::from_rows(&rows).unwrap()
    }

    const TYPED: Crypt = Crypt {
        unlock: Unlock::Typed,
        detached: false,
    };

    fn encrypted(rows: &[&str]) -> Layout {
        layout(rows).with_root_crypt(Some(TYPED))
    }

    const DETACHED: Crypt = Crypt {
        unlock: Unlock::Typed,
        detached: true,
    };

    fn lvm() -> Config {
        Config {
            layout: encrypted(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=lvm:vg",
                "partition disk=/dev/sdb,size=rest,point=lvm:vg",
                "volume vg=vg,name=home,size=rest,point=/home,fs=xfs",
                "volume vg=vg,name=root,size=40GiB,point=/",
                "volume vg=vg,name=swap,size=4GiB,point=swap",
            ]),
            ..config()
        }
    }

    fn raid() -> Config {
        Config {
            layout: encrypted(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=raid:md0",
                "partition disk=/dev/sdb,size=rest,point=raid:md0",
                "raid name=md0,level=1,point=lvm:vg",
                "volume vg=vg,name=root,size=rest,point=/",
            ]),
            ..config()
        }
    }

    fn plan(config: &Config) -> (Vec<String>, Vec<Step>) {
        let steps = build(config, &ids_for(&config.layout));

        (steps.iter().map(Step::to_string).collect(), steps)
    }

    fn at(lines: &[String], prefix: &str) -> usize {
        lines
            .iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("нет шага {prefix}: {lines:#?}"))
    }

    fn written<'a>(steps: &'a [Step], path: &str) -> &'a str {
        steps
            .iter()
            .find_map(|step| match step {
                Step::Write(file) if file.path == path => Some(file.contents.as_str()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("файл {path} не записан"))
    }

    #[test]
    fn every_disk_is_wiped_and_gets_its_own_table() {
        let (lines, _) = plan(&lvm());

        assert!(lines[at(&lines, "sfdisk /dev/sda")].contains(TYPE_LVM));
        assert!(lines[at(&lines, "sfdisk /dev/sda")].contains(TYPE_ESP));
        assert!(lines[at(&lines, "sfdisk /dev/sdb")].contains(TYPE_LVM));
        assert!(at(&lines, "wipefs --all /dev/sdb") < at(&lines, "sfdisk /dev/sdb"));
        assert!(at(&lines, "sfdisk /dev/sdb") < at(&lines, "udevadm settle"));
    }

    #[test]
    fn groups_are_built_from_partitions_and_volumes_are_created_with_rest_last() {
        let (lines, _) = plan(&lvm());

        assert_eq!(
            lines[at(&lines, "pvcreate")],
            "pvcreate --yes /dev/sda2 /dev/sdb1"
        );
        assert_eq!(
            lines[at(&lines, "vgcreate")],
            "vgcreate vg /dev/sda2 /dev/sdb1"
        );
        assert!(lines.contains(&"lvcreate --yes --name root --size 40960m vg".to_string()));
        assert!(lines.contains(&"lvcreate --yes --name swap --size 4096m vg".to_string()));
        assert!(
            at(&lines, "lvcreate --yes --name swap") < at(&lines, "lvcreate --yes --name home")
        );
        assert!(lines.contains(&"lvcreate --yes --name home --extents 100%FREE vg".to_string()));
        assert!(at(&lines, "sfdisk /dev/sdb") < at(&lines, "pvcreate"));
        assert!(at(&lines, "lvcreate") < at(&lines, "cryptsetup luksFormat"));
    }

    #[test]
    fn luks_goes_on_the_root_volume_and_other_volumes_stay_plain() {
        let (lines, _) = plan(&lvm());

        assert!(lines[at(&lines, "cryptsetup luksFormat")].contains(" /dev/vg/root <"));
        assert!(lines[at(&lines, "cryptsetup open")].contains(" /dev/vg/root cryptroot <"));
        assert!(
            lines.iter().any(
                |line| line.starts_with("mkfs.ext4") && line.ends_with("/dev/mapper/cryptroot")
            )
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("mkfs.xfs -f -m uuid=")
                    && line.ends_with("/dev/vg/home"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("mkswap --uuid ") && line.ends_with("/dev/vg/swap"))
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("cryptsetup luksFormat"))
                .count(),
            1
        );
    }

    #[test]
    fn filesystems_are_mounted_parents_first() {
        let (lines, _) = plan(&lvm());
        let mounts: Vec<&String> = lines
            .iter()
            .filter(|line| line.starts_with("mount /dev/") && !line.contains("--"))
            .collect();

        assert_eq!(
            mounts,
            [
                "mount /dev/mapper/cryptroot /mnt/target",
                "mount /dev/sda1 /mnt/target/boot",
                "mount /dev/vg/home /mnt/target/home",
            ]
        );
        assert!(at(&lines, "mkdir -p /mnt/target/home") < at(&lines, "mount /dev/vg/home"));
    }

    #[test]
    fn fstab_lists_every_filesystem_and_the_swap() {
        let config = lvm();
        let ids = ids_for(&config.layout);
        let fstab = fstab(&config, &ids);
        let lines: Vec<&str> = fstab.lines().collect();

        assert_eq!(lines.len(), 4);
        assert!(lines[0].ends_with(" / ext4 defaults 0 1"));
        assert!(lines[1].ends_with(" /boot vfat umask=0077 0 2"));
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with(" /home xfs defaults 0 0"))
        );
        assert_eq!(lines[1], "UUID=ABCD-1234 /boot vfat umask=0077 0 2");
        assert!(lines.last().unwrap().ends_with(" none swap defaults 0 0"));
        assert!(lines[0].contains(&ids.device("/dev/vg/root").to_string()));
    }

    #[test]
    fn the_kernel_finds_the_root_volume_and_dracut_gets_the_modules() {
        let (_, steps) = plan(&lvm());
        let grub = written(&steps, "/etc/default/grub");
        let dracut = written(&steps, "/etc/dracut.conf.d/10-moonshine.conf");

        assert!(grub.contains("rd.lvm.lv=vg/root"));
        assert!(grub.contains("rd.luks.uuid="));
        assert!(dracut.contains("crypt lvm"));
        assert!(!dracut.contains("mdraid"));
        assert_eq!(
            written(&steps, "/etc/crypttab"),
            "cryptroot UUID=00000000-0000-0000-0000-000000000001 none luks,x-initrd.attach\n"
        );
    }

    #[test]
    fn needed_packages_follow_the_layers() {
        assert_eq!(packages(&lvm()), ["cryptsetup", "lvm2", "xfsprogs"]);
        assert_eq!(packages(&raid()), ["cryptsetup", "lvm2", "mdadm"]);
        assert!(
            packages(&Config {
                layout: Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi),
                ..config()
            })
            .is_empty()
        );
    }

    #[test]
    fn arrays_are_created_before_the_groups_with_a_known_uuid() {
        let config = raid();
        let ids = ids_for(&config.layout);
        let (lines, steps) = plan(&config);
        let create = &lines[at(&lines, "mdadm --create")];

        assert_eq!(
            create,
            &format!(
                "mdadm --create /dev/md/md0 --run --metadata=1.2 --level=1 --raid-devices=2 --uuid={} /dev/sda2 /dev/sdb1",
                ids.array("md0")
            )
        );
        assert!(at(&lines, "mdadm --create") < at(&lines, "pvcreate"));
        assert_eq!(lines[at(&lines, "pvcreate")], "pvcreate --yes /dev/md/md0");
        assert_eq!(
            written(&steps, "/etc/mdadm.conf"),
            format!("ARRAY /dev/md/md0 metadata=1.2 UUID={}\n", ids.array("md0"))
        );
        assert!(
            written(&steps, "/etc/dracut.conf.d/10-moonshine.conf").contains("crypt lvm mdraid")
        );
    }

    #[test]
    fn layers_are_stopped_in_the_reverse_order() {
        let (lines, _) = plan(&raid());

        assert!(at(&lines, "umount") < at(&lines, "cryptsetup close"));
        assert!(at(&lines, "cryptsetup close") < at(&lines, "vgchange --activate n vg"));
        assert!(at(&lines, "vgchange") < at(&lines, "mdadm --stop /dev/md/md0"));
    }

    #[test]
    fn without_lvm_and_raid_nothing_extra_is_planned() {
        let (lines, steps) = plan(&config());

        for forbidden in [
            "mdadm", "pvcreate", "vgcreate", "lvcreate", "vgchange", "mkswap", "mkfs.xfs",
        ] {
            assert!(
                !lines.iter().any(|line| line.starts_with(forbidden)),
                "{forbidden}"
            );
        }

        assert!(
            steps
                .iter()
                .all(|step| !matches!(step, Step::Write(file) if file.path == "/etc/mdadm.conf"))
        );
    }

    #[test]
    fn a_detached_header_on_a_volume_is_found_by_its_path() {
        let config = Config {
            layout: lvm().layout.with_root_crypt(Some(DETACHED)),
            ..lvm()
        };
        let (lines, steps) = plan(&config);

        assert_eq!(
            written(&steps, "/etc/crypttab"),
            "cryptroot /dev/vg/root none luks,header=/etc/luks/root.hdr,x-initrd.attach\n"
        );
        assert!(!lines[at(&lines, "sfdisk /dev/sda")].contains("uuid="));
        assert!(!written(&steps, "/etc/default/grub").contains("rd.luks.uuid"));
    }

    #[test]
    fn a_raw_root_disk_and_swap_on_a_partition_work_without_lvm() {
        let config = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=8GiB,point=swap",
                "partition disk=/dev/sdb,size=whole,point=/",
            ])
            .with_root_crypt(Some(DETACHED)),
            ..config()
        };
        let (lines, steps) = plan(&config);

        assert!(lines[at(&lines, "sfdisk /dev/sda")].contains(TYPE_SWAP));
        assert!(!lines.iter().any(|line| line.starts_with("sfdisk /dev/sdb")));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("mkswap --uuid ") && line.ends_with("/dev/sda2"))
        );
        assert!(lines[at(&lines, "cryptsetup luksFormat")].contains(" /dev/sdb <"));
        assert!(
            written(&steps, "/etc/crypttab").starts_with("cryptroot /dev/sdb none luks,header=")
        );
    }

    #[test]
    fn uuids_stay_unique_across_devices() {
        let config = lvm();
        let ids = ids_for(&config.layout);
        let mut seen = Vec::new();

        for device in config.layout.devices() {
            let id = ids.device(&device.path);

            assert!(!seen.contains(&id), "{}", device.path);
            seen.push(id);
        }
    }

    #[test]
    fn the_installer_needs_the_tools_of_the_chosen_layers() {
        let config = lvm();
        let all = missing(&config.layout, |_| false);

        assert_eq!(
            all,
            ["dosfstools", "e2fsprogs", "xfsprogs", "cryptsetup", "lvm2"]
        );
        assert_eq!(
            missing(&config.layout, |program| program != "mkfs.xfs"),
            ["xfsprogs"]
        );
        assert!(missing(&config.layout, |_| true).is_empty());
        assert_eq!(
            missing(&raid().layout, |_| false),
            ["dosfstools", "e2fsprogs", "cryptsetup", "lvm2", "mdadm"]
        );
    }

    #[test]
    fn btrfs_gets_its_own_mkfs_fstab_line_package_and_dracut_module() {
        let config = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/,fs=btrfs,mkfs=--nodiscard,options=compress=zstd",
            ]),
            ..config()
        };
        let (lines, steps) = plan(&config);
        let fstab = steps
            .iter()
            .find_map(|step| match step {
                Step::Write(file) if file.path == "/etc/fstab" => Some(file.contents.clone()),
                _ => None,
            })
            .unwrap();

        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("mkfs.btrfs -f -U ")
                    && line.contains("--nodiscard /dev/sda2")),
            "{lines:#?}"
        );
        assert!(fstab.contains(" / btrfs compress=zstd 0 0\n"), "{fstab}");
        assert!(packages(&config).contains(&"btrfs-progs"));
        assert!(dracut_modules(&config).contains(&"btrfs"));
        assert!(missing(&config.layout, |_| false).contains(&"btrfs-progs"));
        assert!(
            Layout::from_rows(&[
                "partition disk=/dev/sda,size=rest,point=/,mkfs=--uuid x".to_string()
            ])
            .is_err()
        );
    }

    #[test]
    fn a_thin_pool_is_created_first_and_thin_volumes_live_inside_it() {
        let config = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=lvm:vg",
                "volume vg=vg,name=home,size=80GiB,point=/home,pool=pool",
                "volume vg=vg,name=pool,size=rest,point=pool",
                "volume vg=vg,name=root,size=20GiB,point=/",
            ]),
            ..config()
        };
        let (lines, _) = plan(&config);
        let lvcreate: Vec<&str> = lines
            .iter()
            .filter(|line| line.starts_with("lvcreate"))
            .map(String::as_str)
            .collect();

        assert_eq!(
            lvcreate,
            [
                "lvcreate --yes --name root --size 20480m vg",
                "lvcreate --yes --name pool --type thin-pool --extents 100%FREE vg",
                "lvcreate --yes --name home --thin --virtualsize 81920m vg/pool",
            ]
        );
        assert!(!lines.iter().any(|line| line.contains("/dev/vg/pool")));
        assert!(packages(&config).contains(&"device-mapper-persistent-data"));
        assert!(missing(&config.layout, |_| false).contains(&"device-mapper-persistent-data"));
    }

    #[test]
    fn missing_tools_are_fetched_with_one_dnf_call_before_everything_else() {
        let config = lvm();
        let steps = fetch(&config, |program| program == "mkfs.fat");

        assert_eq!(
            steps.iter().map(Step::to_string).collect::<Vec<_>>(),
            [
                "dnf install --assumeyes --setopt=cachedir=/run/moonshine/cache e2fsprogs xfsprogs cryptsetup lvm2"
            ]
        );
        assert!(fetch(&config, |_| true).is_empty());

        let full = crate::plan::installation(&config, &ids_for(&config.layout), |_| false);

        assert!(full[0].to_string().starts_with("dnf install --assumeyes "));
        assert!(full[1].to_string().starts_with("освободить диски"));
        assert!(full[2].to_string().starts_with("wipefs"));
    }

    #[test]
    fn a_separate_usr_on_lvm_is_activated_and_mounted_by_the_initramfs() {
        let config = Config {
            layout: encrypted(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=lvm:vg",
                "volume vg=vg,name=root,size=20GiB,point=/",
                "volume vg=vg,name=usr,size=20GiB,point=/usr",
                "volume vg=vg,name=home,size=rest,point=/home",
            ]),
            ..config()
        };
        let (lines, steps) = plan(&config);
        let grub = written(&steps, "/etc/default/grub");

        assert!(grub.contains("rd.lvm.lv=vg/root"));
        assert!(grub.contains("rd.lvm.lv=vg/usr"));
        assert!(!grub.contains("rd.lvm.lv=vg/home"));
        assert!(
            written(&steps, "/etc/dracut.conf.d/10-moonshine.conf").contains("crypt lvm usrmount")
        );
        assert!(at(&lines, "mount /dev/vg/usr") < at(&lines, "dnf --installroot"));
        assert!(fstab(&config, &ids_for(&config.layout)).contains(" /usr ext4 "));
    }

    #[test]
    fn without_a_separate_usr_the_usr_module_is_not_added() {
        let (_, steps) = plan(&lvm());

        assert!(!written(&steps, "/etc/dracut.conf.d/10-moonshine.conf").contains("usrmount"));
    }

    const KEYED: Crypt = Crypt {
        unlock: Unlock::Keyfile,
        detached: true,
    };

    fn data_disk() -> Config {
        Config {
            layout: encrypted(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/",
                "partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key,header=detached",
            ]),
            keys: vec![("/dev/sdb".into(), Secret::new("data-key"))],
            ..config()
        }
    }

    #[test]
    fn a_data_disk_is_encrypted_with_its_own_key_and_a_detached_header() {
        let (lines, steps) = plan(&data_disk());
        let format = &lines[at(&lines, "cryptsetup luksFormat")..];
        let header = "/run/moonshine-sdb.hdr";

        assert!(lines.contains(&format!("truncate -s 16MiB {header}")));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("cryptsetup luksFormat")
                    && line.contains(&format!("--header {header} /dev/sdb <")))
        );
        assert!(
            format
                .iter()
                .any(|line| line.contains(&format!("--header {header} /dev/sdb crypt-sdb <")))
        );
        assert!(
            lines.iter().any(
                |line| line.starts_with("mkfs.ext4") && line.ends_with("/dev/mapper/crypt-sdb")
            )
        );
        assert!(lines.contains(&"mount /dev/mapper/crypt-sdb /mnt/target/mnt/data".to_string()));
        assert!(lines.contains(&format!("cp {header} /mnt/target/etc/luks/sdb.hdr")));
        assert_eq!(written(&steps, "/etc/luks/sdb.key"), "data-key");
    }

    #[test]
    fn a_data_disk_is_opened_by_the_live_system_and_stays_out_of_the_initramfs() {
        let (_, steps) = plan(&data_disk());
        let crypttab = written(&steps, "/etc/crypttab");
        let dracut = written(&steps, "/etc/dracut.conf.d/10-moonshine.conf");

        assert_eq!(
            crypttab,
            "cryptroot UUID=00000000-0000-0000-0000-000000000001 none luks,x-initrd.attach\n\
             crypt-sdb /dev/sdb /etc/luks/sdb.key luks,header=/etc/luks/sdb.hdr\n"
        );
        assert!(dracut.contains("install_items+=\" /etc/crypttab \""));
        assert!(!dracut.contains("sdb"));
        assert_eq!(
            written(&steps, "/etc/default/grub")
                .matches("rd.luks.uuid=")
                .count(),
            1
        );
    }

    #[test]
    fn a_data_disk_is_closed_after_unmounting_and_its_header_leaves_the_installer() {
        let (lines, _) = plan(&data_disk());

        assert!(at(&lines, "umount") < at(&lines, "cryptsetup close crypt-sdb"));
        assert!(at(&lines, "cryptsetup close crypt-sdb") < at(&lines, "rm -f /run/moonshine-sdb"));
        assert_eq!(lines.last().unwrap(), "rm -f /run/moonshine-sdb.hdr");
    }

    #[test]
    fn a_detached_header_on_a_partition_is_found_by_partuuid() {
        let config = Config {
            layout: encrypted(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/",
                "partition disk=/dev/sdb,size=100GiB,point=/srv,crypt=key,header=detached",
                "partition disk=/dev/sdb,size=rest,point=/mnt/data,crypt=pass",
            ]),
            keys: vec![("/dev/sdb1".into(), Secret::new("k"))],
            ..config()
        };
        let ids = ids_for(&config.layout);
        let (lines, steps) = plan(&config);
        let crypttab = written(&steps, "/etc/crypttab");

        assert!(
            lines[at(&lines, "sfdisk /dev/sdb")]
                .contains(&format!("uuid={}", ids.partuuid("/dev/sdb1")))
        );
        assert!(
            !lines[at(&lines, "sfdisk /dev/sdb")].contains(&ids.partuuid("/dev/sdb2").to_string())
        );
        assert!(crypttab.contains(&format!(
            "crypt-sdb1 PARTUUID={} /etc/luks/sdb1.key luks,header=/etc/luks/sdb1.hdr\n",
            ids.partuuid("/dev/sdb1")
        )));
        assert!(crypttab.contains(&format!(
            "crypt-sdb2 UUID={} none luks\n",
            ids.luks_of("/dev/sdb2")
        )));
    }

    #[test]
    fn layers_are_stacked_in_order_luks_raid_luks_lvm_luks() {
        let config = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=raid:md0,crypt=pass",
                "partition disk=/dev/sdb,size=rest,point=raid:md0,crypt=pass",
                "raid name=md0,level=1,point=lvm:vg,crypt=key,header=detached",
                "volume vg=vg,name=root,size=rest,point=/,crypt=pass",
            ]),
            keys: vec![("/dev/md/md0".into(), Secret::new("k"))],
            ..config()
        };

        assert!(config.layout.validate(Firmware::Uefi).is_ok());

        let (lines, _) = plan(&config);
        let format_at = |device: &str| {
            lines
                .iter()
                .position(|line| line.starts_with("cryptsetup luksFormat") && line.contains(device))
                .unwrap_or_else(|| panic!("нет luksFormat для {device}: {lines:#?}"))
        };

        assert!(format_at(" /dev/sda2 <") < at(&lines, "mdadm --create"));
        assert!(format_at(" /dev/sdb1 <") < at(&lines, "mdadm --create"));
        assert!(
            lines[at(&lines, "mdadm --create")]
                .ends_with(" /dev/mapper/crypt-sda2 /dev/mapper/crypt-sdb1")
        );
        assert!(at(&lines, "mdadm --create") < format_at(" /dev/md/md0 <"));
        assert_eq!(
            lines[at(&lines, "pvcreate")],
            "pvcreate --yes /dev/mapper/crypt-md-md0"
        );
        assert_eq!(
            lines[at(&lines, "vgcreate")],
            "vgcreate vg /dev/mapper/crypt-md-md0"
        );
        assert!(at(&lines, "lvcreate") < format_at(" /dev/vg/root <"));
    }

    #[test]
    fn layers_are_closed_from_the_top_to_the_bottom() {
        let config = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=raid:md0,crypt=pass",
                "partition disk=/dev/sdb,size=rest,point=raid:md0,crypt=pass",
                "raid name=md0,level=1,point=lvm:vg,crypt=pass",
                "volume vg=vg,name=root,size=rest,point=/,crypt=pass",
            ]),
            ..config()
        };
        let (lines, _) = plan(&config);
        let order = [
            "umount",
            "cryptsetup close cryptroot",
            "vgchange --activate n vg",
            "cryptsetup close crypt-md-md0",
            "mdadm --stop /dev/md/md0",
            "cryptsetup close crypt-sdb1",
            "cryptsetup close crypt-sda2",
        ];

        for pair in order.windows(2) {
            assert!(at(&lines, pair[0]) < at(&lines, pair[1]), "{pair:?}");
        }
    }

    #[test]
    fn the_initramfs_gets_the_key_and_the_header_of_the_root_only() {
        let config = Config {
            layout: lvm().layout.with_root_crypt(Some(KEYED)),
            keys: vec![("/dev/vg/root".into(), Secret::new("k"))],
            ..lvm()
        };
        let (_, steps) = plan(&config);
        let dracut = written(&steps, "/etc/dracut.conf.d/10-moonshine.conf");

        assert!(
            dracut.contains(
                "install_items+=\" /etc/crypttab /etc/luks/root.hdr /etc/luks/root.key \""
            )
        );
        assert_eq!(written(&steps, "/etc/luks/root.key"), "k");
    }

    #[test]
    fn a_plain_layout_has_no_crypttab_keys_or_crypt_module() {
        let config = Config {
            layout: Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi),
            ..config()
        };
        let (lines, steps) = plan(&config);

        assert!(steps.iter().all(|step| !matches!(
            step,
            Step::Write(file) if file.path == "/etc/crypttab" || file.path.starts_with("/etc/luks")
        )));
        assert!(!lines.iter().any(|line| line.contains("cryptsetup")));
        assert!(dracut_modules(&config).is_empty());
    }

    #[test]
    fn only_devices_under_the_root_and_usr_are_opened_early() {
        let data = data_disk();

        assert_eq!(data.layout.early(), ["/dev/sda2"]);
        assert_eq!(dracut_modules(&data), ["crypt"]);

        let plain = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/",
                "partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=pass",
            ]),
            ..config()
        };

        assert!(plain.layout.early().is_empty());
        assert!(dracut_modules(&plain).is_empty());
    }

    #[test]
    fn mkfs_arguments_and_mount_options_reach_the_commands_and_fstab() {
        let config = Config {
            layout: layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=20GiB,point=/,mkfs=-T largefile4 -m 1,options=noatime;discard",
                "partition disk=/dev/sda,size=rest,point=/data,fs=xfs,mkfs=-m crc=1;finobt=1,options=noatime",
            ]),
            ..config()
        };
        let ids = ids_for(&config.layout);
        let (lines, _) = plan(&config);

        assert!(lines.iter().any(|line| line.starts_with("mkfs.ext4 -F -U ")
            && line.ends_with(" -T largefile4 -m 1 /dev/sda2")));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("mkfs.xfs -f -m uuid=")
                    && line.ends_with(" -m crc=1,finobt=1 /dev/sda3"))
        );

        let fstab = fstab(&config, &ids);

        assert!(fstab.contains(" / ext4 noatime,discard 0 1\n"));
        assert!(fstab.contains(" /data xfs noatime 0 0\n"));
    }
}
