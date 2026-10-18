# Установка installer, модуля dracut и юнита systemd.
#   make -C dist install DESTDIR=/tmp/root PREFIX=/usr
# Бинарник собирается отдельно: cargo build --release --locked

PREFIX ?= /usr
BINARY ?= ../target/release/moonshine-install
MODULE := 90moonshine-install

bindir := $(DESTDIR)$(PREFIX)/bin
libexecdir := $(DESTDIR)$(PREFIX)/libexec/moonshine-install
dracutdir := $(DESTDIR)$(PREFIX)/lib/dracut/modules.d/$(MODULE)
unitdir := $(DESTDIR)$(PREFIX)/lib/systemd/system
docdir := $(DESTDIR)$(PREFIX)/share/doc/moonshine-install

.PHONY: install uninstall check

install:
	install -D -m 0755 $(BINARY) $(bindir)/moonshine-install
	install -D -m 0755 dracut/$(MODULE)/moonshine-run.sh $(libexecdir)/moonshine-run
	install -d $(dracutdir)
	install -m 0755 dracut/$(MODULE)/module-setup.sh dracut/$(MODULE)/moonshine-parse.sh \
		dracut/$(MODULE)/moonshine-start.sh dracut/$(MODULE)/moonshine-run.sh $(dracutdir)
	install -D -m 0644 systemd/moonshine-install.service $(unitdir)/moonshine-install.service
	install -D -m 0755 scripts/build-initrd.sh $(libexecdir)/build-initrd
	install -D -m 0755 scripts/grub-entry.sh $(libexecdir)/grub-entry
	install -d $(docdir)
	install -m 0644 examples/* $(docdir)

uninstall:
	rm -f $(bindir)/moonshine-install $(unitdir)/moonshine-install.service
	rm -rf $(libexecdir) $(dracutdir) $(docdir)

check:
	sh scripts/check.sh
