# Run tests:  make test
# Install:    make install  (symlinks into ~/.local/share/nvim/site/pack/cade/start/)

NVIM      ?= nvim
PLUGIN_DIR = $(HOME)/.local/share/nvim/site/pack/cade/start/cade-neovim

.PHONY: test install

test:
	@$(NVIM) --headless -u NONE -u test/runner.lua 2>&1; \
	  echo "exit $$?"

install:
	@mkdir -p $(PLUGIN_DIR)
	@ln -sfn $(CURDIR)/lua  $(PLUGIN_DIR)/lua
	@ln -sfn $(CURDIR)/plugin $(PLUGIN_DIR)/plugin
	@echo "Installed → $(PLUGIN_DIR)"
