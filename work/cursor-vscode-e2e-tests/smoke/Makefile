# Tier 2 smoke targets. Run on a Mac with the relevant apps installed +
# cliclick + macOS Accessibility permission granted to your shell host.
#
# Not in CI. Run before tagging a release. Capture stdout/stderr in a PR
# artifact for the changelog.

SHELL := /usr/bin/env bash

.PHONY: all cursor vscode claude-desktop deps

all: deps cursor vscode claude-desktop
	@echo
	@echo "All smoke scripts passed. Now walk through manual-verify.md for the final ladder."

deps:
	@command -v cliclick >/dev/null 2>&1 || (echo "Missing dep: cliclick. Run: brew install cliclick" && exit 1)
	@command -v python3   >/dev/null 2>&1 || (echo "Missing dep: python3"                       && exit 1)
	@echo "deps OK: cliclick + python3"

cursor:
	@bash $(CURDIR)/cursor.sh

vscode:
	@bash $(CURDIR)/vscode.sh

claude-desktop:
	@bash $(CURDIR)/claude-desktop.sh
