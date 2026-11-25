# SaveSync 1.0.0 — кандидат

Код требований первой итерации готов. Production tag и подписанный release
не создаются до внешних gates ниже. Автотриггеры запуска/выхода игры относятся
к следующей итерации и не включены в этот кандидат.

## Реализовано

| Требование | Результат |
|---|---|
| Поиск/база | Steam/UFS discovery, дополнительные библиотеки, bundled manifests, проверяемый импорт, custom rules |
| История/сейвы | Hash verification, portable locations, конфликты/merge versions, переносимые бэкапы, retention |
| Restore | Staging, safety backups, rollback файлов и metadata, recovery прерванного процесса, retry публикации |
| Облака | Настраиваемый PKCE/loopback OAuth, refresh/rotation, OS credential store и encrypted fallback |
| Конкурентные записи | Immutable delta publications, все ветки сохраняются, no-op suppression, hash cache, pagination |
| LAN | QUIC/TLS, SPAKE2, pinned Ed25519 identities, двусторонний SAS, mDNS/ручной адрес, отзыв доверия |
| UI | Desktop/Windows tray и Decky QAM, фоновые jobs, отзывчивый status, pairing в QAM |
| Windows | Per-user installer/autostart, корректный stop/upgrade/uninstall, проверяемый signed updater, Secrets pipeline |
| SteamOS | Home-only staged install/update с rollback, systemd user service, совместимый Linux package |
| IPC | Проверка владельца обоих endpoints до передачи PIN/config; Linux допускает privileged Decky |

Детали и ограничения: [QA](QA-2026-11-25.md), [OAuth](OAUTH.md), [LAN](LAN.md),
[index publications](INDEX-PUBLICATIONS.md), [Windows](WINDOWS-DELIVERY.md),
[SteamOS](STEAMOS-DELIVERY.md).

## Автоматические gates

- Rust fmt/clippy/test на Windows и Linux; CLI/IPC/двухузловой QUIC smoke.
- TypeScript/Rollup и Python Decky proxy tests.
- Silent Windows install/upgrade/uninstall с проверкой HKCU и сохранения профиля.
- POSIX installer regression: порядок stop, failed start/rollback, spaces, symlinks.
- Отдельный release build с Rust 1.89/glibc 2.35 и проверка готового Linux archive.
- Negative signer test: неподписанный файл не принимается updater.

Актуальные результаты публикуются в [CI репозитория](https://github.com/Langepas94/savesync/actions).
Read-only QA закрыл обнаруженные P1 по данным, vault, IPC, pairing и responsiveness.
Физический GUI/Deck не подменяется headless tests.

## Внешние gates перед публикацией

| Gate | Что требуется |
|---|---|
| Signing | Реальный code-signing certificate владельца и GitHub Secrets из WINDOWS-DELIVERY.md. Пока сертификата нет |
| Live OAuth | Зарегистрированные Google Desktop/Яндекс приложения; проверить consent, refresh/rotation и отказ доступа на тестовых аккаунтах |
| Physical Deck | Запустить verify-steamos.sh, проверить Gaming Mode/QAM, service/vault после перезапуска и одноразовый save roundtrip между двумя хостами |

Неизвестные секреты, сертификат и результаты физической проверки не генерируются
проектом. Настраиваемые OAuth/Secrets реализованы без них. До прохождения gates
доступны development packages, которые явно помечены как неподписанные.

## Выпуск после gates

Release branch `release/1.0.0` содержит согласованные версии workspace/installer.
Минимум сборки — Rust 1.89: он соответствует зафиксированным зависимостям
(`zip 9.8.2` требует 1.89). Изменять major/minor Rust dependencies для выпуска не нужно.

После проверки кандидата merge release PR в `main` и обратно в `develop`, затем
tag `v1.0.0`. Workflow проверяет совпадение версии, подписывает Windows executable,
setup/uninstaller/self-copies, проверяет Authenticode и создаёт draft Release
с Windows/Linux пакетами и checksums. Без signing secrets Windows release job
завершается ошибкой; unsigned installer не выдаётся за подписанный.

Все устройства должны перейти на новый cloud publication protocol. Старые
клиенты не видят новые immutable records. Перед сменой Yandex legacy root на
app-folder OAuth сохраните переносимый backup согласно OAUTH.md.
