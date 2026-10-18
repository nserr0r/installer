#!/bin/sh
# Проверка скриптов: синтаксис, shellcheck (если есть) и тесты moonshine-run.

set -eu

cd "$(dirname "$0")/.."

for file in dracut/90moonshine-install/*.sh scripts/*.sh tests/*.sh; do
    case $file in
    */module-setup.sh) bash -n "$file" ;;
    *) sh -n "$file" ;;
    esac
done

if command -v shellcheck > /dev/null; then
    shellcheck -s sh dracut/90moonshine-install/moonshine-*.sh scripts/*.sh tests/*.sh
    shellcheck -s bash dracut/90moonshine-install/module-setup.sh
fi

sh tests/run.sh
echo "dist: всё в порядке"
