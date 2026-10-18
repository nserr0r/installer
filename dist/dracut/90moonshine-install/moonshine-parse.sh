#!/bin/sh
# Хук cmdline: если задан moonshine.config=, просит у dracut сеть.

type getarg > /dev/null 2>&1 || . /lib/dracut-lib.sh

config=$(getarg moonshine.config=)

case $config in
http://* | https://* | git+*)
    mkdir -p /etc/cmdline.d
    echo "rd.neednet=1" > /etc/cmdline.d/40-moonshine.conf

    if [ -z "$(getarg ip=)" ]; then
        echo "ip=dhcp" >> /etc/cmdline.d/40-moonshine.conf
    fi
    ;;
esac
