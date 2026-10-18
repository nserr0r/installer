<div align="center">

# 🌙 moonshine-install

**Установщик RPM-системы из репозиториев: мастер, CLI и конфиг (файл, URL, git).**

🌐 **Русский** · [English](README.en.md)

</div>

---

**🚀 Суть:** система собирается `dnf --installroot` из ваших репозиториев прямо на новый диск. Машина описывается одним небольшим файлом.

> ⚠️ Установщик **стирает диски**. Сначала: `moonshine-install check ЦЕЛЬ --plan`.

---

## ⚙️ Использование

1. Мастер:

```sh
sudo moonshine-install
```

2. Проверка конфига без изменений:

```sh
moonshine-install check https://evillin.online/moonshine.toml --plan
```

3. Установка по конфигу (файл, `https://`, `git+URL#ветка:путь.toml`):

```sh
sudo moonshine-install --config https://evillin.online/moonshine.toml
```

4. Установка флагами:

```sh
sudo moonshine-install --disk /dev/sda --repo baseos --hostname moonshine \
  --user alice:'$6$salt$...' --timezone Europe/Moscow
```

Минимальный `moonshine.toml`:

```toml
disk     = "/dev/sda"
hostname = "moonshine"
repo     = ["baseos"]

[[users]]
name = "alice"
hash = "$6$salt$..."             # openssl passwd -6
```

Перед стиранием установщик просит ввести `YES`; `--yes` отключает вопрос.

---

## ✨ Возможности

- 🖥️ **Мастер** (TUI), **CLI**, **TOML-конфиг**
- 📦 **Репозитории**: хоста, локальные, из конфига; приоритеты как в yum
- 💾 **Разметка**: разделы, **mdadm RAID**, **LVM2** (в том числе тонкие тома), ext4 / xfs / btrfs, swap
- 🔐 **LUKS2** на любом устройстве: пароль или ключ-файл, отдельный заголовок, разблокировка по SSH
- 🔌 **Загрузка на отдельном диске** (флешке)
- 🌐 **Платформы**: BIOS+UEFI (гибрид), UEFI, BIOS, U-Boot
- 🧩 **Параметры ядра** и **службы systemd**
- 🛠️ **Сеть**: отдельно для initramfs и системы
- 💿 **Модуль dracut**: установка с носителя или по сети
- 🌍 **Языки**: English, Русский

| Архитектура | Статус |
|---|---|
| x86_64 | поддерживается |
| aarch64 | в тестировании |

---

## 🔎 Железо

Определяется при запуске, переопределяется `--arch`, `--cpu`, `--firmware`.

| Что | Источник | Результат |
|---|---|---|
| Архитектура | бинарник | `x86_64` / `aarch64` |
| Процессор | `/proc/cpuinfo` | Intel → `microcode_ctl` |
| Прошивка | x86_64 → **hybrid**; aarch64 → UEFI при `/sys/firmware/efi`, иначе U-Boot | раскладка, загрузчик |

| Платформа | `grub2-install --target` |
|---|---|
| hybrid | `i386-pc`, `i386-efi`, `x86_64-efi` |
| uefi | `x86_64-efi` / `arm64-efi` |
| bios | `i386-pc` |
| uboot | `extlinux/extlinux.conf` |

Команды запускаются в `chroot` с `--no-nvram --force --removable`, затем `grub2-mkconfig`.

---

## 📦 Репозитории и пакеты

| Источник | Флаг / ключ | Остаётся в системе |
|---|---|---|
| Репозиторий хоста | `--repo ID` | через `--repo-file` |
| Локальный каталог | `--repo-dir PATH` | нет |
| Репозиторий из конфига | `--repository …` / `[[repository]]` | да |

```toml
[[repository]]
id       = "baseos"
baseurl  = "https://mirror.evillin.online/9/BaseOS/$basearch/os/"
priority = 10                   # 1…99, меньше = важнее
gpgkey   = "file:///etc/pki/rpm-gpg/RPM-GPG-KEY-example"

[[repository]]
id       = "company"
baseurl  = "https://rpm.evillin.online/company/$basearch/"
priority = 5
gpgcheck = false
```

| Ключ | Значение |
|---|---|
| `id` | обязателен, уникален |
| `baseurl` / `metalink` / `mirrorlist` | ровно один |
| `name`, `priority`, `exclude`, `includepkgs` | необязательны |
| `gpgcheck` | по умолчанию включён |
| `gpgkey` | обязателен при `gpgcheck` |

- Приоритет репозитория хоста: `--repo-priority ID=N`, только на время установки.
- Параметры dnf: `--option ключ=значение` (`releasever=44` → `--releasever=44`).
- Пакеты: `--package ИМЯ` (имя, `@группа`, шаблон); `--bare` отключает встроенный набор.
- Список пакетов с зависимостями показывается до установки.

### Другой RPM-дистрибутив

Нужны `dnf`, GRUB2 (`grub2-install`, `grub2-mkconfig`) и `dracut`. С `--bare` пакеты задаются полностью:

```toml
bare     = true
firmware = "hybrid"
disk     = "/dev/sda"
hostname = "rocky"
option   = ["releasever=9"]
package  = ["@core", "kernel", "dracut", "grub2-tools", "grub2-pc-modules",
            "grub2-efi-ia32-modules", "grub2-efi-x64-modules", "cryptsetup", "sudo"]

[[repository]]
id       = "baseos"
baseurl  = "https://mirror.evillin.online/9/BaseOS/$basearch/os/"
priority = 10
gpgkey   = "file:///etc/pki/rpm-gpg/RPM-GPG-KEY-example"

[[users]]
name = "alice"
hash = "$6$salt$..."
```

---

## 👤 Пользователи

```sh
moonshine-install ... --user alice:'$6$salt$hash' --user bob
```

- Без `root` в списке он блокируется.
- Остальные получают домашний каталог и группу **wheel**; ставится `sudo`.
- Без хеша пароль спрашивается на консоли.

---

## 🧩 Ядро и службы

| Что | Флаг | TOML | Мастер |
|---|---|---|---|
| Параметры ядра | `--kernel-arg` | `kernel_arg = ["quiet"]` | «Параметры ядра» |
| Службы | `--service` | `service = ["sshd"]` | «Службы для включения» |

- Параметры дописываются в `GRUB_CMDLINE_LINUX` (`/etc/default/grub`) и пишутся в `/etc/kernel/cmdline`.
- Допустимы печатные ASCII без пробелов, кавычек, `$`, `` ` `` и `\`.
- Службы включаются `systemctl enable` в `chroot`; пакет службы добавьте в `--package`.

---

## 🛠️ Сеть

Настраивается отдельно для initramfs (`--initrd-*`, `[initrd]`) и системы (`--system-*`, `[system]`).

| Режим | Значение |
|---|---|
| `none` | без сети |
| `dhcp` | DHCP, `interface` необязателен |
| `manual` | обязательны `interface`, `address`, `gateway`; `dns` повторяется |

- Службу системы задаёт `--system-backend networkd|networkmanager`.
- Вторая служба маскируется (`systemctl mask`), выбранная включается: при `networkmanager` — маска `systemd-networkd`, при `networkd` — маска `NetworkManager`. Отсутствие юнита не ошибка.
- `--ssh-key` ставит dracut-sshd и требует сети в initramfs.

---

## 💾 Диски и разметка

Диск задаётся путём: `/dev/sda`, `/dev/nvme0n1` или `/dev/disk/by-id/…` (предпочтительно).

| Схема | Раскладка |
|---|---|
| `--disk` | biosboot (BIOS, hybrid) + `/boot` FAT32 + корень |
| `--disk` + `--boot-disk` | загрузка на втором диске, система целиком без таблицы разделов |
| U-Boot | `/boot` первым разделом с отступом 16 MiB |

- `/boot` — FAT32, он же ESP. По умолчанию 512 MiB (`--boot-size`, минимум 256).
- В мастере также «LVM» и «RAID + LVM» из нескольких дисков.

### Своя разметка

```text
--partition disk=ДИСК,size=РАЗМЕР,point=ТОЧКА[,fs=ext4|xfs|btrfs][,mkfs=…][,options=…][,crypt=pass|key[,header=detached]]
--raid      name=ИМЯ,level=0|1|5|6|10,point=ТОЧКА[,fs=…][,mkfs=…][,options=…][,crypt=…]
--volume    vg=ГРУППА,name=ИМЯ,size=РАЗМЕР,point=ТОЧКА[,pool=ПУЛ][,fs=…][,mkfs=…][,options=…][,crypt=…]
```

| `size` | Значение |
|---|---|
| `512MiB`, `40GiB` | точный размер |
| `rest` | остаток |
| `whole` | диск целиком, без таблицы разделов |

| `point` | Значение |
|---|---|
| `biosboot` | 4 MiB для GRUB (BIOS, hybrid) |
| `reserve` | пустой раздел размером с `biosboot` и `/boot` |
| `boot` | `/boot`, FAT32, всегда обычный раздел |
| `swap` | подкачка |
| `pool` | пул тонких томов (`--volume`) |
| `lvm:ГРУППА` | физический том |
| `raid:ИМЯ` | участник массива |
| `/`, `/home`, … | файловая система |

```text
--partition disk=/dev/sda,size=4MiB,point=biosboot
--partition disk=/dev/sda,size=512MiB,point=boot
--partition disk=/dev/sda,size=rest,point=lvm:vg
--volume vg=vg,name=root,size=40GiB,point=/
--volume vg=vg,name=home,size=rest,point=/home,fs=xfs
```

RAID1 под LVM, загрузка с флешки:

```toml
firmware = "uefi"

[[partition]]
disk  = "/dev/disk/by-id/usb-SanDisk_Cruzer-0:0"
size  = "512MiB"
point = "boot"

[[partition]]
disk  = "/dev/sda"
size  = "whole"
point = "raid:md0"

[[partition]]
disk  = "/dev/sdb"
size  = "whole"
point = "raid:md0"

[[raid]]
name  = "md0"
level = 1
point = "lvm:vg"

[[volume]]
vg    = "vg"
name  = "root"
size  = "50GiB"
point = "/"

[[volume]]
vg    = "vg"
name  = "swap"
size  = "8GiB"
point = "swap"

[[volume]]
vg      = "vg"
name    = "home"
size    = "rest"
point   = "/home"
fs      = "xfs"
options = "noatime"
```

### Тонкие тома LVM

Пул — том с `point=pool`; тонкие тома указывают его в `pool=`. Размер тонкого тома виртуальный (`rest` не допускается).

```text
--partition disk=/dev/sda,size=4MiB,point=biosboot
--partition disk=/dev/sda,size=512MiB,point=boot
--partition disk=/dev/sda,size=rest,point=lvm:vg
--volume vg=vg,name=pool,size=100GiB,point=pool
--volume vg=vg,name=root,size=50GiB,point=/,pool=pool
--volume vg=vg,name=home,size=200GiB,point=/home,pool=pool
```

- Пул не шифруется, `swap` не бывает тонким.
- Превышение объёма пула даёт замечание в `check`.

### Параметры файловой системы

| Ключ | Значение |
|---|---|
| `mkfs=АРГУМЕНТЫ` | аргументы `mkfs.*` через пробел |
| `options=ПАРАМЕТРЫ` | поле `fstab` (по умолчанию `defaults`) |

Запятая в значениях пишется как `;`. Запрещены `-U`, `-F`, `-f`, `--uuid`, `--force`.

```text
--partition disk=/dev/sdb,size=whole,point=/data,mkfs=-T largefile4 -m 1,options=noatime
--partition disk=/dev/sdb,size=whole,point=/data,fs=xfs,mkfs=-m crc=1;finobt=1,options=noatime;discard
```

### Проверка разметки

До стирания проверяется:

- один `/` и один `boot`; `biosboot` только для BIOS и hybrid
- число участников RAID, разные диски
- тома в группе, не более одного `rest`
- безопасные и уникальные точки монтирования
- размеры по реальным дискам (только при установке)

### Редактор разметки

Страница «Диск» → «Схема дисков» → «Своя разметка».

| Клавиши | Действие |
|---|---|
| `↑` `↓` `Home` `End` `PgUp` `PgDn` | перемещение |
| `→` / `←` | развернуть / свернуть |
| `n` | добавить по месту |
| `r` / `v` | RAID / том LVM |
| `e` (`F2`) | править |
| `f` | файловая система |
| `o` | параметры: по умолчанию → SSD → большие файлы |
| `c` / `h` | шифрование / отдельный заголовок |
| `d` (`Delete`) | удалить |
| `Tab` | подставить `point=` |
| `u` | отменить (до 32 шагов) |
| `Esc` (`q`) | закрыть |

### Утилиты установщика

Недостающие утилиты ставятся хостовым `dnf` до любых изменений дисков.

| Утилита | Пакет | Когда |
|---|---|---|
| `mkfs.fat` | `dosfstools` | всегда |
| `mkfs.ext4` | `e2fsprogs` | ext4 |
| `mkfs.xfs` | `xfsprogs` | xfs |
| `mkfs.btrfs` | `btrfs-progs` | btrfs |
| `thin_check` | `device-mapper-persistent-data` | тонкие тома |
| `cryptsetup` | `cryptsetup` | шифрование |
| `pvcreate` | `lvm2` | LVM |
| `mdadm` | `mdadm` | RAID |

---

## 🔐 Шифрование

LUKS2 на разделе, массиве, физическом или логическом томе; слои комбинируются.

| Ключ | Значение |
|---|---|
| `crypt=pass` | парольная фраза, одна на все устройства |
| `crypt=key` | случайный ключ `/etc/luks/ИМЯ.key` (600) |
| `header=detached` | заголовок `/etc/luks/ИМЯ.hdr` |

- Простые схемы шифруют корень: `--no-encryption`, `--keyfile`, `--detached-header`.
- В initramfs попадают только устройства, нужные для `/` и `/usr`; остальные открывает система по `crypttab`.
- `/boot` и biosboot не шифруются.
- Для устройства в initramfs `crypt=key` требует `header=detached`.
- Заголовок внутри устройства занимает до 16 MiB.
- Отдельный заголовок на разделе в `crypttab` задаётся через `PARTUUID=`.

```text
--partition disk=/dev/sda,size=4MiB,point=biosboot
--partition disk=/dev/sda,size=512MiB,point=boot
--partition disk=/dev/sda,size=rest,point=/,crypt=key,header=detached
--partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key,header=detached
```

### Удалённая разблокировка

`--ssh-key` включает dracut-sshd; парольная фраза вводится по SSH.

> ⚠️ Ключ и заголовок корня в initramfs не защищают от потери флешки с `/boot`. Храните копию заголовков отдельно.

---

## 📄 Конфиг

TOML повторяет флаги CLI:

1. `a_b` → `--a-b`.
2. Массив повторяет флаг.
3. `true` ставит флаг, `false` ничего не делает.
4. `[initrd]`, `[system]`, `[uboot]` добавляют префикс.
5. `[[users]]`, `[[partition]]`, `[[raid]]`, `[[volume]]`, `[[repository]]` — блоки.

```toml
disk      = "/dev/disk/by-id/nvme-Samsung_SSD_980_PRO"
boot_disk = "/dev/disk/by-id/usb-SanDisk_Cruzer-0:0"
hostname  = "moonshine"
timezone  = "Europe/Moscow"

repo          = ["updates"]
repo_priority = ["updates=20"]
repo_dir      = ["/srv/rpms"]
package       = ["vim-enhanced", "htop"]
option        = ["tsflags=nodocs", "weak_deps=False", "releasever=44"]

keyfile = true
ssh_key = ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq3Zr0C6c3C3m1x0hJ6iHqB0t9pQ0QJ5h2S3mT1mYxQ alice@evillin.online"]

[[repository]]
id       = "company"
baseurl  = "https://rpm.evillin.online/company/$basearch/"
priority = 5
gpgcheck = false

[[users]]
name = "alice"
hash = "$6$salt$..."

[initrd]
network = "dhcp"

[system]
network   = "manual"
address   = "10.0.0.5/24"
gateway   = "10.0.0.1"
dns       = ["9.9.9.9"]
interface = "enp3s0"
backend   = "networkd"
```

```sh
moonshine-install --config /root/moonshine.toml
moonshine-install --config https://evillin.online/moonshine.toml
moonshine-install --config 'git+https://git.evillin.online/infra.git#main:machines/moonshine.toml'
```

- Git: `git+URL#[ветка|тег|коммит:]путь.toml`, путь относительный, без `..`.
- Аргументы после файла важнее конфига.
- Пример: [`dist/examples/moonshine.toml`](dist/examples/moonshine.toml).

---

## ✅ Проверка (check)

`check` проверяет конфиг целиком, **не меняя и не читая диски**. Код `0` — верно, `1` — ошибка.

```sh
moonshine-install check moonshine.toml
moonshine-install check moonshine.toml --plan
```

```text
Конфигурация корректна.

Платформа:      UEFI · x86_64 · Intel
Имя машины:     moonshine
Часовой пояс:   Europe/Moscow
Пользователи:   alice
Параметры ядра: нет
Службы:         нет

Разметка (диски и их размеры при проверке не читаются):
  /dev/sda1              512MiB  boot
  /dev/sda2                rest  /  [LUKS: ключ-файл, заголовок отдельно]

Источники пакетов:
  репозиторий хоста updates (приоритет 20)
  репозиторий из конфига company (приоритет 5)
  пакеты: встроенный набор + указанные пакеты, указано 2
Сеть в initramfs: нет
Сеть в системе:   нет

Замечания:
  ! root не указан: он будет заблокирован, входить нужно пользователем из wheel
  ! сеть в установленной системе не настроена
```

Замечания не меняют код возврата.

---

## 🔑 Секреты

| Что | Как передать |
|---|---|
| Пароль пользователя | хеш: `--user alice:'$6$…'` или `hash` |
| Парольная фраза дисков | файл (600): `--passphrase-file` / `passphrase_file`, иначе ввод |
| Ключ диска | создаётся сам (`crypt=key`) |
| Ключи SSH | `--ssh-key` |
| Доступ к конфигу по `https` | `~/.netrc` |
| Доступ к git | ключ SSH или менеджер учётных данных |

- Хеш: `openssl passwd -6`.
- Парольная фраза хешем быть не может: LUKS выводит ключ из самой фразы.
- Файл с правами для группы или остальных, пустой или многострочный — установка не начнётся.
- Пароль в URL не писать.

---

## 💿 Установка с носителя (dracut)

`dist/` превращает установщик в загрузочный образ: флешка, PXE/iPXE, пункт GRUB.

```sh
cargo build --release --locked
sudo make -C dist install
sudo /usr/libexec/moonshine-install/build-initrd moonshine-installer.img
```

`make install` понимает `DESTDIR` и `PREFIX`. Модуль подключается только по запросу (`dracut --add moonshine-install`).

| Параметр ядра | Значение |
|---|---|
| `moonshine.config=ГДЕ` | файл, `https://…`, `git+URL#[ветка:]путь.toml` |
| `moonshine.wizard=1` | мастер на консоли |
| `moonshine.yes=1` | без вопроса `YES` |
| `moonshine.check=1` | только проверка |
| `moonshine.after=reboot\|poweroff\|shell\|none` | действие после успеха (`reboot`) |
| `moonshine.lang=en\|ru` | язык |

- Конфиг по сети требует `ip=dhcp`; без `ip=` и `rd.neednet` модуль добавляет `rd.neednet=1 ip=dhcp`.
- Файл парольной фразы кладётся в образ: `build-initrd inst.img "$(uname -r)" --include /root/luks.txt /root/luks.txt`.
- При ошибке остаётся аварийная консоль dracut.

**GRUB:**

```sh
/usr/libexec/moonshine-install/grub-entry /vmlinuz /moonshine-installer.img \
    https://evillin.online/moonshine.toml moonshine.yes=1 >> /boot/grub2/custom.cfg
```

**iPXE:**

```text
#!ipxe
kernel http://evillin.online/vmlinuz moonshine.config=http://evillin.online/moonshine.toml moonshine.yes=1 ip=dhcp
initrd http://evillin.online/moonshine-installer.img
boot
```

**Живая система:**

```sh
sudo systemctl enable moonshine-install.service
```

> ⚠️ Конфиг по сети содержит хеши паролей: отдавайте по `https`. Для `crypt=pass` без присмотра используйте `passphrase_file`, `crypt=key` или не шифруйте.

Проверка скриптов: `make -C dist check`.

---

## 🧭 Управление в мастере

Страницы: **Шифрование → Диск → Репозитории → Система → Сеть в initramfs → Сеть в системе → Пакеты → Итог**.

| Клавиши | Действие |
|---|---|
| `Tab` / `PageDown` | следующая страница |
| `Shift+Tab` / `PageUp` | предыдущая страница |
| `↑` `↓` | страница, поле, пункт списка |
| `Enter` / `→` | войти, изменить, выбрать |
| `←` / `Esc` | назад, отмена |
| `Пробел` | переключить / открыть список |
| `e` / `F2` | править текстом |
| `Backspace` / `Delete` | убрать последнее значение |
| `Home` / `End` | первое / последнее поле |
| `Ctrl+C` | выход |

Экран установки показывает план: ✓ выполнено, ✗ сбой, ▸ идёт, · ожидает.

---

## 🌍 Язык

| Порядок | Способ |
|---|---|
| 1 | `--lang en\|ru` |
| 2 | `lang` в конфиге |
| 3 | `MOONSHINE_LANG` |
| 4 | `moonshine.lang=en\|ru` |
| 5 | `LC_ALL`, `LC_MESSAGES`, `LANG`: `ru…` → русский, иначе английский |

Ключи конфига, флаги и команды плана не переводятся. В initramfs по умолчанию английский.

---

## 📚 Флаги

```text
moonshine-install                              мастер
moonshine-install ФЛАГИ                        установка
moonshine-install check ЦЕЛЬ [--plan] [ФЛАГИ]  проверка

--disk ПУТЬ                   диск с корнем
--boot-disk ПУТЬ              диск загрузки (по умолчанию тот же)
--boot-size MIB               размер /boot (от 256), по умолчанию 512
--partition …  --raid …  --volume …   своя разметка
--firmware hybrid|uefi|bios|uboot
--arch x86_64|aarch64
--cpu intel|amd|other
--uboot-image ФАЙЛ            сырой образ U-Boot
--uboot-offset СЕКТОРЫ        смещение образа, меньше 32768

--repo ID                     репозиторий хоста (повторяется)
--repo-priority ID=N          приоритет 1…99, на время установки
--repository id=…,baseurl|metalink|mirrorlist=…[,name=…][,priority=…][,gpgcheck=0|1][,gpgkey=…]
--repo-dir КАТАЛОГ            локальный каталог с rpm (повторяется)
--repo-file ФАЙЛ              перенести .repo хоста в систему (повторяется)
--package ИМЯ                 пакет (повторяется)
--bare                        без встроенного набора пакетов
--option КЛЮЧ=ЗНАЧЕНИЕ        параметр dnf (повторяется)

--hostname ИМЯ  --timezone ЗОНА (по умолчанию UTC)
--kernel-arg ПАРАМЕТР         параметр ядра системы (повторяется)
--service ЮНИТ                служба systemd (повторяется)
--user ИМЯ[:ХЕШ]              пользователь (повторяется)

--no-encryption               без шифрования корня (с --disk)
--detached-header             заголовок LUKS корня отдельно (с --disk)
--keyfile                     ключ-файл корня (с --disk)
--ssh-key "КЛЮЧ"              ключ для разблокировки по SSH (повторяется)

--initrd-network none|dhcp|manual
--system-network none|dhcp|manual
  и --initrd-/--system- interface, address, gateway, dns
--system-backend networkd|networkmanager

--config ГДЕ                  файл, https:// или git+URL#[ветка:]путь
--passphrase-file ПУТЬ        файл с парольной фразой (600)
--lang en|ru                  язык
--yes                         без подтверждения
```

---

## 🛡️ Проверка входных данных

Правила одинаковы для флагов, мастера, TOML, URL и git; проверка идёт до первого действия над диском.

| Что | Правило |
|---|---|
| Диски | `/dev/…`, безопасные символы, без `..` |
| Пути | абсолютные, без `..`, пробелов и спецсимволов |
| Параметры dnf | `ключ=значение`, до 255 символов |
| Пакеты, id | короткий алфавит, без ведущего `-` |
| Пароли | без управляющих символов; хеши только допустимого формата |
| SSH-ключи | одна строка, известный тип, без опций |
| URL | `http(s)`; git: `https`, `http`, `ssh`, `git`; `ext::` и `file://` запрещены |

Данные пользователя не попадают в оболочку: команды запускаются с отдельными аргументами.

---

## 🏗️ Устройство

```text
Config ──► plan::build ──► Vec<Step> ──► exec::execute
 (CLI, TUI,   чистая         Run / Write /   по порядку,
  TOML)       функция        Release         секреты через stdin
```

Порядок: освобождение дисков → разметка → RAID → LVM → форматирование и LUKS → монтирование → `dnf` → настройка → `dracut` → загрузчик → размонтирование и закрытие слоёв.

| Модуль | Назначение |
|---|---|
| `main`, `cli`, `check` | вход, режимы, отчёт |
| `plan`, `storage`, `layout` | шаги, хранилище, модель разметки |
| `config`, `options`, `manifest`, `secrets` | конфигурация, флаги, TOML/URL/git |
| `repository`, `repos`, `sources`, `resolve` | репозитории, расчёт пакетов |
| `hardware`, `disks`, `network` | железо, диски, сеть |
| `draft`, `app`, `ui`, `tui`, `editor`, `input` | мастер и редактор |
| `exec`, `error`, `i18n` | выполнение, ошибки, язык |

---

## 🧪 Сборка и тесты

Rust edition 2024.

```sh
cargo build --release
cargo fmt && cargo clippy --all-targets && cargo test
make -C dist check
```

Тесты лежат в модулях `tests`, права root не нужны. На хосте нужны `dnf`, `sfdisk`, `wipefs`, `mount`, `dracut`; `git` и `curl` — для конфига из git и по URL.

---

## 🚧 Ограничения

- Архитектуры: `x86_64`, `aarch64` (в тестировании); файловые системы: ext4, xfs, btrfs без подтомов.
- Конфиг по URL и из git не проверяется подписью.
- Группа LVM, занимающая и другой диск, остаётся там частично активной.

Не проверено на реальных системах (проверялись планы и тесты):

- LUKS: несколько устройств в initramfs, отдельный заголовок, LUKS на RAID и LVM, закрытие слоёв при завершении
- RAID, LVM, тонкие тома, раздел `reserve`, модуль `usrmount`
- загрузчик: имена пакетов `*-modules`, hybrid, `grubenv` и `grub.cfg` на FAT
- `dnf` 4 и 5: `reposdir` со списком каталогов, `$releasever` хостовых репозиториев
- параметры ядра при BLS-записях Fedora, `systemctl enable` и `mask` в установленной системе
- btrfs, U-Boot, доставка утилит установщика
- модуль dracut и юнит systemd на реальном образе
- английские тексты не просматривались носителем языка

---

## 📜 Лицензия

Этот проект лицензирован под Apache License 2.0. Вы можете свободно использовать, распространять и модифицировать в соответствии с условиями лицензии.
Для получения подробной информации смотрите файл [`LICENSE`](LICENSE) или посетите [сайт Apache](https://www.apache.org/licenses/LICENSE-2.0).

---

## 👤 Контакт

Автор: NSerr0R  
Email: [nserr0r@gmail.com](mailto:nserr0r@gmail.com)
