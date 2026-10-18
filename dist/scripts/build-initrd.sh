#!/bin/sh
# Собирает initramfs с установщиком.
#   build-initrd.sh [ВЫХОДНОЙ_ФАЙЛ [ВЕРСИЯ_ЯДРА [АРГУМЕНТЫ_DRACUT...]]]
# Пример с файлом парольной фразы внутри образа:
#   build-initrd.sh inst.img "$(uname -r)" --include /root/luks.txt /root/luks.txt
# Модуль 90moonshine-install должен быть установлен (make -C dist install).

set -eu

output=${1:-moonshine-installer.img}
kernel=${2:-$(uname -r)}

if [ "$#" -ge 2 ]; then shift 2; else shift "$#"; fi

command -v dracut > /dev/null || {
    echo "build-initrd: dracut не найден" >&2
    exit 1
}

command -v moonshine-install > /dev/null || {
    echo "build-initrd: moonshine-install не найден в PATH" >&2
    exit 1
}

dracut --force --no-hostonly --add moonshine-install "$@" "$output" "$kernel"

cat << TEXT
Готово: $output (ядро $kernel)
Параметры ядра для установки по конфигу:
  moonshine.config=https://evillin.online/moonshine.toml moonshine.yes=1 ip=dhcp
Для мастера на консоли:
  moonshine.wizard=1
TEXT
