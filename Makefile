UUID := $(shell python3 -c 'import json; print(json.load(open("metadata.json"))["uuid"])')
SCHEMA := schemas/org.gnome.shell.extensions.stats.gschema.xml
EXT_DIR := $(HOME)/.local/share/gnome-shell/extensions/$(UUID)

.PHONY: schemas install enable disable logs test pack lint

schemas:
	glib-compile-schemas --strict schemas/

install: schemas
	mkdir -p "$(HOME)/.local/share/gnome-shell/extensions"
	ln -sfn "$(CURDIR)" "$(EXT_DIR)"
	glib-compile-schemas --strict "$(EXT_DIR)/schemas"
	@echo "Enable with: gnome-extensions enable $(UUID)"
	@echo "On Wayland, log out and back in, or start a nested session (see README)."

enable:
	gnome-extensions enable $(UUID)

disable:
	gnome-extensions disable $(UUID)

logs:
	journalctl -f -o cat /usr/bin/gnome-shell

test:
	gjs -m tests/parse.test.js

pack: schemas
	gnome-extensions pack --force --extra-source=lib --extra-source=tests --schema=$(SCHEMA)

lint:
	@test -x node_modules/.bin/eslint || npm install --ignore-scripts
	./node_modules/.bin/eslint .
