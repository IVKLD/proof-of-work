# proof-of-work

Быстрый трекер рабочего времени для Wayland (niri) с автоматическими скриншотами рабочего окна.

## Особенности
* **Чистый захват рабочего процесса**: Снимает текущее активное окно в фокусе (с отображением курсора мыши). В снимок **НЕ попадают обои рабочего стола**, фон и соседние окна. 1 интервал = ровно 1 файл.
* **Штамп даты и времени без фона**: На каждый снимок автоматически наносится аккуратный таймштамп в левом нижнем углу с теневой обводкой для читаемости без загромождения тёмным фоновым прямоугольником.
* **Архивация**: Автоматическая сборка в `.tar.gz` и опциональная выгрузка на сервер по окончании работы.

---

## Команды управления

```bash
worktrack start       # Запустить трекер в фоне
worktrack shot        # Сделать тестовый снимок прямо сейчас (без обоев)
worktrack pause       # Поставить на паузу
worktrack resume      # Снять с паузы (снимок сразу)
worktrack status      # Текущее состояние и количество снимков за сегодня
worktrack outputs     # Список подключённых мониторов (для WORKTRACK_OUTPUT)
worktrack pack        # Принудительно упаковать день в .tar.gz
worktrack upload      # Запустить команду выгрузки архива
worktrack stop        # Завершить трекер, упаковать день в .tar.gz и выгрузить
```

---

## Конфигурация ПО (`~/.config/worktrack.env`)

Параметры считываются из файла `~/.config/worktrack.env` (или системных переменных окружения):

```bash
# Каталог для логов, скриншотов и архивов (по умолчанию: ~/worklog):
WORKTRACK_DIR="$HOME/worklog"

# Интервал между автоматическими снимками в секундах (по умолчанию: 60):
WORKTRACK_INTERVAL=60

# Привязка к конкретному экрану (опционально, например "DP-1" или "DP-2").
# Если строка закомментирована — снимок делается с активного монитора с фокусом:
# WORKTRACK_OUTPUT="DP-1"

# Команда для отправки архива в конце рабочего дня.
# Токен {file} автоматически заменяется на полный путь к созданному .tar.gz:
# WORKTRACK_UPLOAD_CMD='curl -fsS -F "file=@{file}" https://example.com/api/upload'
# WORKTRACK_UPLOAD_CMD='rsync -avz "{file}" user@remote:/backups/worklog/'
```

### Параметры конфигурации

| Переменная | Значение по умолчанию | Описание |
| :--- | :--- | :--- |
| `WORKTRACK_DIR` | `~/worklog` | Каталог, в который складываются папки по датам (`YYYY-MM-DD`), лог событий `events.log` и итоговые архивы `.tar.gz`. Поддерживает `$HOME` и `~`. |
| `WORKTRACK_INTERVAL` | `60` | Частота создания скриншотов в секундах. |
| `WORKTRACK_OUTPUT` | *(не задано)* | Имя конкретного монитора для захвата (узнать имена можно через команду `worktrack outputs`). Если параметр не задан, скриншотится экран, на котором сейчас находится фокус ввода. |
| `WORKTRACK_UPLOAD_CMD`| *(не задано)* | Произвольная команда терминала, исполняемая при вызове `worktrack stop` или `worktrack upload`. |

---

## Интеграция с Niri (`~/.config/niri/config.kdl`)

### Горячие клавиши
Для быстрого управления трекером добавьте в секцию `binds` файла `~/.config/niri/config.kdl`:

```kdl
binds {
    // Пауза / Снятие с паузы
    Mod+Shift+P { spawn "sh" "-c" "if [ -f \"${XDG_RUNTIME_DIR:-/tmp}/worktrack.paused\" ]; then worktrack resume; else worktrack pause; fi"; }

    // Принудительный снимок (активное окно без обоев)
    Mod+Print { spawn "worktrack" "shot"; }

    // Завершение работы с архивацией
    Mod+Shift+Escape { spawn "worktrack" "stop"; }
}
```

---

## Установка в NixOS

Добавьте flake в конфигурацию системы `/etc/nixos/flake.nix`:

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

После применения конфигурации (`nixos-rebuild switch --flake /etc/nixos#desktop`) бинарник `worktrack` и shell-автодополнения для bash, zsh и fish будут установлены в систему.
