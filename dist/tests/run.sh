#!/bin/sh
# Тесты moonshine-run: поддельный установщик пишет свои аргументы в файл.

set -u

RUN="$(dirname "$0")/../dracut/90moonshine-install/moonshine-run.sh"
WORK=$(mktemp -d)
failed=0

trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/moonshine-install" << 'FAKE'
#!/bin/sh
printf '%s\n' "$*" > "$MOONSHINE_LOG"
exit "${MOONSHINE_STATUS:-0}"
FAKE
chmod +x "$WORK/moonshine-install"

# run ПАРАМЕТРЫ_ЯДРА: код возврата runner-а; аргументы установщика в $WORK/log
run() {
    echo "$1" > "$WORK/cmdline"
    rm -f "$WORK/log"
    MOONSHINE_CMDLINE="$WORK/cmdline" MOONSHINE_BIN="$WORK/moonshine-install" \
        MOONSHINE_LOG="$WORK/log" sh "$RUN" > /dev/null 2>&1
}

check() {
    name=$1
    expected=$2
    actual=$3

    if [ "$expected" = "$actual" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name: ждали «$expected», получили «$actual»"
        failed=1
    fi
}

logged() {
    if [ -f "$WORK/log" ]; then cat "$WORK/log"; else echo "-"; fi
}

run "ro quiet moonshine.config=https://h/m.toml moonshine.after=none"
check "конфиг по URL" "--config https://h/m.toml" "$(logged)"

run "moonshine.config=/etc/m.toml moonshine.yes=1 moonshine.after=none"
check "yes" "--config /etc/m.toml --yes" "$(logged)"

run "moonshine.config=git+https://h/r.git#main:m.toml moonshine.check=1 moonshine.yes=1"
check "проверка конфига из git" "check git+https://h/r.git#main:m.toml" "$(logged)"

run "moonshine.check=1"
check "проверка без конфига" "2" "$?"

run "moonshine.wizard moonshine.after=none"
check "мастер" "" "$(logged)"

run "ro quiet root=live:CDLABEL=X"
check "без параметров ничего не запускается" "-" "$(logged)"

run "moonshine.config=/a moonshine.lang=en moonshine.after=none"
check "язык передаётся установщику" "--config /a --lang en" "$(logged)"

run "moonshine.config=/a moonshine.lang=de"
check "неизвестный язык" "2" "$?"

run "moonshine.config=/a moonshine.config=/b moonshine.after=none"
check "побеждает последний параметр" "--config /b" "$(logged)"

run "moonshine.config=-x"
check "конфиг не может быть ключом" "2" "$?"

run "moonshine.config=/a;reboot"
check "недопустимые символы" "2" "$?"

run "moonshine.config=/a moonshine.after=explode"
check "неизвестное after" "2" "$?"

MOONSHINE_STATUS=3 run "moonshine.config=/a moonshine.after=none"
check "ошибка установщика передаётся наружу" "3" "$?"

exit "$failed"
