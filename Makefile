TARGET = x86_64-pc-windows-gnu
RELEASE_DIR = target/$(TARGET)/release
BIN_NAME = iracing-teleport

# Default: cross-compile for Windows
all: build

# Install the required target (one-time setup)
setup:
	rustup target add $(TARGET)
	brew list mingw-w64 || brew install mingw-w64

lint:
	cargo fmt --all
	cargo clippy --target=$(TARGET) --all-targets --all-features --fix --allow-dirty

# Cross compile using cargo
build:
	cargo build --target=$(TARGET) --release

# Remove compiled artifacts
clean:
	cargo clean

# Show the output binary path
print:
	@echo "Binary: $(RELEASE_DIR)/$(BIN_NAME).exe"