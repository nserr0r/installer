NAME    := moonshine-install

DESTDIR ?=
CARGO   ?= cargo
PREFIX  ?= /usr
BINDIR  ?= $(PREFIX)/bin

TARGET_DIR := /tmp/cargo-target-$(NAME)-$(shell id -u)
export CARGO_TARGET_DIR = $(TARGET_DIR)

define link
	@mkdir -p $(TARGET_DIR)
	@if [ -L target ]; then \
		unlink target; \
	elif [ -e target ]; then \
		rm -rf target; \
	fi
	@ln -sf $(TARGET_DIR) target
endef

.PHONY: all build setup fmt test debug clean clippy release install uninstall

all: setup fmt test clippy release

setup:
	$(call link)

build: release

fmt:
	$(CARGO) fmt --all

test: setup
	$(CARGO) test --workspace

debug: setup
	$(CARGO) build --workspace

clean:
	$(CARGO) clean

clippy: setup
	$(CARGO) clippy --all-targets

release: setup
	$(CARGO) build --release

install:
	install -dm0755 $(DESTDIR)$(BINDIR)
	install -m0755  $(TARGET_DIR)/release/$(NAME) $(DESTDIR)$(BINDIR)/$(NAME)

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/$(NAME)
