#!/bin/bash
# Модуль dracut: установщик Moonshine внутри initramfs.
# Подключается только по запросу: dracut --add moonshine-install

PROGRAMS=(
    moonshine-install
    sh cat sync mkdir cp rm chmod ln chroot truncate dd mount umount swapoff lsblk tac
    mountpoint rmdir dmesg
    sfdisk wipefs udevadm
    mkfs.fat mkfs.ext4 mkfs.xfs mkswap
    cryptsetup
    mdadm
    pvcreate vgcreate lvcreate vgchange dmsetup
    git curl
    reboot poweroff
)

# Нужны для dnf: он написан на Python и читает свои настройки и ключи.
OPTIONAL_PROGRAMS=(dnf dnf-3 dnf4 dnf5 rpm python3 sudo mkfs.btrfs thin_check)
OPTIONAL_TREES=(
    /usr/lib/python3 /usr/lib64/python3 /usr/lib/python3.* /usr/lib64/python3.*
    /usr/share/dnf5 /usr/lib64/libdnf5 /usr/lib/libdnf5
    /etc/dnf /etc/pki /etc/ssl /etc/crypto-policies /etc/rpm /usr/lib/rpm
    /usr/share/pki /usr/share/crypto-policies
)

check() {
    # 255: модуль включается только явно, через --add или add_dracutmodules
    return 255
}

depends() {
    echo network
    return 0
}

installkernel() {
    instmods =drivers/block =drivers/nvme =drivers/ata =drivers/scsi =drivers/usb/storage
    instmods ext4 xfs btrfs vfat dm_crypt dm_mod dm_thin_pool raid0 raid1 raid10 raid456 loop
    instmods =crypto
}

install_tree() {
    local root path

    for root in "$@"; do
        [ -e "$root" ] || continue
        find "$root" \( -type f -o -type l \) -print | while read -r path; do
            inst_simple "$path"
        done
    done
}

install() {
    inst_multiple "${PROGRAMS[@]}"
    inst_multiple -o "${OPTIONAL_PROGRAMS[@]}"
    install_tree "${OPTIONAL_TREES[@]}"

    inst_simple "$moddir/moonshine-run.sh" /usr/libexec/moonshine-install/moonshine-run
    inst_hook cmdline 90 "$moddir/moonshine-parse.sh"
    inst_hook initqueue/settled 90 "$moddir/moonshine-start.sh"
    inst_hook initqueue/online 90 "$moddir/moonshine-start.sh"
}
