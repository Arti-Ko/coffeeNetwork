#!/usr/bin/env bash
# =============================================================================
# install-sudoers.sh — включить запуск TUN-режима БЕЗ пароля.
#
# По умолчанию coffeeNetwork в режиме TUN просит пароль администратора при
# каждом подключении (sing-box поднимает сетевой интерфейс от root). Этот скрипт
# ставит NOPASSWD-правило sudo, после чего приложение стартует TUN тихо
# (оно сперва пробует `sudo -n`, и только при неудаче показывает запрос пароля).
#
#   sudo bash scripts/install-sudoers.sh          # включить
#   sudo bash scripts/install-sudoers.sh --remove # выключить
#
# ⚠️ БЕЗОПАСНОСТЬ: правило разрешает твоему пользователю запускать `/bin/sh`
# от root без пароля. На личной машине это осознанный компромисс ради удобства;
# на общем/рабочем компьютере ставить НЕ рекомендуется. Удалить — флагом --remove.
# =============================================================================
set -euo pipefail

SUDOERS=/etc/sudoers.d/coffeenetwork
USER_NAME="${SUDO_USER:-$(id -un)}"

if [[ "${1:-}" == "--remove" ]]; then
  rm -f "$SUDOERS"
  echo "✓ Правило удалено — TUN снова будет спрашивать пароль."
  exit 0
fi

if [[ $EUID -ne 0 ]]; then
  echo "Запусти через sudo:  sudo bash scripts/install-sudoers.sh"
  exit 1
fi

TMP="$(mktemp)"
printf '%s ALL=(root) NOPASSWD: /bin/sh -c *\n' "$USER_NAME" > "$TMP"

# Проверяем синтаксис перед установкой — кривой sudoers ломает sudo целиком.
if ! visudo -cf "$TMP" >/dev/null 2>&1; then
  echo "✗ Проверка синтаксиса sudoers не прошла — правило НЕ установлено."
  rm -f "$TMP"; exit 1
fi

install -m 0440 "$TMP" "$SUDOERS"
rm -f "$TMP"
echo "✓ Готово. Пользователь '$USER_NAME' теперь поднимает TUN без пароля."
echo "  Выключить:  sudo bash scripts/install-sudoers.sh --remove"
