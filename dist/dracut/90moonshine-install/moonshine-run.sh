#!/bin/sh
# Запуск moonshine-install по параметрам ядра. Один и тот же скрипт работает
# в initramfs (модуль dracut) и в живой системе (юнит systemd).
#
#   moonshine.config=ГДЕ   файл, https://… или git+URL#[ветка:]путь.toml
#   moonshine.wizard=1     без конфига: открыть мастер
#   moonshine.yes=1        не спрашивать YES перед стиранием дисков
#   moonshine.check=1      только проверить конфиг (moonshine-install check), ничего не стирая
#   moonshine.after=reboot|poweroff|shell|none   что делать после успеха (по умолчанию reboot)
#   moonshine.lang=ru|en   язык установщика (по умолчанию по LC_ALL, LC_MESSAGES, LANG)
#
# Для проверки: MOONSHINE_CMDLINE=файл вместо /proc/cmdline, MOONSHINE_BIN=программа.

set -u

CMDLINE=${MOONSHINE_CMDLINE:-/proc/cmdline}
BIN=${MOONSHINE_BIN:-moonshine-install}

# Значение последнего параметра с таким именем; код 1, если параметра нет.
arg() {
    value=
    found=1

    for word in $(cat "$CMDLINE"); do
        case $word in
        "$1"=*)
            value=${word#*=}
            found=0
            ;;
        "$1")
            value=1
            found=0
            ;;
        esac
    done

    [ "$found" -eq 0 ] && printf '%s\n' "$value"
    return "$found"
}

enabled() {
    case $(arg "$1") in
    1 | yes | true | on) return 0 ;;
    *) return 1 ;;
    esac
}

lang=$(arg moonshine.lang) || lang=

case $lang in
"" | en | ru) ;;
*) echo "moonshine-run: moonshine.lang: en or ru / en или ru, not/а не $lang" >&2; exit 2 ;;
esac

if [ -z "$lang" ]; then
    case ${LC_ALL:-${LC_MESSAGES:-${LANG:-}}} in
    ru*) message=ru ;;
    *) message=en ;;
    esac
else
    message=$lang
fi

# say РУССКИЙ ENGLISH: сообщение на выбранном языке.
say() {
    if [ "$message" = ru ]; then printf '%s\n' "$1"; else printf '%s\n' "$2"; fi
}

die() {
    echo "moonshine-run: $(say "$1" "$2")" >&2
    exit 2
}

config=$(arg moonshine.config) || config=

case $config in
*[!A-Za-z0-9._~:/?#@%'&'=+,-]*) die "в moonshine.config недопустимые символы" "moonshine.config contains invalid characters" ;;
-*) die "moonshine.config не может начинаться с «-»" "moonshine.config cannot start with \"-\"" ;;
esac

if enabled moonshine.check; then
    [ -n "$config" ] || die "moonshine.check требует moonshine.config" "moonshine.check needs moonshine.config"
    set -- check "$config"
elif [ -n "$config" ]; then
    set -- --config "$config"
elif enabled moonshine.wizard; then
    set --
else
    exit 0
fi

enabled moonshine.yes && ! enabled moonshine.check && set -- "$@" --yes
[ -n "$lang" ] && set -- "$@" --lang "$lang"

after=$(arg moonshine.after) || after=reboot

case $after in
reboot | poweroff | shell | none) ;;
*) die "moonshine.after: reboot, poweroff, shell или none, а не $after" "moonshine.after: reboot, poweroff, shell or none, not $after" ;;
esac

"$BIN" "$@"
status=$?

if [ "$status" -ne 0 ]; then
    echo "moonshine-run: $(say "установка завершилась с кодом $status" "the installation ended with code $status")" >&2

    if command -v emergency_shell > /dev/null 2>&1; then
        emergency_shell -n moonshine "$(say "установка не удалась" "the installation failed")"
    fi

    exit "$status"
fi

enabled moonshine.check && after=none

sync

case $after in
reboot) systemctl reboot 2> /dev/null || reboot -f ;;
poweroff) systemctl poweroff 2> /dev/null || poweroff -f ;;
shell) exec sh ;;
esac
