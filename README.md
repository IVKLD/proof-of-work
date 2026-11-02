# proof-of-work

Pure Wayland (niri) трекер рабочего времени со скриншотами активных приложений и оверлейным индикатором. Без X11 / XWayland, без Python, без внешних утилит.

## Особенности
* **Захват окон вместо экрана**: Снимает только окна разрешённых рабочих программ на видимых воркспейсах через IPC `niri`. Обои, чужие столы и личные окна (пароли, личные чаты) не попадают в архив.
* **Штамп даты и времени**: В левом нижнем углу снимка наносится текущая метка времени (быстрый рендеринг прямо в память через встроенный TTF).
* **Wayland-индикатор**: Легковесная точка в углу экрана (`wlr-layer-shell`), клики проходят сквозь неё:
  * 🔴 **Красный** — идёт запись
  * 🟡 **Жёлтый** — пауза (отошёл пообедать)
* **Встроенная архивация**: Создание `.tar.gz` средствами Rust и автовыгрузка в конце дня.

---

## Запуск без установки (Nix Run)

```bash
# Посмотреть открытые окна и статус фильтра:
nix run github:IVKLD/proof-of-work -- windows

# Запустить трекер:
nix run github:IVKLD/proof-of-work -- start

# Пауза / Возобновление:
nix run github:IVKLD/proof-of-work -- pause
nix run github:IVKLD/proof-of-work -- resume

# Статус:
nix run github:IVKLD/proof-of-work -- status

# Остановка и упаковка архива:
nix run github:IVKLD/proof-of-work -- stop
```

---

## Установка в NixOS через Flake

### В `flake.nix` вашей системы (`/etc/nixos/flake.nix`):

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    
    # Подключение proof-of-work:
    proof-of-work = {
      url = "github:IVKLD/proof-of-work";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, proof-of-work, ... }: {
    nixosConfigurations.desktop = nixpkgs.lib.nixosSystem {
      modules = [
        # Вариант 1 (рекомендуемый): через готовый модуль
        proof-of-work.nixosModules.default
        {
          programs.worktrack.enable = true;
        }

        # ИЛИ Вариант 2: просто добавить пакет в systemPackages
        # {
        #   environment.systemPackages = [
        #     proof-of-work.packages.${pkgs.stdenv.hostPlatform.system}.default
        #   ];
        # }
      ];
    };
  };
}
```

После применения (`nixos-rebuild switch`) команда `worktrack` будет доступна глобально в системе.

---

## Настройка (`~/.config/worktrack.env`)

Создайте файл `~/.config/worktrack.env` при необходимости переопределить параметры:

```bash
# Директория для скриншотов и архивов (по умолчанию ~/worklog):
WORKTRACK_DIR="$HOME/worklog"

# Интервал снимков в секундах (по умолчанию 60):
WORKTRACK_INTERVAL=60

# Регулярка по app_id для фильтрации окон:
WORKTRACK_APPS="^(kitty|firefox|code|Code|discord|vesktop|org\\.telegram\\.desktop)$"

# Команда автоматической отправки архива в конце дня (stop/pack/upload):
# WORKTRACK_UPLOAD_CMD='curl -fsS -F "file=@{file}" https://example.com/upload'
```
