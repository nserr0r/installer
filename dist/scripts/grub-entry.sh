#!/bin/sh
# Печатает пункт меню GRUB для загрузки установщика.
#   grub-entry.sh ЯДРО INITRD КОНФИГ [ДОПОЛНИТЕЛЬНЫЕ_ПАРАМЕТРЫ_ЯДРА]
# Пример: grub-entry.sh /vmlinuz /moonshine-installer.img https://evillin.online/moonshine.toml

set -eu

[ $# -ge 3 ] || {
    echo "использование: grub-entry.sh ЯДРО INITRD КОНФИГ [ПАРАМЕТРЫ…]" >&2
    exit 2
}

kernel=$1
initrd=$2
config=$3
shift 3

cat << TEXT
menuentry 'Установка Moonshine' {
    linux $kernel moonshine.config=$config ip=dhcp $*
    initrd $initrd
}
TEXT
