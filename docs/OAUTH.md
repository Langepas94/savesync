# Настройка OAuth

Релиз может включать публичный Client ID Яндекса из переменной сборки
`SAVESYNC_YANDEX_PUBLIC_CLIENT_ID`. Для своей регистрации укажите Client ID в
«Облако» → «Яндекс Диск» → «Настройка OAuth и ручной токен». Секреты и токены
храните отдельно от Git.

## Локальная конфигурация

Создайте `oauth-apps.json` рядом с `config.json`: `%APPDATA%\savesync` на Windows,
`${XDG_DATA_HOME:-$HOME/.local/share}/savesync` на Linux/SteamOS.

```json
{
  "google": { "client_id": "YOUR_GOOGLE_DESKTOP_CLIENT_ID" },
  "yandex": { "client_id": "YOUR_YANDEX_CLIENT_ID", "redirect_port": 17653 }
}
```

Альтернатива: `SAVESYNC_GOOGLE_CLIENT_ID`, `SAVESYNC_YANDEX_CLIENT_ID`.
Переменные окружения имеют приоритет. Если приложение требует client secret,
задайте `SAVESYNC_GOOGLE_CLIENT_SECRET` / `SAVESYNC_YANDEX_CLIENT_SECRET`
в окружении процесса демона. Файл `.env` автоматически не загружается.
После изменения окружения перезапустите демон.

Google: создайте OAuth client типа **Desktop app**, включите Drive API,
настройте consent/test users. Scope: `https://www.googleapis.com/auth/drive.appdata`.
Используется loopback IP `127.0.0.1` со свободным портом, S256 PKCE и
`access_type=offline`, `prompt=consent` для refresh token. Если ваш Desktop
client требует выданный Google secret, передайте его через указанную переменную.

Яндекс: зарегистрируйте OAuth приложение с разрешением `cloud_api:disk.app_folder`
и redirect URI **`http://127.0.0.1:17653/oauth/callback`**. Если выбрали другой
фиксированный порт, измените и регистрацию, и `redirect_port`. Flow использует
PKCE. Новые подключения работают в `app:/SaveSync`; старые вручную введённые
токены сохраняют прежний root `disk:/SaveSync`. Эти пространства различаются:
перед сменой способа входа сохраните переносимый backup, если нужна старая история.

Официальная документация: [Google native apps](https://developers.google.com/identity/protocols/oauth2/native-app),
[Яндекс authorization code/PKCE](https://yandex.ru/dev/id/doc/ru/codes/code-url),
[Яндекс refresh](https://yandex.ru/dev/id/doc/ru/tokens/refresh-client).

## Вход

В desktop UI откройте «Облако» и нажмите кнопку входа нужного сервиса.
После входа через Яндекс SaveSync записывает, читает и удаляет временный
проверочный файл в папке приложения. Новое подключение сохраняется после
успешной проверки. Существующее подключение можно проверить кнопкой
«Проверить доступ». Отмена доступна в UI. На вход в браузере отводится пять минут.

Без GUI:

```sh
savesync-daemon remote login google
savesync-daemon remote login yandex
savesync-daemon remote logout google
```

CLI печатает URL: откройте его на **том же компьютере**, где работает CLI/демон.
Запущенный демон обслуживает сессию через IPC; без него CLI выполняет flow сам.
Настройка каналов через CLI применяется к запущенному демону без перезапуска.
`remote logout` отключает канал и удаляет локальную credential запись; отзыв
доступа у провайдера выполняется в настройках соответствующего аккаунта.

## Секреты и обновление токенов

Windows использует Credential Manager, Linux — Secret Service в пользовательской
D-Bus сессии (например, GNOME Keyring/KWallet с Secret Service). Нужны доступный
и разблокированный store. SteamOS Gaming Mode без него использует предварительно настроенный Secret Service
или явный encrypted fallback ниже. Plaintext fallback не включён. Ошибка хранилища
показывается явно, токен не записывается в `config.json`.

Config/IPC содержат `keyring:<opaque-id>`. При первом чтении старого config
access tokens переносятся в системное хранилище. Старые резервные копии config
могут по-прежнему содержать plaintext: удалите их самостоятельно, если больше не нужны.
Не переносите references на другую машину: войдите заново. Ручной токен через
старый `remote setup` поддержан, но без refresh; повторный вход нужен при истечении.

Refresh выполняется перед истечением и один раз после HTTP 401. Ротация
сохраняется до использования нового token; операции сериализованы между
процессами профиля. HTTP 403 не вызывает бесконечного refresh. Reconnect
заменяет старую credential запись после успешного сохранения config.

Проверено mock HTTP и fake vault тестами, без управления компьютером.
Живой вход требует ваших зарегистрированных приложений и пока не заявлен.


## Шифрованный fallback без Secret Service

Если Gaming Mode не предоставляет разблокированный Secret Service, перед первым
входом выберите encrypted file vault: задайте **`SAVESYNC_VAULT_PASSPHRASE`**
в окружении приложения и демона. Минимум 12 символов; используйте длинную случайную
фразу. Приложение не сохраняет пароль на диск. Не добавляйте его в config,
`.env`, Git или unit file. Пароль нужен после перезапуска сессии/машины.

Для systemd user service можно в Desktop Mode безопасно ввести пароль через
`read -rs SAVESYNC_VAULT_PASSPHRASE`, экспортировать переменную, выполнить
`systemctl --user import-environment SAVESYNC_VAULT_PASSPHRASE` и перезапустить
`savesync.service`. Запускайте desktop app из окружения с той же переменной.
Команда с самим паролем не нужна и не должна попадать в историю shell.

Vault хранится в `<profile>/credentials.encrypted.json`: Argon2id (19 MiB,
2 прохода, 1 lane), случайные salt и 192-bit nonce, XChaCha20-Poly1305 с AAD.
Записи меняются под file lock через temp/flush/rename; на Unix файл создаётся
с mode 0600. Неверный пароль/повреждение отклоняется до записи и не заменяет
рабочий vault. Расшифрованный буфер и ключ очищаются при освобождении.

Выбор file vault явный: при наличии переменной используется этот store;
без неё — системный keyring. Выберите режим до OAuth. Existing references
не мигрируют между двумя stores автоматически: при смене режима войдите заново.
Храните резервную копию зашифрованного файла и пароль отдельно. Потерянный пароль
не восстанавливается программой; доступ к облаку можно подключить новым OAuth.
Это шифрованный fallback, plaintext fallback не предусмотрен.
