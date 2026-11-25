# Windows: установка, трей, обновления и подпись

Установщик использует текущего пользователя: `%LOCALAPPDATA%\Programs\SaveSync`,
HKCU Run и пользовательские ярлыки. SYSTEM-служба и права администратора не нужны.
Приложение запускает соседний daemon скрытым процессом и остаётся в трее.
Закрытие окна скрывает его; «Выход из приложения» завершает UI, демон продолжает
обслуживать IPC/бэкапы. Повторный запуск активирует существующее окно.

При обновлении/удалении установщик вызывает `savesync-daemon stop` и
`savesync-app --quit`. Stop сериализован с активной операцией демона: замена
файлов происходит после завершения синхронизации. Ошибка остановки отменяет
установку. Профиль `%APPDATA%\savesync` сохраняется при upgrade/uninstall.
Для полного удаления данных отдельно удалите профиль и локальные credentials.

## Сборка и проверки

Нужны Rust, Windows SDK и Inno Setup 6 из [официального источника](https://jrsoftware.org/isinfo.php).

```powershell
cargo build --release --workspace
./packaging/windows/build.ps1 -SourceDir target/release -Version 0.1.0
```

Без `-Signed` получается **`savesync-setup-dev.exe`** — неподписанная dev-сборка.
Она не считается релизом и не принимается автоматическим updater.
CI отдельно компилирует этот пакет и проверяет silent install, HKCU autostart,
upgrade с работающим демоном, uninstall и сохранение профиля. Smoke разрешён
только на изолированном CI runner; пользовательская GUI-сессия не используется.
Тесты логики трея выполняются без создания tray icon; живой desktop UX не заявлен
как проверенный только по этим тестам.

## GitHub Secrets для релиза

Сертификат не поставляется с исходниками; подпись настраивается через Secrets.
Добавьте в приватный репозиторий:

- `SAVESYNC_SIGNING_PFX_BASE64`: Base64 PFX с code-signing certificate и private key.
- `SAVESYNC_SIGNING_PFX_PASSWORD`: пароль PFX.

Сертификат должен поддерживать Authenticode, иметь private key, быть действующим
и доверенным Windows. Не добавляйте PFX или пароль в Git, артефакты или чат.
Для сертификата/сервиса с аппаратным ключом замените `sign.ps1` на интеграцию
провайдера, сохранив проверку подписи и signer pin.

На release tag `vX.Y.Z` версия тега обязана совпадать с workspace version.
Workflow импортирует ключ в ephemeral Windows runner, вычисляет SHA256 DER
сертификата и встраивает его как `SAVESYNC_SIGNER_SHA256` в updater.
Подписываются daemon, приложение, installer, uninstaller и setup self-copies.
Используются SHA256 и RFC3161 timestamp; `signtool verify /pa /all /tw` обязателен.
Отсутствие secrets или ошибка проверки останавливает release job.

Артефакты попадают в draft GitHub Release после всех проверок. Публикация draft
выполняется владельцем после живого OAuth/Deck gate. В этой работе release tag и
неподписанный «1.0.0» не публикуются. Пока ключа нет, реальная подпись не заявляется.

## Обновления приложения

«Настройки → Проверить Releases» выполняет сетевую работу в фоне. Для приватного
репозитория нужен `SAVESYNC_GITHUB_TOKEN` с read-only доступом к Releases.
Программа не пересылает API token на asset CDN. Скачивается стабильный release
новее текущего SemVer, без drafts/prereleases.

Перед запуском `savesync-setup.exe` проверяются:

- точный размер и SHA256 из release checksum file;
- Windows Authenticode со статусом Valid;
- точный SHA256 сертификата издателя, встроенный в текущую сборку;
- версия installer, совпадающая с выбранной версией release.

Installer запускается только после кнопки «Установить». Dev-сборки без signer
pin не выполняют автоматическую установку. Смена сертификата требует отдельного
плана перехода доверия; новый сертификат не принимается автоматически. SteamOS
использует userspace install script и перезапуск systemd unit.

Справка: [SignTool](https://learn.microsoft.com/windows/win32/seccrypto/signtool),
[Inno signing](https://jrsoftware.org/ishelp/topic_setup_signtool.htm).
