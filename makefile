# 
# author: Brando
# date: 6/15/23
#

UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Linux)
BUILD_TYPE=linux
PACKAGE_MODE = package-linux
else ifeq ($(UNAME_S),Darwin)
BUILD_TYPE=macos
PACKAGE_MODE = package-macos
endif
 
BIN_NAME = cpy
SCRIPTS_PATH = ./scripts/*
BIN_PATH = ./bin
PACKAGE_NAME = cpy 
PLATFORM=$(BUILD_TYPE)

CONFIG=release
ifeq ($(CONFIG), release)
BIN_DIR = $(BIN_PATH)/release
GOTO_TOOL_BUILD_PATH = ./target/release/$(BIN_NAME)
BUILD_TYPE_FLAG=--release
else ifeq ($(CONFIG), debug)
BIN_DIR = $(BIN_PATH)/debug
GOTO_TOOL_BUILD_PATH = ./target/debug/$(BIN_NAME)

# default is debug
BUILD_TYPE_FLAG=
endif

SCRIPT_NAMES = 
SCRIPT_DEST = 

COMPONENT_NAMES = $(BIN_NAME)
PACKAGE_COMPONENTS = $(patsubst %, $(PACKAGE_NAME)/%, $(COMPONENT_NAMES))

.PHONY: package-setup

build: setup $(SCRIPT_DEST)
	cargo build $(BUILD_TYPE_FLAG) --target-dir $(BIN_PATH)

help:
	@echo "Usage:"
	@echo "	make [target] variables"
	@echo ""
	@echo "Target(s):"
	@echo "	clean			cleans build and bin folder"
	@echo "	build 			builds release verions"
	@echo "	package			compresses build"
	@echo ""
	@echo "Variable(s):"
	@echo "	CONFIG		use this to change the build config. Accepts \"release\" (default), \"debug\", or \"test\""
	@echo "	IDENTITY	(macos only) \"Developer ID Application\" common name"
	@echo "	TEAMID 		(macos only) Organizational Unit"
	@echo "	EMAIL 		(macos only) Developer account email"
	@echo "	PW		(macos only) Developer account password"
	@echo ""
	@echo "Example(s):"
	@echo "	Build for release for macOS distribution"
	@echo "		make clean build codesign package notarize staple IDENTITY=\"\" TEAMID=\"\" EMAIL=\"\" PW=\"\""
	@echo "	Build for release for Linux distribution"
	@echo "		make clean build package"


$(BIN_DIR)/%: scripts/%
	@cp -afv $< $(BIN_DIR)

setup: $(BIN_DIR)

$(BIN_PATH)/%:
	mkdir -p $@

clean:
	rm -rfv $(BIN_PATH)
	rm -rfv $(PACKAGE_NAME)
	cargo clean --verbose --color always

test:
	RUST_BACKTRACE=1 cargo test -- --test-threads=1

### Packaging

package: $(PACKAGE_NAME) $(PACKAGE_COMPONENTS) $(PACKAGE_MODE)

package-linux: 
	zip -r $(BIN_DIR)/$(PACKAGE_NAME)-$(PLATFORM).zip $(PACKAGE_NAME)
	tar vczf $(BIN_DIR)/$(PACKAGE_NAME)-$(PLATFORM).tar.gz $(PACKAGE_NAME)

package-macos:
	hdiutil create -fs HFS+ -volname Goto -srcfolder $(PACKAGE_NAME) $(BIN_DIR)/$(PACKAGE_NAME)-$(PLATFORM).dmg

$(PACKAGE_NAME):
	mkdir -p $@

$(PACKAGE_NAME)/%: $(BIN_DIR)/%
	@cp -afv $< $(PACKAGE_NAME)

codesign:
	codesign -s "$(IDENTITY)" --options=runtime --timestamp $(BIN_DIR)/$(BIN_NAME)

notarize:
	xcrun notarytool submit --apple-id "$(EMAIL)" --password "$(PW)" --team-id "$(TEAMID)" --wait $(BIN_DIR)/$(PACKAGE_NAME)-$(PLATFORM).dmg

staple:
	xcrun stapler staple $(BIN_DIR)/$(PACKAGE_NAME)-$(PLATFORM).dmg


