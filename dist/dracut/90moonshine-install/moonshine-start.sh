#!/bin/sh
# Хуки initqueue/settled и initqueue/online: запускают установку один раз.

type getarg > /dev/null 2>&1 || . /lib/dracut-lib.sh

LOCK=/run/moonshine-started

if ! getarg moonshine.config= > /dev/null && ! getargbool 0 moonshine.wizard; then
    exit 0
fi

[ -e "$LOCK" ] && exit 0

case $(getarg moonshine.config=) in
http://* | https://* | git+*)
    # Сеть поднимает dracut; без неё ждём следующего вызова хука.
    ls /tmp/net.*.up > /dev/null 2>&1 || exit 0
    ;;
esac

mkdir -p /run
: > "$LOCK"
exec /usr/libexec/moonshine-install/moonshine-run
