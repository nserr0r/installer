<div align="center">

# 🌙 moonshine-install

**Installer for RPM systems from repositories: wizard, CLI and config (file, URL, git).**

🌐 [Русский](README.md) · **English**

</div>

---

**🚀 Idea:** the system is built by `dnf --installroot` from your repositories straight onto the new disk. A machine is described by one small file.

> ⚠️ The installer **erases disks**. First run `moonshine-install check TARGET --plan`.

---

## ⚙️ Usage

1. Wizard:

```sh
sudo moonshine-install
```

2. Check a config without changing anything:

```sh
moonshine-install check https://evillin.online/moonshine.toml --plan
```

3. Install from a config (file, `https://`, `git+URL#branch:path.toml`):

```sh
sudo moonshine-install --config https://evillin.online/moonshine.toml
```

4. Install with flags:

```sh
sudo moonshine-install --disk /dev/sda --repo baseos --hostname moonshine \
  --user alice:'$6$salt$...' --timezone Europe/Moscow
```

Minimal `moonshine.toml`:

```toml
disk     = "/dev/sda"
hostname = "moonshine"
repo     = ["baseos"]

[[users]]
name = "alice"
hash = "$6$salt$..."             # openssl passwd -6
```

Before erasing, the installer asks for `YES`; `--yes` skips the question.

---

## ✨ Features

- 🖥️ **Wizard** (TUI), **CLI**, **TOML config**
- 📦 **Repositories**: host, local, from the config; yum-style priorities
- 💾 **Layout**: partitions, **mdadm RAID**, **LVM2** (including thin volumes), ext4 / xfs / btrfs, swap
- 🔐 **LUKS2** on any device: passphrase or key file, detached header, unlock over SSH
- 🔌 **Boot from a separate disk** (USB stick)
- 🌐 **Platforms**: BIOS+UEFI (hybrid), UEFI, BIOS, U-Boot
- 🧩 **Kernel options** and **systemd services**
- 🛠️ **Network**: separately for initramfs and the system
- 💿 **dracut module**: install from media or over the network
- 🌍 **Languages**: English, Русский

| Architecture | Status |
|---|---|
| x86_64 | supported |
| aarch64 | in testing |

---

## 🔎 Hardware

Detected at start; override with `--arch`, `--cpu`, `--firmware`.

| What | Source | Result |
|---|---|---|
| Architecture | binary | `x86_64` / `aarch64` |
| CPU | `/proc/cpuinfo` | Intel → `microcode_ctl` |
| Firmware | x86_64 → **hybrid**; aarch64 → UEFI if `/sys/firmware/efi` exists, else U-Boot | layout, bootloader |

| Platform | `grub2-install --target` |
|---|---|
| hybrid | `i386-pc`, `i386-efi`, `x86_64-efi` |
| uefi | `x86_64-efi` / `arm64-efi` |
| bios | `i386-pc` |
| uboot | `extlinux/extlinux.conf` |

Commands run in a `chroot` with `--no-nvram --force --removable`, followed by `grub2-mkconfig`.

---

## 📦 Repositories and packages

| Source | Flag / key | Stays in the system |
|---|---|---|
| Host repository | `--repo ID` | via `--repo-file` |
| Local directory | `--repo-dir PATH` | no |
| Config repository | `--repository …` / `[[repository]]` | yes |

```toml
[[repository]]
id       = "baseos"
baseurl  = "https://mirror.evillin.online/9/BaseOS/$basearch/os/"
priority = 10                   # 1…99, lower = more important
gpgkey   = "file:///etc/pki/rpm-gpg/RPM-GPG-KEY-example"

[[repository]]
id       = "company"
baseurl  = "https://rpm.evillin.online/company/$basearch/"
priority = 5
gpgcheck = false
```

| Key | Meaning |
|---|---|
| `id` | required, unique |
| `baseurl` / `metalink` / `mirrorlist` | exactly one |
| `name`, `priority`, `exclude`, `includepkgs` | optional |
| `gpgcheck` | enabled by default |
| `gpgkey` | required while `gpgcheck` is on |

- Host repository priority: `--repo-priority ID=N`, during installation only.
- dnf options: `--option key=value` (`releasever=44` → `--releasever=44`).
- Packages: `--package NAME` (name, `@group`, pattern); `--bare` disables the built-in set.
- The package list with dependencies is shown before installation.

### Another RPM distribution

Requires `dnf`, GRUB2 (`grub2-install`, `grub2-mkconfig`) and `dracut`. With `--bare` the package list is fully yours:

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

## 👤 Users

```sh
moonshine-install ... --user alice:'$6$salt$hash' --user bob
```

- If `root` is not listed, it is locked.
- Others get a home directory and the **wheel** group; `sudo` is installed.
- Without a hash the password is asked on the console.

---

## 🧩 Kernel and services

| What | Flag | TOML | Wizard |
|---|---|---|---|
| Kernel options | `--kernel-arg` | `kernel_arg = ["quiet"]` | "Kernel options" |
| Services | `--service` | `service = ["sshd"]` | "Services to enable" |

- Options are appended to `GRUB_CMDLINE_LINUX` (`/etc/default/grub`) and written to `/etc/kernel/cmdline`.
- Printable ASCII without spaces, quotes, `$`, `` ` `` and `\` is allowed.
- Services are enabled with `systemctl enable` in the `chroot`; add the service package to `--package`.

---

## 🛠️ Network

Configured separately for initramfs (`--initrd-*`, `[initrd]`) and the system (`--system-*`, `[system]`).

| Mode | Meaning |
|---|---|
| `none` | no network |
| `dhcp` | DHCP, `interface` optional |
| `manual` | `interface`, `address`, `gateway` required; `dns` repeats |

- The system service is set by `--system-backend networkd|networkmanager`.
- The other service is masked (`systemctl mask`) and the chosen one is enabled: `networkmanager` masks `systemd-networkd`, `networkd` masks `NetworkManager`. A missing unit is not an error.
- `--ssh-key` installs dracut-sshd and requires a network in initramfs.

---

## 💾 Disks and layout

A disk is given by path: `/dev/sda`, `/dev/nvme0n1` or `/dev/disk/by-id/…` (preferred).

| Scheme | Layout |
|---|---|
| `--disk` | biosboot (BIOS, hybrid) + `/boot` FAT32 + root |
| `--disk` + `--boot-disk` | boot on the second disk, system on the whole disk without a partition table |
| U-Boot | `/boot` as the first partition with a 16 MiB offset |

- `/boot` is FAT32 and doubles as the ESP. Default 512 MiB (`--boot-size`, minimum 256).
- The wizard also offers "LVM" and "RAID + LVM" over several disks.

### Custom layout

```text
--partition disk=DISK,size=SIZE,point=POINT[,fs=ext4|xfs|btrfs][,mkfs=…][,options=…][,crypt=pass|key[,header=detached]]
--raid      name=NAME,level=0|1|5|6|10,point=POINT[,fs=…][,mkfs=…][,options=…][,crypt=…]
--volume    vg=GROUP,name=NAME,size=SIZE,point=POINT[,pool=POOL][,fs=…][,mkfs=…][,options=…][,crypt=…]
```

| `size` | Meaning |
|---|---|
| `512MiB`, `40GiB` | exact size |
| `rest` | the remainder |
| `whole` | whole disk, no partition table |

| `point` | Meaning |
|---|---|
| `biosboot` | 4 MiB for GRUB (BIOS, hybrid) |
| `reserve` | empty partition the size of `biosboot` and `/boot` |
| `boot` | `/boot`, FAT32, always a plain partition |
| `swap` | swap |
| `pool` | thin pool (`--volume`) |
| `lvm:GROUP` | physical volume |
| `raid:NAME` | array member |
| `/`, `/home`, … | filesystem |

```text
--partition disk=/dev/sda,size=4MiB,point=biosboot
--partition disk=/dev/sda,size=512MiB,point=boot
--partition disk=/dev/sda,size=rest,point=lvm:vg
--volume vg=vg,name=root,size=40GiB,point=/
--volume vg=vg,name=home,size=rest,point=/home,fs=xfs
```

RAID1 under LVM, boot from a USB stick:

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

### LVM thin volumes

A pool is a volume with `point=pool`; thin volumes name it in `pool=`. A thin volume size is virtual (`rest` is not allowed).

```text
--partition disk=/dev/sda,size=4MiB,point=biosboot
--partition disk=/dev/sda,size=512MiB,point=boot
--partition disk=/dev/sda,size=rest,point=lvm:vg
--volume vg=vg,name=pool,size=100GiB,point=pool
--volume vg=vg,name=root,size=50GiB,point=/,pool=pool
--volume vg=vg,name=home,size=200GiB,point=/home,pool=pool
```

- A pool is not encrypted; `swap` cannot be thin.
- Exceeding the pool size gives a note in `check`.

### Filesystem options

| Key | Meaning |
|---|---|
| `mkfs=ARGS` | `mkfs.*` arguments separated by spaces |
| `options=OPTS` | `fstab` field (`defaults` if omitted) |

A comma inside values is written as `;`. `-U`, `-F`, `-f`, `--uuid`, `--force` are forbidden.

```text
--partition disk=/dev/sdb,size=whole,point=/data,mkfs=-T largefile4 -m 1,options=noatime
--partition disk=/dev/sdb,size=whole,point=/data,fs=xfs,mkfs=-m crc=1;finobt=1,options=noatime;discard
```

### Layout validation

Checked before anything is erased:

- one `/` and one `boot`; `biosboot` only for BIOS and hybrid
- RAID member count, distinct disks
- volumes in a group, at most one `rest`
- safe and unique mount points
- sizes against real disks (at install time only)

### Layout editor

Page "Disk" → "Disk scheme" → "Custom layout".

| Keys | Action |
|---|---|
| `↑` `↓` `Home` `End` `PgUp` `PgDn` | move |
| `→` / `←` | expand / collapse |
| `n` | add in context |
| `r` / `v` | RAID / LVM volume |
| `e` (`F2`) | edit |
| `f` | filesystem |
| `o` | options: default → SSD → large files |
| `c` / `h` | encryption / detached header |
| `d` (`Delete`) | delete |
| `Tab` | fill in `point=` |
| `u` | undo (up to 32 steps) |
| `Esc` (`q`) | close |

### Installer utilities

Missing utilities are installed by the host `dnf` before any disk is touched.

| Utility | Package | When |
|---|---|---|
| `mkfs.fat` | `dosfstools` | always |
| `mkfs.ext4` | `e2fsprogs` | ext4 |
| `mkfs.xfs` | `xfsprogs` | xfs |
| `mkfs.btrfs` | `btrfs-progs` | btrfs |
| `thin_check` | `device-mapper-persistent-data` | thin volumes |
| `cryptsetup` | `cryptsetup` | encryption |
| `pvcreate` | `lvm2` | LVM |
| `mdadm` | `mdadm` | RAID |

---

## 🔐 Encryption

LUKS2 on a partition, array, physical or logical volume; layers combine freely.

| Key | Meaning |
|---|---|
| `crypt=pass` | passphrase, one for all devices |
| `crypt=key` | random key `/etc/luks/NAME.key` (600) |
| `header=detached` | header `/etc/luks/NAME.hdr` |

- Simple schemes encrypt root: `--no-encryption`, `--keyfile`, `--detached-header`.
- Only devices required for `/` and `/usr` go into initramfs; the system opens the rest from `crypttab`.
- `/boot` and biosboot are not encrypted.
- For an initramfs device `crypt=key` requires `header=detached`.
- A header inside the device takes up to 16 MiB.
- A detached header on a partition is referenced in `crypttab` by `PARTUUID=`.

```text
--partition disk=/dev/sda,size=4MiB,point=biosboot
--partition disk=/dev/sda,size=512MiB,point=boot
--partition disk=/dev/sda,size=rest,point=/,crypt=key,header=detached
--partition disk=/dev/sdb,size=whole,point=/mnt/data,crypt=key,header=detached
```

### Remote unlock

`--ssh-key` enables dracut-sshd; the passphrase is entered over SSH.

> ⚠️ The root key and header in initramfs do not protect against losing the USB stick with `/boot`. Keep a copy of the headers elsewhere.

---

## 📄 Config

TOML mirrors the CLI flags:

1. `a_b` → `--a-b`.
2. An array repeats the flag.
3. `true` sets the flag, `false` does nothing.
4. `[initrd]`, `[system]`, `[uboot]` add a prefix.
5. `[[users]]`, `[[partition]]`, `[[raid]]`, `[[volume]]`, `[[repository]]` are blocks.

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

- Git: `git+URL#[branch|tag|commit:]path.toml`, relative path without `..`.
- Arguments after the file override the config.
- Example: [`dist/examples/moonshine.toml`](dist/examples/moonshine.toml).

---

## ✅ Check

`check` validates the whole config, **changing and reading no disks**. Exit code `0` — valid, `1` — error.

```sh
moonshine-install check moonshine.toml
moonshine-install check moonshine.toml --plan
```

```text
The configuration is valid.

Platform:       UEFI · x86_64 · Intel
Hostname:       moonshine
Time zone:      Europe/Moscow
Users:          alice
Kernel options: none
Services:       none

Layout (disks and their sizes are not read during a check):
  /dev/sda1              512MiB  boot
  /dev/sda2                rest  /  [LUKS: key file, detached header]

Package sources:
  host repository updates (priority 20)
  config repository company (priority 5)
  packages: built-in set + listed packages, listed 2
Network in initramfs: none
Network in system:    none

Notes:
  ! root is not listed: it will be locked, log in as a wheel user
  ! the network of the installed system is not configured
```

Notes do not change the exit code.

---

## 🔑 Secrets

| What | How to pass |
|---|---|
| User password | hash: `--user alice:'$6$…'` or `hash` |
| Disk passphrase | file (600): `--passphrase-file` / `passphrase_file`, otherwise prompt |
| Disk key | generated (`crypt=key`) |
| SSH keys | `--ssh-key` |
| Config access over `https` | `~/.netrc` |
| Git access | SSH key or git credential helper |

- Hash: `openssl passwd -6`.
- A passphrase cannot be a hash: LUKS derives the key from the phrase itself.
- A file readable by group or others, empty or multi-line stops the installation.
- Do not put a password in a URL.

---

## 💿 Install from media (dracut)

`dist/` turns the installer into a bootable image: USB stick, PXE/iPXE, GRUB entry.

```sh
cargo build --release --locked
sudo make -C dist install
sudo /usr/libexec/moonshine-install/build-initrd moonshine-installer.img
```

`make install` honours `DESTDIR` and `PREFIX`. The module is included on request only (`dracut --add moonshine-install`).

| Kernel parameter | Meaning |
|---|---|
| `moonshine.config=WHERE` | file, `https://…`, `git+URL#[branch:]path.toml` |
| `moonshine.wizard=1` | wizard on the console |
| `moonshine.yes=1` | skip the `YES` question |
| `moonshine.check=1` | check only |
| `moonshine.after=reboot\|poweroff\|shell\|none` | action on success (`reboot`) |
| `moonshine.lang=en\|ru` | language |

- A config over the network needs `ip=dhcp`; without `ip=` and `rd.neednet` the module adds `rd.neednet=1 ip=dhcp`.
- Put the passphrase file into the image: `build-initrd inst.img "$(uname -r)" --include /root/luks.txt /root/luks.txt`.
- On failure the dracut emergency console stays open.

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

**Live system:**

```sh
sudo systemctl enable moonshine-install.service
```

> ⚠️ A config served over the network contains password hashes: serve it over `https`. For unattended `crypt=pass` use `passphrase_file`, `crypt=key`, or no encryption.

Script check: `make -C dist check`.

---

## 🧭 Wizard controls

Pages: **Encryption → Disk → Repositories → System → Initramfs network → System network → Packages → Summary**.

| Keys | Action |
|---|---|
| `Tab` / `PageDown` | next page |
| `Shift+Tab` / `PageUp` | previous page |
| `↑` `↓` | page, field, list item |
| `Enter` / `→` | enter, edit, select |
| `←` / `Esc` | back, cancel |
| `Space` | toggle / open list |
| `e` / `F2` | edit as text |
| `Backspace` / `Delete` | remove the last value |
| `Home` / `End` | first / last field |
| `Ctrl+C` | quit |

The install screen shows the plan: ✓ done, ✗ failed, ▸ running, · pending.

---

## 🌍 Language

| Order | Method |
|---|---|
| 1 | `--lang en\|ru` |
| 2 | `lang` in the config |
| 3 | `MOONSHINE_LANG` |
| 4 | `moonshine.lang=en\|ru` |
| 5 | `LC_ALL`, `LC_MESSAGES`, `LANG`: `ru…` → Russian, otherwise English |

Config keys, flags and plan commands are not translated. In initramfs the default is English.

---

## 📚 Flags

```text
moonshine-install                                wizard
moonshine-install FLAGS                          install
moonshine-install check TARGET [--plan] [FLAGS]  check

--disk PATH                   root disk
--boot-disk PATH              boot disk (default: same)
--boot-size MIB               /boot size (min 256), default 512
--partition …  --raid …  --volume …   custom layout
--firmware hybrid|uefi|bios|uboot
--arch x86_64|aarch64
--cpu intel|amd|other
--uboot-image FILE            raw U-Boot image
--uboot-offset SECTORS        image offset, below 32768

--repo ID                     host repository (repeatable)
--repo-priority ID=N          priority 1…99, during installation
--repository id=…,baseurl|metalink|mirrorlist=…[,name=…][,priority=…][,gpgcheck=0|1][,gpgkey=…]
--repo-dir DIR                local directory with rpms (repeatable)
--repo-file FILE              copy a host .repo into the system (repeatable)
--package NAME                package (repeatable)
--bare                        no built-in package set
--option KEY=VALUE            dnf option (repeatable)

--hostname NAME  --timezone ZONE (default UTC)
--kernel-arg OPTION           kernel option of the system (repeatable)
--service UNIT                systemd service (repeatable)
--user NAME[:HASH]            user (repeatable)

--no-encryption               no root encryption (with --disk)
--detached-header             detached LUKS header for root (with --disk)
--keyfile                     root key file (with --disk)
--ssh-key "KEY"               key for unlock over SSH (repeatable)

--initrd-network none|dhcp|manual
--system-network none|dhcp|manual
  and --initrd-/--system- interface, address, gateway, dns
--system-backend networkd|networkmanager

--config WHERE                file, https:// or git+URL#[branch:]path
--passphrase-file PATH        passphrase file (600)
--lang en|ru                  language
--yes                         no confirmation
```

---

## 🛡️ Input validation

Rules are the same for flags, wizard, TOML, URL and git; validation runs before the first action on a disk.

| What | Rule |
|---|---|
| Disks | `/dev/…`, safe characters, no `..` |
| Paths | absolute, no `..`, spaces or shell metacharacters |
| dnf options | `key=value`, up to 255 characters |
| Packages, ids | short alphabet, no leading `-` |
| Passwords | no control characters; hashes of valid format only |
| SSH keys | one line, known type, no options |
| URL | `http(s)`; git: `https`, `http`, `ssh`, `git`; `ext::` and `file://` forbidden |

User data never reaches a shell: commands run with separate arguments.

---

## 🏗️ Design

```text
Config ──► plan::build ──► Vec<Step> ──► exec::execute
 (CLI, TUI,   pure           Run / Write /   in order,
  TOML)       function       Release         secrets via stdin
```

Order: release disks → partitioning → RAID → LVM → formatting and LUKS → mounting → `dnf` → configuration → `dracut` → bootloader → unmount and close layers.

| Module | Purpose |
|---|---|
| `main`, `cli`, `check` | entry, modes, report |
| `plan`, `storage`, `layout` | steps, storage, layout model |
| `config`, `options`, `manifest`, `secrets` | configuration, flags, TOML/URL/git |
| `repository`, `repos`, `sources`, `resolve` | repositories, package resolution |
| `hardware`, `disks`, `network` | hardware, disks, network |
| `draft`, `app`, `ui`, `tui`, `editor`, `input` | wizard and editor |
| `exec`, `error`, `i18n` | execution, errors, language |

---

## 🧪 Build and tests

Rust edition 2024.

```sh
cargo build --release
cargo fmt && cargo clippy --all-targets && cargo test
make -C dist check
```

Tests live in `tests` modules and need no root. The host needs `dnf`, `sfdisk`, `wipefs`, `mount`, `dracut`; `git` and `curl` for configs from git and URLs.

---

## 🚧 Limitations

- Architectures: `x86_64`, `aarch64` (in testing); filesystems: ext4, xfs, btrfs without subvolumes.
- A config from a URL or git is not signature-checked.
- An LVM group that also spans another disk stays partly active there.

Not verified on real systems (plans and tests only):

- LUKS: several devices in initramfs, detached header, LUKS on RAID and LVM, closing layers on finish
- RAID, LVM, thin volumes, the `reserve` partition, the `usrmount` module
- bootloader: `*-modules` package names, hybrid, `grubenv` and `grub.cfg` on FAT
- `dnf` 4 and 5: `reposdir` with a directory list, `$releasever` of host repositories
- kernel options with Fedora BLS entries, `systemctl enable` and `mask` in the installed system
- btrfs, U-Boot, installing the installer's utilities
- the dracut module and systemd unit on a real image
- English texts have not been reviewed by a native speaker

---

## 📜 License

This project is licensed under the Apache License 2.0. You may freely use, distribute and modify it under the terms of the license.
For details see the [`LICENSE`](LICENSE) file or visit the [Apache website](https://www.apache.org/licenses/LICENSE-2.0).

---

## 👤 Contact

Author: NSerr0R  
Email: [nserr0r@gmail.com](mailto:nserr0r@gmail.com)
