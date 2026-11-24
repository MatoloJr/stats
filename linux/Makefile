UUID := $(shell python3 -c 'import json; print(json.load(open("metadata.json"))["uuid"])')
SCHEMA := schemas/org.gnome.shell.extensions.stats.gschema.xml
EXT_DIR := $(HOME)/.local/share/gnome-shell/extensions/$(UUID)

.PHONY: schemas install enable disable logs test spec pack lint
PACK_FILES := extension.js prefs.js metadata.json stylesheet.css lib schemas/org.gnome.shell.extensions.stats.gschema.xml

schemas:
	glib-compile-schemas --strict schemas/

install: schemas
	mkdir -p "$(HOME)/.local/share/gnome-shell/extensions"
	ln -sfn "$(CURDIR)" "$(EXT_DIR)"
	glib-compile-schemas --strict "$(EXT_DIR)/schemas"
	@echo "Enable with: make enable"
	@echo "On Wayland, log out and back in after that, or start a nested session (see README)."

enable:
	@if gnome-extensions info "$(UUID)" >/dev/null 2>&1; then \
		gnome-extensions enable "$(UUID)"; \
	else \
		printf '%s\n' \
			'import ast, subprocess, sys' \
			'uuid = sys.argv[1]' \
			'schema = "org.gnome.shell"' \
			'def get_list(key):' \
			'    raw = subprocess.check_output(["gsettings", "get", schema, key], text=True).strip()' \
			'    if raw in ("@as []", "[]"):' \
			'        return []' \
			'    return list(ast.literal_eval(raw))' \
			'enabled = get_list("enabled-extensions")' \
			'if uuid not in enabled:' \
			'    enabled.append(uuid)' \
			'    subprocess.check_call(["gsettings", "set", schema, "enabled-extensions", str(enabled)])' \
			'disabled = get_list("disabled-extensions")' \
			'if uuid in disabled:' \
			'    disabled = [item for item in disabled if item != uuid]' \
			'    subprocess.check_call(["gsettings", "set", schema, "disabled-extensions", str(disabled)])' \
			| python3 - "$(UUID)"; \
		echo "$(UUID) is not loaded in this Shell session yet."; \
		echo "It is now in enabled-extensions and will start after you log out and back in,"; \
		echo "or inside a nested session started after install (see README)."; \
	fi

disable:
	gnome-extensions disable $(UUID)

logs:
	journalctl -f -o cat /usr/bin/gnome-shell

test: spec
	gjs -m tests/parse.test.js

# Linux defaults against ../shared/defaults.ini (needs node, not gnome-shell).
spec:
	node ../shared/check-spec.mjs

# Release zip, built with python3 so CI needs no gnome-shell. The installer
# compiles the schema on the user's machine.
pack:
	rm -f "$(UUID).shell-extension.zip"
	python3 tools/pack.py "$(UUID).shell-extension.zip" $(PACK_FILES)
	@echo "Wrote $(UUID).shell-extension.zip"

lint:
	@test -x node_modules/.bin/eslint || npm install --ignore-scripts
	./node_modules/.bin/eslint .
