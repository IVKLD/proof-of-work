# proof-of-work

Быстрый трекер рабочего времени для Wayland (niri) с автоматическими скриншотами рабочих окон и экранным индикатором статуса.

## Особенности
* **Захват только рабочих окон**: Снимает только выбранные приложения через IPC `niri`. Личные программы, фоновые вкладки и рабочий стол не попадают в архив.
* **Штамп даты и времени**: На каждый снимок автоматически наносится таймштамп в левом нижнем углу.
* **Экранный индикатор (Wayland)**: Небольшая точка-оверлей в правом верхнем углу (клики проходят сквозь неё):
  * 🔴 **Красный** — запись активна
  * 🟡 **Жёлтый** — на паузе
* **Архивация**: Автоматическая сборка в `.tar.gz` и опциональная выгрузка на сервер по окончании работы.

---

## Запуск (Nix Run)

```bash
# Проверить список окон и статус фильтра:
nix run github:IVKLD/proof-of-work -- windows

# Управление:
nix run github:IVKLD/proof-of-work -- start
nix run github:IVKLD/proof-of-work -- pause
nix run github:IVKLD/proof-of-work -- resume
nix run github:IVKLD/proof-of-work -- status
nix run github:IVKLD/proof-of-work -- stop
```

---

## Установка в NixOS

Добавьте в `/etc/nixos/flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    proof-of-work = {
      url = "github:IVKLD/proof-of-work";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, proof-of-work, ... }: {
    nixosConfigurations.desktop = nixpkgs.lib.nixosSystem {
      modules = [
        proof-of-work.nixosModules.default
        {
          programs.worktrack.enable = true;
        }
      ];
    };
  };
}
```

---

## Конфигурация (`~/.config/worktrack.env`)

```bash
# Директория архивов и скриншотов (по умолчанию ~/worklog):
WORKTRACK_DIR="$HOME/worklog"

# Интервал снимков в секундах (по умолчанию 60):
WORKTRACK_INTERVAL=60

# Фильтр окон по app_id (regex):
WORKTRACK_APPS="^(kitty|firefox|code|Code|discord|vesktop|org\\.telegram\\.desktop)$"

# Команда выгрузки архива в конце дня:
# WORKTRACK_UPLOAD_CMD='curl -fsS -F "file=@{file}" https://example.com/upload'
```
