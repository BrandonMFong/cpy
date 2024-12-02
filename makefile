# 
# author: Brando
# date: 6/15/23
#

BIN_NAME = cpy
BIN_PATH = ./bin
BUILD_PATH = ./build

CONFIG=release
ifeq ($(CONFIG), release)
BUILD_TYPE_FLAG=--release
else ifeq ($(CONFIG), debug)
BUILD_TYPE_FLAG=
endif

build: setup $(SCRIPT_DEST)
	cargo build $(BUILD_TYPE_FLAG) --target-dir $(BUILD_PATH)
	cp -afv $(BUILD_PATH)/$(CONFIG)/$(BIN_NAME) $(BIN_PATH)/$(CONFIG)

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

DIRS = $(addsuffix -setup, $(BUILD_PATH) $(BIN_PATH)/$(CONFIG))
setup: $(DIRS)
$(DIRS):
	mkdir -p $(subst -setup,,$@)

clean:
	rm -rfv $(BIN_PATH)
	rm -rfv $(BUILD_PATH)
	rm -rfv $(PACKAGE_NAME)
	cargo clean --verbose --color always

test:
	RUST_BACKTRACE=1 cargo test -- --test-threads=1

