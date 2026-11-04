#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Worktrack Fast Binary Installer (No Cargo/Rust Compilation Required)
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/IVKLD/proof-of-work/main/install.sh | bash
#   or locally: ./install.sh
# ==============================================================================

REPO="IVKLD/proof-of-work"
INSTALL_DIR="${WORKTRACK_BIN_DIR:-$HOME/.local/bin}"
CONFIG_DIR="$HOME/.config"
CONFIG_FILE="$CONFIG_DIR/worktrack.env"

echo "=== Установка worktrack (быстрый бинарный установщик) ==="

# 1. Check OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

if [ "$OS" != "Linux" ]; then
    echo "Ошибка: worktrack поддерживает только Linux (Pure Wayland). Обнаружена ОС: $OS" >&2
    exit 1
fi

if [ "$ARCH" != "x86_64" ]; then
    echo "Ошибка: готовые бинарники доступны для архитектуры x86_64. Обнаружено: $ARCH" >&2
    exit 1
fi

mkdir -p "$INSTALL_DIR"

# 2. Check for local pre-compiled binary or download from GitHub Releases
if [ -f "./target/release/worktrack" ] && [ "${1:-}" = "--local" ]; then
    echo "Используется локально скомпилированный бинарник: ./target/release/worktrack"
    cp "./target/release/worktrack" "$INSTALL_DIR/worktrack"
else
    DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/worktrack-linux-x86_64"
    echo "Скачивание готового бинарника с GitHub Releases ($DOWNLOAD_URL)..."

    TMP_FILE="$(mktemp /tmp/worktrack.XXXXXX)"
    trap 'rm -f "$TMP_FILE"' EXIT

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$DOWNLOAD_URL" -o "$TMP_FILE"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$TMP_FILE" "$DOWNLOAD_URL"
    else
        echo "Ошибка: для скачивания требуется curl или wget." >&2
        exit 1
    fi

    # Verify that the downloaded file is a valid Linux ELF executable
    if ! head -c 4 "$TMP_FILE" | grep -q $'\x7fELF'; then
        echo "Ошибка: скачанный файл повреждён или не является бинарником Linux ELF." >&2
        exit 1
    fi

    mv "$TMP_FILE" "$INSTALL_DIR/worktrack"
    trap - EXIT
fi

chmod +x "$INSTALL_DIR/worktrack"
echo "Бинарник успешно установлен в $INSTALL_DIR/worktrack"

# 3. Create default configuration file if not exists
if [ ! -f "$CONFIG_FILE" ]; then
    mkdir -p "$CONFIG_DIR"
    cat << 'EOF' > "$CONFIG_FILE"
# Каталог для логов, скриншотов и архивов:
WORKTRACK_DIR="$HOME/worklog"

# Интервал снимков в секундах (по умолчанию 60):
WORKTRACK_INTERVAL=60

# Дневная норма рабочего времени в часах (по умолчанию 8):
WORKTRACK_DAILY_HOURS=8

# Выбор конкретного рабочего пространства (workspace) в niri:
# WORKTRACK_WORKSPACE="work"

# Выбор конкретного монитора (например DP-1, DP-2):
# WORKTRACK_OUTPUT="DP-1"

# Список запрещённых приложений (окна закрашиваются чёрным):
WORKTRACK_BLOCKED_APPS="telegram,keepassxc,discord"

# Формат скриншотов и оптимизация размера (~500 КБ):
WORKTRACK_FORMAT="jpg"
WORKTRACK_JPEG_QUALITY=60
WORKTRACK_MAX_WIDTH=1920
EOF
    echo "Создан файл настроек: $CONFIG_FILE"
fi

# 4. Generate shell auto-completions
WT="$INSTALL_DIR/worktrack"
if [ -x "$WT" ]; then
    # Bash
    BASH_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions"
    if [ -d "$BASH_DIR" ] || [ -f "$HOME/.bashrc" ]; then
        mkdir -p "$BASH_DIR"
        "$WT" completions bash > "$BASH_DIR/worktrack" 2>/dev/null || true
    fi

    # Zsh
    ZSH_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/zsh/site-functions"
    mkdir -p "$ZSH_DIR"
    "$WT" completions zsh > "$ZSH_DIR/_worktrack" 2>/dev/null || true

    # Fish
    FISH_DIR="$HOME/.config/fish/completions"
    mkdir -p "$FISH_DIR"
    "$WT" completions fish > "$FISH_DIR/worktrack.fish" 2>/dev/null || true
fi

# 5. Check PATH variable
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo ""
        echo "⚠️  Внимание: $INSTALL_DIR отсутствует в системной переменной PATH."
        echo "Добавьте в ваш ~/.bashrc или ~/.zshrc:"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

# 6. Check external dependency grim
if ! command -v grim >/dev/null 2>&1; then
    echo "Ошибка: требуется утилита 'grim'." >&2
    exit 1
fi

echo ""
echo "=== Установка завершена без компиляции! ==="
echo "Запуск трекера:   worktrack start"
echo "Текущий статус:   worktrack status"
echo "Обновление ПО:    worktrack update"
