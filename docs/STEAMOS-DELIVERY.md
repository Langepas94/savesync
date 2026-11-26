# SaveSync для SteamOS

Пакет содержит daemon, desktop app, systemd user unit и собранный Decky frontend.
Установка выполняется только в home, без изменения read-only rootfs и без sudo.

Распакуйте `savesync-steamos.tar.gz`, войдите в Desktop Mode и выполните:

```sh
cd savesync-steamos
./install-steamos.sh
```

Приложение появится в меню. Daemon запускается через `systemctl --user` и
не зависит от открытого окна или Decky. `./install-steamos.sh --no-start`
только устанавливает файлы и unit без запуска службы.

Обновление тем же скриптом проверяет версии, готовит staging, корректно
останавливает старый daemon/app и ждёт готовности нового IPC. Ошибка возвращает
прежние файлы и running state службы. Конфиг, история, custom manifests,
credential vault и пользовательские бэкапы сохраняются. Target symlinks
отклоняются, чтобы обновление не записывало данные за пределами home.

Облака подключаются через [OAuth](OAUTH.md). Gaming Mode использует Secret Service
или явный encrypted vault с паролем в окружении сессии. [LAN](LAN.md) требует
подтверждения SAS на обоих устройствах; сеть не получает доверие автоматически.

Готовый каталог плагина находится в `decky-plugin/`. Установите его штатным
способом вашей версии Decky Loader. Плагин содержит только proxy/QAM UI;
конфликты разрешаются в desktop app. Публикация в Decky Store не заявляется.

## Сборка и проверка пакета

Следующие команды выполняются из корня репозитория с исходниками.

```sh
cargo build --release --locked --workspace
npm ci --prefix decky-plugin
npm run build --prefix decky-plugin
./packaging/package-steamos.sh target/release dist
python3 scripts/check_steamos_package.py dist/savesync-steamos.tar.gz
```

CI собирает Linux package с Rust 1.89 в Ubuntu 22.04 container и проверяет
максимальную требуемую GLIBC version не выше 2.35. Это предотвращает случайную
привязку release к новой glibc runner image. Проверяются archive paths, executable
modes, версии обоих бинарей, Decky dist и установка в temporary home без сервиса.
Ubuntu latest отдельно запускает unit/CLI/QUIC и installer rollback tests.

Это не заменяет физический gate. На настоящем Deck выполните read-only
`./verify-steamos.sh`, затем проверьте живой OAuth/refresh, QAM в Gaming Mode,
pairing с другим хостом и круговую синхронизацию одноразового тестового сейва.
Не используйте единственную копию рабочего сейва для первичной проверки.

После загрузки пакета `sha256sum -c savesync-steamos.tar.gz.sha256` проверяет
целостность файла. Checksum не является отдельной подписью издателя.

Техническая база: [Ubuntu 22.04 runner libraries](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2204-Readme.md).
